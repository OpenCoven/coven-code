//! Plugin marketplace — mirrors src/commands/plugin/ (17 files).
//!
//! Provides search, install, update, list, and uninstall for plugins
//! from the Claude registry.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A plugin entry from the marketplace registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub download_url: String,
    pub hash: String,
    pub tags: Vec<String>,
    pub updated_at: Option<u64>,
}

/// An installed plugin summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub name: String,
    pub version: String,
    pub install_path: std::path::PathBuf,
    pub description: String,
}

// ---------------------------------------------------------------------------
// Marketplace API client
// ---------------------------------------------------------------------------

const REGISTRY_URL: &str = "https://registry.claude.ai/plugins";

/// Search the marketplace for plugins matching `query`, optionally filtered by `tags`.
///
/// When `tags` is non-empty, `tags[]=tag` query parameters are appended to the URL.
pub async fn marketplace_search_filtered(
    query: &str,
    tags: &[&str],
) -> Result<Vec<MarketplaceEntry>, String> {
    let mut params: Vec<String> = Vec::new();

    if !query.is_empty() {
        params.push(format!("q={}", urlencoding::encode(query)));
    }
    for tag in tags {
        params.push(format!("tags[]={}", urlencoding::encode(tag)));
    }

    let url = if params.is_empty() {
        REGISTRY_URL.to_string()
    } else {
        format!("{}?{}", REGISTRY_URL, params.join("&"))
    };

    let resp = reqwest::get(&url)
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Registry returned {}", resp.status()));
    }

    resp.json::<Vec<MarketplaceEntry>>()
        .await
        .map_err(|e| format!("Parse error: {e}"))
}

/// Search the marketplace for plugins matching `query`.
///
/// Convenience wrapper around [`marketplace_search_filtered`] with no tag filter.
pub async fn marketplace_search(query: &str) -> Result<Vec<MarketplaceEntry>, String> {
    marketplace_search_filtered(query, &[]).await
}

/// Check all installed plugins for updates.
///
/// Returns `(name, current_version, latest_version)` for each plugin that has
/// a newer version available in the marketplace.
pub async fn marketplace_check_updates_all() -> Vec<(String, String, String)> {
    let installed = list_installed();
    let mut futures_vec = Vec::new();
    for plugin in &installed {
        let name = plugin.name.clone();
        let current = plugin.version.clone();
        futures_vec.push(async move {
            let results = marketplace_search(&name).await.unwrap_or_default();
            if let Some(latest) = results.iter().find(|e| e.name == name) {
                if latest.version != current {
                    return Some((name, current, latest.version.clone()));
                }
            }
            None
        });
    }
    futures::future::join_all(futures_vec)
        .await
        .into_iter()
        .flatten()
        .collect()
}

/// Install a plugin by name from the marketplace, or from a URL directly.
/// ZIP extraction failures are returned without treating the archive as YAML.
/// Downloads are parsed in memory, without a scratch file in the plugin directory.
/// Direct YAML writes preserve existing manifest permissions; new files use normal
/// creation permissions. Extraction and manifest writes are not atomic.
pub async fn marketplace_install(name_or_url: &str) -> Result<InstalledPlugin, String> {
    let entry = if name_or_url.starts_with("http") {
        // Direct URL install
        MarketplaceEntry {
            name: name_or_url
                .split('/')
                .next_back()
                .unwrap_or("plugin")
                .trim_end_matches(".zip")
                .to_string(),
            version: "0.0.0".to_string(),
            description: String::new(),
            author: String::new(),
            download_url: name_or_url.to_string(),
            hash: String::new(),
            tags: Vec::new(),
            updated_at: None,
        }
    } else {
        // Reject unsafe names before contacting the registry.
        plugin_install_dir(name_or_url)?;
        // Search by name
        let results = marketplace_search(name_or_url).await?;
        results
            .into_iter()
            .find(|e| e.name == name_or_url)
            .ok_or_else(|| format!("Plugin '{}' not found in marketplace", name_or_url))?
    };

    let install_dir = plugin_install_dir(&entry.name)?;

    // Download archive
    let resp = reqwest::get(&entry.download_url)
        .await
        .map_err(|e| format!("Download error: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Download returned {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("Read bytes: {e}"))?;

    // Verify hash if provided
    if !entry.hash.is_empty() {
        use sha2::{Digest, Sha256};
        let computed = hex::encode(Sha256::digest(&bytes));
        if computed != entry.hash {
            return Err(format!(
                "Hash mismatch: expected {}, got {}",
                entry.hash, computed
            ));
        }
    }

    std::fs::create_dir_all(&install_dir).map_err(|e| format!("Create dir: {e}"))?;

    // The download is already buffered; parsing it directly avoids collisions
    // between a scratch archive and installed files or archive members.
    match zip::ZipArchive::new(std::io::Cursor::new(&bytes)) {
        Ok(mut archive) => archive
            .extract(&install_dir)
            .map_err(|e| format!("Extract archive: {e}"))?,
        Err(_) => {
            // Preserve direct-manifest fallback when ZIP parsing fails.
            std::fs::write(install_dir.join("manifest.yaml"), &bytes)
                .map_err(|e| format!("Write manifest: {e}"))?;
        }
    }

    Ok(InstalledPlugin {
        name: entry.name.clone(),
        version: entry.version.clone(),
        install_path: install_dir,
        description: entry.description.clone(),
    })
}

/// Check for an update to `name` and download if newer.
pub async fn marketplace_update(name: &str) -> Result<Option<String>, String> {
    let installed = list_installed();
    let current = installed
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| format!("Plugin '{}' is not installed", name))?;

    let results = marketplace_search(name).await?;
    let latest = results
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| format!("Plugin '{}' not found in marketplace", name))?;

    if latest.version == current.version {
        return Ok(None); // Already up to date
    }

    marketplace_install(name).await?;
    Ok(Some(latest.version.clone()))
}

