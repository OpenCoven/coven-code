//! User-owned plugin preferences. Project settings are never an authorization source.

use crate::{LoadedPlugin, PluginError, PluginRegistry, PluginSource};
use anyhow::Context;
use claurst_core::config::Settings;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct ProjectPluginGrant<'a> {
    source: &'a PluginSource,
    project_root: PathBuf,
    plugin_root: PathBuf,
}

fn preference_key(project_dir: &Path, plugin: &LoadedPlugin) -> anyhow::Result<String> {
    if plugin.source != PluginSource::Project {
        return Ok(plugin.name.clone());
    }

    let grant = ProjectPluginGrant {
        source: &plugin.source,
        project_root: project_dir
            .canonicalize()
            .with_context(|| format!("Cannot identify project folder {}", project_dir.display()))?,
        plugin_root: plugin
            .path
            .canonicalize()
            .with_context(|| format!("Cannot identify plugin folder {}", plugin.path.display()))?,
    };
    // JSON escapes path delimiters without lossy conversions. The space in the
    // prefix cannot occur in a valid plugin name, keeping legacy names distinct.
    Ok(format!(
        "project plugin v1:{}",
        serde_json::to_string(&grant)?
    ))
}

fn load_user_settings() -> anyhow::Result<Settings> {
    let path = Settings::global_settings_path();
    // Settings::load_sync defaults on invalid JSON. Trust decisions must instead
    // report that error, and must never merge repository-owned settings.
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot read {}", path.display()));
        }
    };
    serde_json::from_slice(&bytes).with_context(|| format!("Cannot parse {}", path.display()))
}

/// Persist an explicit user choice in engine-global settings.
///
/// Project consent trusts this canonical plugin folder (including future
/// changes) only within this canonical project. Callers must not enable or
/// report a saved preference unless this operation succeeds.
pub fn save_plugin_enabled(
    project_dir: &Path,
    plugin: &LoadedPlugin,
    enabled: bool,
) -> anyhow::Result<()> {
    let key = preference_key(project_dir, plugin)?;
    let mut settings = load_user_settings()?;
    if enabled {
        settings.disabled_plugins.remove(&key);
        settings.enabled_plugins.insert(key);
    } else {
        settings.enabled_plugins.remove(&key);
        settings.disabled_plugins.insert(key);
    }
    settings
        .save_sync()
        .with_context(|| format!("Cannot save {}", Settings::global_settings_path().display()))
}

pub(crate) fn apply_saved_state(registry: &mut PluginRegistry, project_dir: &Path) {
    let settings = match load_user_settings() {
        Ok(settings) => settings,
        Err(error) => {
            // Unknown saved state must not activate plugins or lose a disable.
            let names: Vec<_> = registry.all().iter().map(|p| p.name.clone()).collect();
            for name in names {
                registry.disable(&name);
            }
            registry.errors.push(PluginError::Io {
                path: Settings::global_settings_path()
                    .to_string_lossy()
                    .into_owned(),
                message: format!("{error:#}"),
            });
            return;
        }
    };

    let preferences: Vec<_> = registry
        .all()
        .into_iter()
        .map(|plugin| {
            let enabled = preference_key(project_dir, plugin).map(|key| {
                !settings.disabled_plugins.contains(&key)
                    && (plugin.source != PluginSource::Project
                        || settings.enabled_plugins.contains(&key))
            });
            (plugin.name.clone(), enabled)
        })
        .collect();
    for (name, enabled) in preferences {
        match enabled {
            Ok(true) => {
                registry.enable(&name);
            }
            Ok(false) => {
                registry.disable(&name);
            }
            Err(error) => {
                registry.disable(&name);
                registry.errors.push(PluginError::Validation {
                    name,
                    message: format!("Cannot authorize plugin: {error:#}"),
                });
            }
        }
    }
}
