use super::*;
use claurst_core::plugin_mcp::{PluginMcpContribution, PluginMcpOrigin, PluginMcpSource};
use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

static STATE: Mutex<()> = Mutex::new(());

struct Fixture {
    root: tempfile::TempDir,
    previous_engine: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl Fixture {
    fn new() -> Self {
        let lock = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let root = tempfile::tempdir().unwrap();
        let previous_engine = std::env::var_os("COVEN_CODE_HOME");
        std::env::set_var("COVEN_CODE_HOME", root.path().join("engine"));
        Self {
            root,
            previous_engine,
            _lock: lock,
        }
    }

    fn project(&self, name: &str) -> PathBuf {
        let project = self.root.path().join(name);
        std::fs::create_dir_all(&project).unwrap();
        project
    }

    fn contribution(
        &self,
        project: &std::path::Path,
        name: &str,
        source: PluginMcpSource,
    ) -> PluginMcpContribution {
        PluginMcpContribution::new(
            PluginMcpOrigin::new(source, project, Some(project)).unwrap(),
            claurst_core::McpServerConfig {
                name: name.into(),
                command: Some("sh".into()),
                args: vec![
                    "-c".into(),
                    "printf x >> \"$MCP_PROJECT_MARKER\"; exit 0".into(),
                ],
                env: [(
                    "MCP_PROJECT_MARKER".into(),
                    self.root
                        .path()
                        .join(name)
                        .with_extension("starts")
                        .display()
                        .to_string(),
                )]
                .into_iter()
                .collect(),
                url: None,
                server_type: "stdio".into(),
            },
        )
    }

    async fn connect(&self, config: &Config) {
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            connect_mcp_manager_arc(config, config.project_dir.as_deref()),
        )
        .await
        .expect("stdio marker must finish");
        assert_eq!(
            result.as_ref().map_or(0, |manager| manager.server_count()),
            0
        );
    }

    fn starts(&self, name: &str) -> usize {
        std::fs::read(self.root.path().join(name).with_extension("starts"))
            .map_or(0, |bytes| bytes.len())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        claurst_plugins::set_global_hooks(Default::default());
        claurst_plugins::set_global_registry(Default::default());
        match &self.previous_engine {
            Some(value) => std::env::set_var("COVEN_CODE_HOME", value),
            None => std::env::remove_var("COVEN_CODE_HOME"),
        }
    }
}

#[test]
fn resume_configuration_withdraws_old_project_before_real_reconnect() {
    let fixture = Fixture::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = fixture.project("a");
        let b = fixture.project("b");
        let mut config = Config {
            project_dir: Some(a.clone()),
            mcp_servers: vec![
                fixture
                    .contribution(&a, "user", PluginMcpSource::User)
                    .config,
            ],
            plugin_mcp_servers: vec![fixture.contribution(&a, "project", PluginMcpSource::Project)],
            ..Default::default()
        };
        fixture.connect(&config).await;
        assert_eq!(fixture.starts("project"), 1);
        assert_eq!(fixture.starts("user"), 1);
        claurst_plugins::set_global_registry(claurst_plugins::load_plugins(&a, &[]).await);
        // Both the launch --resume and in-session /resume paths call this helper.
        refresh_mcp_project_config(&mut config, &b).await.unwrap();
        assert!(config.plugin_mcp_servers.is_empty());
        fixture.connect(&config).await;
        assert_eq!(fixture.starts("project"), 1);
        assert_eq!(fixture.starts("user"), 2);
        assert!(claurst_plugins::global_plugin_registry()
            .unwrap()
            .matches_project(&b));
    });
}

#[test]
fn hosted_review_filters_plugin_contributions_and_all_mcp_launches() {
    let fixture = Fixture::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = fixture.project("a");
        let mut config = Config {
            project_dir: Some(a.clone()),
            mcp_servers: vec![
                fixture
                    .contribution(&a, "user", PluginMcpSource::User)
                    .config,
            ],
            plugin_mcp_servers: vec![fixture.contribution(&a, "plugin", PluginMcpSource::User)],
            ..Default::default()
        };
        config.hosted_review.enabled = true;
        config.hosted_review.allow_mcp_servers = true;
        config.hosted_review.allow_plugins = false;
        fixture.connect(&config).await;
        assert_eq!(fixture.starts("user"), 1);
        assert_eq!(fixture.starts("plugin"), 0);
        config.hosted_review.allow_mcp_servers = false;
        fixture.connect(&config).await;
        assert_eq!(fixture.starts("user"), 1);
        assert_eq!(fixture.starts("plugin"), 0);
    });
}

#[test]
fn stale_config_cannot_relaunch_a_withdrawn_contribution() {
    let fixture = Fixture::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = fixture.project("a");
        let b = fixture.project("b");
        let mut config = Config {
            project_dir: Some(a.clone()),
            plugin_mcp_servers: vec![fixture.contribution(
                &a,
                "withdrawn",
                PluginMcpSource::Project,
            )],
            ..Default::default()
        };
        for caller in [None, Some(b.as_path()), Some(a.join("missing").as_path())] {
            let manager = connect_mcp_manager_arc(&config, caller).await.unwrap();
            assert_eq!(manager.server_count(), 0);
        }
        assert_eq!(
            fixture.starts("withdrawn"),
            0,
            "unresolved or foreign launch caller must fail before spawn"
        );
        let stale = config.clone();
        config.set_mcp_project(&b).await.unwrap();
        fixture.connect(&stale).await;
        assert_eq!(
            fixture.starts("withdrawn"),
            0,
            "revoked stale config must not reach actual stdio launch"
        );
        // Fresh enable is a new lifetime, and cannot revive the old clone.
        config.set_mcp_project(&a).await.unwrap();
        config
            .reconcile_plugin_mcp_servers(vec![fixture.contribution(
                &a,
                "withdrawn",
                PluginMcpSource::Project,
            )])
            .await
            .unwrap();
        let alias = a.join(".");
        let manager = connect_mcp_manager_arc(&config, Some(&alias))
            .await
            .unwrap();
        assert_eq!(
            manager.failed_servers().len(),
            1,
            "canonical alias reaches the marker child"
        );
        fixture.connect(&stale).await;
        assert_eq!(fixture.starts("withdrawn"), 1);
    });
}
