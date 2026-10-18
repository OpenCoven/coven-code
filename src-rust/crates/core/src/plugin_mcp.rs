//! Runtime-only plugin MCP provenance. Settings-owned servers stay in Config::mcp_servers.

use crate::config::{Config, McpServerConfig};
use futures::future::{BoxFuture, FutureExt, Shared};
use parking_lot::Mutex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::watch;

type Retirement = Shared<BoxFuture<'static, Result<(), String>>>;

struct PluginMcpAuthority {
    revoked: watch::Sender<bool>,
    retirements: Mutex<Vec<Retirement>>,
}

impl std::fmt::Debug for PluginMcpAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginMcpAuthority")
            .field("revoked", &*self.revoked.borrow())
            .finish_non_exhaustive()
    }
}

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
    authority: Arc<PluginMcpAuthority>,
}

impl PluginMcpContribution {
    pub fn new(origin: PluginMcpOrigin, config: McpServerConfig) -> Self {
        Self {
            origin,
            config,
            authority: Arc::new(PluginMcpAuthority {
                revoked: watch::channel(false).0,
                retirements: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn is_revoked(&self) -> bool {
        *self.authority.revoked.borrow()
    }

    pub fn check_caller(&self, caller: Option<&Path>) -> anyhow::Result<()> {
        anyhow::ensure!(!self.is_revoked(), "Plugin MCP authority has been revoked");
        anyhow::ensure!(
            self.origin.matches_project(caller),
            "Plugin MCP caller project does not match its origin"
        );
        Ok(())
    }

    /// Admit a synchronous effect atomically with respect to withdrawal.
    pub fn authorize<T>(
        &self,
        caller: Option<&Path>,
        operation: impl FnOnce() -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let _registration = self.authority.retirements.lock();
        self.check_caller(caller)?;
        operation()
    }

    pub fn revocation(&self) -> watch::Receiver<bool> {
        self.authority.revoked.subscribe()
    }

    /// Deny new requests immediately; the independently owned cleanup tasks
    /// finish actual transport close before `wait_retired` can succeed.
    pub fn revoke(&self) {
        let _registration = self.authority.retirements.lock();
        self.authority.revoked.send_replace(true);
    }

    /// Register ownership before the task can launch anything. Revocation and
    /// admission share this short lock; cleanup never awaits while holding it.
    pub fn spawn_retirement<F>(
        &self,
        caller: Option<&Path>,
        task: impl FnOnce(watch::Receiver<bool>) -> F,
    ) -> anyhow::Result<()>
    where
        F: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let mut tasks = self.authority.retirements.lock();
        self.check_caller(caller)?;
        tasks.retain(|task| !matches!(task.clone().now_or_never(), Some(Ok(()))));
        let handle = tokio::spawn(task(self.revocation()));
        tasks.push(
            async move {
                handle
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())
            }
            .boxed()
            .shared(),
        );
        Ok(())
    }

    pub async fn wait_retired(&self) -> anyhow::Result<()> {
        let tasks = self.authority.retirements.lock().clone();
        let mut result = Ok(());
        for task in tasks {
            // Await every owner even after an earlier error; no detached cleanup
            // may be mistaken for completed retirement.
            result = result.and(task.await.map_err(anyhow::Error::msg));
        }
        result
    }

    pub async fn guard<T>(
        &self,
        caller: Option<&Path>,
        operation: impl std::future::Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<T> {
        let mut revoked = self.revocation();
        let (_completed, completion) = tokio::sync::oneshot::channel::<()>();
        {
            let mut tasks = self.authority.retirements.lock();
            self.check_caller(caller)?;
            tasks.retain(|task| !matches!(task.clone().now_or_never(), Some(Ok(()))));
            tasks.push(
                async move {
                    let _ = completion.await;
                    Ok(())
                }
                .boxed()
                .shared(),
            );
        }
        tokio::select! {
            biased;
            _ = revoked.changed() => anyhow::bail!("Plugin MCP authority has been revoked"),
            result = operation => {
                self.check_caller(caller)?;
                result
            }
        }
    }

    fn same_configuration(&self, other: &Self) -> bool {
        self.origin == other.origin
            && self.config.name == other.config.name
            && self.config.command == other.config.command
            && self.config.args == other.config.args
            && self.config.env == other.config.env
            && self.config.url == other.config.url
            && self.config.server_type == other.config.server_type
    }
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
                    && !entry.is_revoked()
                    && entry.origin.matches_project(self.project_dir.as_deref())
                    && names.insert(&entry.config.name)
                {
                    Some(&entry.config)
                } else {
                    None
                }
            }))
    }

    /// Keep identical eligible contributions on the same lifetime. Withdrawal
    /// revokes all clones before awaiting actual transport retirement, without
    /// holding any registry or state lock across cleanup.
    pub async fn reconcile_plugin_mcp_servers(
        &mut self,
        candidates: Vec<PluginMcpContribution>,
    ) -> anyhow::Result<()> {
        let old = std::mem::take(&mut self.plugin_mcp_servers);
        let mut next = Vec::new();
        let mut retired = Vec::new();
        let plugins_allowed = !(self.hosted_review_enabled() && !self.hosted_review.allow_plugins);
        for mut candidate in candidates {
            if !plugins_allowed
                || candidate.is_revoked()
                || !candidate
                    .origin
                    .matches_project(self.project_dir.as_deref())
                || self
                    .mcp_servers
                    .iter()
                    .any(|user| user.name == candidate.config.name)
                || next
                    .iter()
                    .any(|entry: &PluginMcpContribution| entry.config.name == candidate.config.name)
            {
                candidate.revoke();
                retired.push(candidate);
                continue;
            }
            if let Some(previous) = old
                .iter()
                .find(|previous| !previous.is_revoked() && previous.same_configuration(&candidate))
            {
                if !Arc::ptr_eq(&candidate.authority, &previous.authority) {
                    // A separately discovered candidate must not retain an
                    // independent authority after the existing lifetime wins.
                    candidate.revoke();
                    retired.push(candidate);
                    candidate = previous.clone();
                }
            }
            next.push(candidate);
        }
        retired.extend(old.into_iter().filter(|previous| {
            !next
                .iter()
                .any(|entry| Arc::ptr_eq(&previous.authority, &entry.authority))
        }));
        for previous in &retired {
            previous.revoke();
        }
        self.plugin_mcp_servers = next;
        let mut result = Ok(());
        for previous in retired {
            result = result.and(previous.wait_retired().await);
        }
        result
    }

    /// Foreign contributions are permanently revoked even in stale Config clones.
    pub async fn set_mcp_project(&mut self, project_dir: &Path) -> anyhow::Result<()> {
        self.project_dir = Some(project_dir.to_path_buf());
        self.reconcile_plugin_mcp_servers(self.plugin_mcp_servers.clone())
            .await
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
            plugin_mcp_servers: vec![PluginMcpContribution::new(
                PluginMcpOrigin::new(PluginMcpSource::User, root.path(), None).unwrap(),
                server("plugin", "plugin-command"),
            )],
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
            plugin_mcp_servers: vec![PluginMcpContribution::new(
                PluginMcpOrigin::new(PluginMcpSource::User, root.path(), None).unwrap(),
                server("shared", "plugin"),
            )],
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
        tokio::runtime::Runtime::new().unwrap().block_on(async {
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
                    PluginMcpContribution::new(origin, server("a", "a")),
                    PluginMcpContribution::new(
                        PluginMcpOrigin::new(PluginMcpSource::User, a.path(), None).unwrap(),
                        server("global", "global"),
                    ),
                ],
                ..Default::default()
            };
            assert_eq!(
                config.effective_mcp_servers().count(),
                2,
                "missing project identity denies project configuration"
            );
            config.set_mcp_project(a.path()).await.unwrap();
            assert_eq!(config.effective_mcp_servers().count(), 3);
            config.set_mcp_project(b.path()).await.unwrap();
            assert_eq!(config.plugin_mcp_servers.len(), 1);
            assert_eq!(config.effective_mcp_servers().count(), 2);
            config.set_mcp_project(a.path()).await.unwrap();
            assert_eq!(
                config.effective_mcp_servers().count(),
                2,
                "returning does not restore withdrawn project contributions"
            );
        });
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