/// List all installed plugins.
pub fn list_installed() -> Vec<InstalledPlugin> {
    let plugins_dir = claurst_core::config::config_home().join("plugins");

    let Ok(entries) = std::fs::read_dir(&plugins_dir) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if !path.is_dir() {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().to_string();

            let yaml_path = path.join("manifest.yaml");
            let json_path = path.join("manifest.json");

            let (version, description) = if yaml_path.exists() {
                let content = std::fs::read_to_string(&yaml_path).unwrap_or_default();
                let version =
                    extract_yaml_str(&content, "version").unwrap_or_else(|| "0.0.0".to_string());
                let description = extract_yaml_str(&content, "description").unwrap_or_default();
                (version, description)
            } else if json_path.exists() {
                let content = std::fs::read_to_string(&json_path).unwrap_or_default();
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    (
                        v["version"].as_str().unwrap_or("0.0.0").to_string(),
                        v["description"].as_str().unwrap_or("").to_string(),
                    )
                } else {
                    ("0.0.0".to_string(), String::new())
                }
            } else {
                ("0.0.0".to_string(), String::new())
            };

            Some(InstalledPlugin {
                name,
                version,
                install_path: path,
                description,
            })
        })
        .collect()
}

/// Uninstall a plugin by removing its directory.
pub fn marketplace_uninstall(name: &str) -> Result<(), String> {
    let dir = plugin_install_dir(name)?;
    if !dir.exists() {
        return Err(format!("Plugin '{}' is not installed", name));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("Remove dir: {e}"))
}

fn plugin_install_dir(name: &str) -> Result<std::path::PathBuf, String> {
    if name.trim_matches(['.', ' ']).is_empty() || name.contains(['/', '\\', ':', '\0']) {
        return Err(
            "Plugin name must be a single directory name, not a path or drive-qualified name"
                .to_string(),
        );
    }
    Ok(claurst_core::config::config_home()
        .join("plugins")
        .join(name))
}

fn extract_yaml_str(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix(&format!("{key}:")) {
            return Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{marketplace_uninstall, plugin_install_dir};

    #[test]
    fn marketplace_uninstall_rejects_absolute_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let outside = temporary.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("marker"), "preserve").unwrap();
        // The failing baseline can delete only this owned temporary directory.
        let result = marketplace_uninstall(outside.to_str().unwrap());
        assert!(
            outside.join("marker").exists(),
            "uninstall escaped the plugins directory"
        );
        assert_eq!(
            std::fs::read_to_string(outside.join("marker")).unwrap(),
            "preserve"
        );
        assert!(result.is_err());
    }

    #[test]
    fn marketplace_paths_reject_non_names() {
        for name in [
            "",
            ".",
            "..",
            "...",
            " ",
            ".. ",
            " . . ",
            "../outside",
            "/absolute",
            "nested/plugin",
            r"nested\plugin",
            r"C:\outside",
            "C:relative",
            "plugin:stream",
            "bad\0name",
        ] {
            assert!(plugin_install_dir(name).is_err(), "accepted {name:?}");
        }
    }

    #[test]
    fn marketplace_paths_preserve_ordinary_names() {
        for name in [
            "my-plugin",
            "plugin_v2",
            "plugin.v2",
            "éclair",
            ".hidden-plugin",
            "some plugin",
        ] {
            let directory = plugin_install_dir(name).unwrap();
            assert_eq!(directory.file_name().unwrap(), name);
            assert_eq!(directory.parent().unwrap().file_name().unwrap(), "plugins");
        }
    }
}
