//! Runtime-only plugin MCP provenance. Settings-owned servers stay in Config::mcp_servers.

use crate::config::{Config, McpServerConfig};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Typed discovery source; display labels never establish ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginMcpSource {
    User,
    Project,
    Extra,
    Inline,
    Builtin,
}

/// Canonical plugin identity, created only by runtime discovery, never deserialized.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PluginMcpOrigin {
    source: PluginMcpSource,
    plugin_root: PathBuf,
    project_root: Option<PathBuf>,
}

impl PluginMcpOrigin {
    pub fn new(
        source: PluginMcpSource,
        plugin_root: &Path,
        project_root: Option<&Path>,
    ) -> std::io::Result<Self> {
        let project_root = if source == PluginMcpSource::Project {
            Some(
                project_root
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "project MCP origin requires a project root",
                        )
                    })?
                    .canonicalize()?,
            )
        } else {
            None
        };
        Ok(Self {
            source,
            plugin_root: plugin_root.canonicalize()?,
            project_root,
        })
    }

    pub fn source(&self) -> PluginMcpSource {
        self.source
    }
    pub fn plugin_root(&self) -> &Path {
        &self.plugin_root
    }
    pub fn project_root(&self) -> Option<&Path> {
        self.project_root.as_deref()
    }

    /// Configuration eligibility only; live request and transport revocation is
    /// a separate responsibility of the MCP runtime.
    pub fn matches_project(&self, project_dir: Option<&Path>) -> bool {
        match &self.project_root {
            None => true,
            Some(root) => {
                project_dir
                    .and_then(|path| path.canonicalize().ok())
                    .as_ref()
                    == Some(root)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct PluginMcpContribution {
    pub origin: PluginMcpOrigin,
    pub config: McpServerConfig,
}

impl Config {
    /// Runtime view with explicit settings entries taking precedence at every
    /// read, including user servers added or edited after plugin discovery.
    pub fn effective_mcp_servers(&self) -> impl Iterator<Item = &McpServerConfig> {
        let mut names: HashSet<&str> = self
            .mcp_servers
            .iter()
            .map(|server| server.name.as_str())
            .collect();
        let plugins_allowed = !(self.hosted_review_enabled() && !self.hosted_review.allow_plugins);
        self.mcp_servers
            .iter()
            .chain(self.plugin_mcp_servers.iter().filter_map(move |entry| {
                if plugins_allowed
                    && entry.origin.matches_project(self.project_dir.as_deref())
                    && names.insert(&entry.config.name)
                {
                    Some(&entry.config)
                } else {
                    None
                }
            }))
    }

    /// Retire foreign project contributions without rediscovering or activating
    /// plugins. User-global contributions remain reconnect candidates.
    pub fn set_mcp_project(&mut self, project_dir: &Path) {
        self.project_dir = Some(project_dir.to_path_buf());
        self.plugin_mcp_servers
            .retain(|entry| entry.origin.matches_project(Some(project_dir)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;

    fn server(name: &str, command: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.into(),
            command: Some(command.into()),
            args: vec![],
            env: Default::default(),
            url: None,
            server_type: "stdio".into(),
        }
    }

    #[test]
    fn runtime_contributions_survive_clones_but_never_serde_settings_boundaries() {
        let _env = crate::coven_shared::COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let root = tempfile::tempdir().unwrap();
        let config = Config {
            mcp_servers: vec![server("user", "user-command")],
            plugin_mcp_servers: vec![PluginMcpContribution {
                origin: PluginMcpOrigin::new(PluginMcpSource::User, root.path(), None).unwrap(),
                config: server("plugin", "plugin-command"),
            }],
            ..Default::default()
        };
        assert_eq!(config.clone().effective_mcp_servers().count(), 2);
        let settings = Settings {
            config,
            ..Default::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(!json.contains("plugin_mcp_servers"));
        assert!(!json.contains("plugin-command"));
        let decoded: Settings = serde_json::from_str(&json).unwrap();
        assert!(decoded.config.plugin_mcp_servers.is_empty());
        assert_eq!(decoded.config.effective_mcp_servers().count(), 1);
        assert_eq!(
            decoded.config.mcp_servers[0].command.as_deref(),
            Some("user-command")
        );
    }

    #[test]
    fn external_settings_cannot_inject_runtime_plugin_ownership() {
        let _env = crate::coven_shared::COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let injected = serde_json::json!({"config": {
            "mcp_servers": [{"name":"user", "command":"user-command", "type":"stdio"}],
            "plugin_mcp_servers": [{
                "origin": {"source":"User", "plugin_root":"/tmp/fake", "project_root":null},
                "config": {"name":"injected", "command":"injected-command", "type":"stdio"}
            }]
        }});
        let settings: Settings = serde_json::from_value(injected).unwrap();
        assert!(settings.config.plugin_mcp_servers.is_empty());
        assert_eq!(settings.config.effective_mcp_servers().count(), 1);
        // Even malformed ownership input is skipped rather than parsed as trusted state.
        let config: Config =
            serde_json::from_value(serde_json::json!({"plugin_mcp_servers":"forged"})).unwrap();
        assert!(config.plugin_mcp_servers.is_empty());
    }

    #[test]
    fn explicit_user_entries_win_before_and_after_plugin_discovery() {
        let _env = crate::coven_shared::COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let root = tempfile::tempdir().unwrap();
        let mut config = Config {
            mcp_servers: vec![server("shared", "before")],
            plugin_mcp_servers: vec![PluginMcpContribution {
                origin: PluginMcpOrigin::new(PluginMcpSource::User, root.path(), None).unwrap(),
                config: server("shared", "plugin"),
            }],
            ..Default::default()
        };
        assert_eq!(config.effective_mcp_servers().count(), 1);
        assert_eq!(
            config
                .effective_mcp_servers()
                .next()
                .unwrap()
                .command
                .as_deref(),
            Some("before")
        );
        config.mcp_servers.clear();
        assert_eq!(
            config
                .effective_mcp_servers()
                .next()
                .unwrap()
                .command
                .as_deref(),
            Some("plugin")
        );
        config.mcp_servers.push(server("shared", "after"));
        config.mcp_servers[0].command = Some("edited".into());
        config.plugin_mcp_servers.clear();
        assert_eq!(config.effective_mcp_servers().count(), 1);
        assert_eq!(config.mcp_servers[0].command.as_deref(), Some("edited"));
    }

    #[test]
    fn project_transition_withdraws_foreign_origins_and_keeps_user_origins() {
        let _env = crate::coven_shared::COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let origin =
            PluginMcpOrigin::new(PluginMcpSource::Project, a.path(), Some(a.path())).unwrap();
        assert_eq!(origin.source(), PluginMcpSource::Project);
        assert_eq!(origin.plugin_root(), a.path().canonicalize().unwrap());
        assert_eq!(
            origin.project_root(),
            Some(a.path().canonicalize().unwrap().as_path())
        );
        let mut config = Config {
            mcp_servers: vec![server("user", "user")],
            plugin_mcp_servers: vec![
                PluginMcpContribution {
                    origin,
                    config: server("a", "a"),
                },
                PluginMcpContribution {
                    origin: PluginMcpOrigin::new(PluginMcpSource::User, a.path(), None).unwrap(),
                    config: server("global", "global"),
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            config.effective_mcp_servers().count(),
            2,
            "missing project identity denies project configuration"
        );
        config.set_mcp_project(a.path());
        assert_eq!(config.effective_mcp_servers().count(), 3);
        config.set_mcp_project(b.path());
        assert_eq!(config.plugin_mcp_servers.len(), 1);
        assert_eq!(config.effective_mcp_servers().count(), 2);
        config.set_mcp_project(a.path());
        assert_eq!(
            config.effective_mcp_servers().count(),
            2,
            "returning does not restore withdrawn project contributions"
        );
    }

    #[test]
    fn origins_require_canonical_roots_and_aliases_share_identity() {
        let root = tempfile::tempdir().unwrap();
        assert!(PluginMcpOrigin::new(PluginMcpSource::Project, root.path(), None).is_err());
        assert!(
            PluginMcpOrigin::new(PluginMcpSource::User, &root.path().join("missing"), None)
                .is_err()
        );
        assert!(PluginMcpOrigin::new(
            PluginMcpSource::Project,
            root.path(),
            Some(&root.path().join("missing"))
        )
        .is_err());
        let origin =
            PluginMcpOrigin::new(PluginMcpSource::Project, root.path(), Some(root.path())).unwrap();
        assert!(origin.matches_project(Some(&root.path().join("."))));
        assert!(!origin.matches_project(None));
        #[cfg(unix)]
        {
            let aliases = tempfile::tempdir().unwrap();
            let alias = aliases.path().join("alias");
            std::os::unix::fs::symlink(root.path(), &alias).unwrap();
            assert_eq!(
                origin,
                PluginMcpOrigin::new(PluginMcpSource::Project, &alias, Some(&alias)).unwrap()
            );
        }
    }
}