#[cfg(test)]
mod authority_tests {
    use super::*;

    fn contribution(root: &Path, command: &str) -> PluginMcpContribution {
        PluginMcpContribution::new(
            PluginMcpOrigin::new(PluginMcpSource::User, root, None).unwrap(),
            McpServerConfig {
                name: "same".into(),
                command: Some(command.into()),
                args: vec![],
                env: Default::default(),
                url: None,
                server_type: "stdio".into(),
            },
        )
    }

    #[test]
    fn reconciliation_preserves_unchanged_authority_and_reenable_never_revives_old_clones() {
        let _env = crate::coven_shared::COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let root = tempfile::tempdir().unwrap();
            let original = contribution(root.path(), "one");
            let mut config = Config {
                plugin_mcp_servers: vec![original.clone()],
                ..Default::default()
            };
            config
                .reconcile_plugin_mcp_servers(vec![contribution(root.path(), "one")])
                .await
                .unwrap();
            assert!(!original.is_revoked());
            assert!(Arc::ptr_eq(
                &original.authority,
                &config.plugin_mcp_servers[0].authority
            ));
            config
                .reconcile_plugin_mcp_servers(Vec::new())
                .await
                .unwrap();
            assert!(original.is_revoked());
            config
                .reconcile_plugin_mcp_servers(vec![contribution(root.path(), "one")])
                .await
                .unwrap();
            assert!(!config.plugin_mcp_servers[0].is_revoked());
            assert!(original.is_revoked());
            let changed = config.plugin_mcp_servers[0].clone();
            config
                .reconcile_plugin_mcp_servers(vec![contribution(root.path(), "two")])
                .await
                .unwrap();
            assert!(changed.is_revoked());
            assert!(!config.plugin_mcp_servers[0].is_revoked());
        });
    }

    #[test]
    fn later_user_configuration_revokes_only_shadowed_plugin_authority() {
        let _env = crate::coven_shared::COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let root = tempfile::tempdir().unwrap();
            let plugin = contribution(root.path(), "plugin");
            let mut config = Config {
                plugin_mcp_servers: vec![plugin.clone()],
                ..Default::default()
            };
            let mut user = plugin.config.clone();
            user.command = Some("user-edited".into());
            config.mcp_servers.push(user);
            config
                .reconcile_plugin_mcp_servers(vec![contribution(root.path(), "plugin")])
                .await
                .unwrap();
            assert!(plugin.is_revoked());
            assert!(config.plugin_mcp_servers.is_empty());
            assert_eq!(
                config.mcp_servers[0].command.as_deref(),
                Some("user-edited")
            );
        });
    }
    #[tokio::test]
    async fn retirement_preserves_completed_cleanup_failures_after_new_requests() {
        let root = tempfile::tempdir().unwrap();
        let plugin = contribution(root.path(), "fixture");
        plugin
            .spawn_retirement(None, |_| async { anyhow::bail!("fixture close failure") })
            .unwrap();
        assert!(plugin.wait_retired().await.is_err());
        plugin.guard(None, async { Ok(()) }).await.unwrap();
        plugin.revoke();
        assert!(
            plugin.wait_retired().await.is_err(),
            "new admission must not erase a failed cleanup receipt"
        );
    }

    #[tokio::test]
    async fn retirement_awaits_every_owner_even_when_one_cleanup_fails() {
        let root = tempfile::tempdir().unwrap();
        let plugin = contribution(root.path(), "fixture");
        let (finished, completion) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        plugin
            .spawn_retirement(None, |_| async { anyhow::bail!("fixture close failure") })
            .unwrap();
        assert!(plugin.wait_retired().await.is_err());
        plugin
            .spawn_retirement(None, |_| async move {
                released.await.unwrap();
                let _ = finished.send(());
                Ok(())
            })
            .unwrap();
        plugin.revoke();
        let retirement = plugin.wait_retired();
        tokio::pin!(retirement);
        assert!(
            futures::poll!(&mut retirement).is_pending(),
            "an earlier error must not skip outstanding cleanup"
        );
        release.send(()).unwrap();
        assert!(retirement.await.is_err());
        completion.await.unwrap();
    }

    #[test]
    fn reconciliation_retires_discarded_candidate_owners_too() {
        let _env = crate::coven_shared::COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let root = tempfile::tempdir().unwrap();
            let existing = contribution(root.path(), "one");
            let duplicate = contribution(root.path(), "one");
            let (closed, receipt) = tokio::sync::oneshot::channel();
            duplicate
                .spawn_retirement(None, |mut revoked| async move {
                    let _ = revoked.changed().await;
                    let _ = closed.send(());
                    Ok(())
                })
                .unwrap();
            let mut config = Config {
                plugin_mcp_servers: vec![existing.clone()],
                ..Default::default()
            };
            config
                .reconcile_plugin_mcp_servers(vec![duplicate.clone()])
                .await
                .unwrap();
            assert!(
                duplicate.is_revoked(),
                "a replaced candidate authority cannot remain live independently"
            );
            receipt.await.unwrap();
            assert!(!existing.is_revoked());
        });
    }
}
