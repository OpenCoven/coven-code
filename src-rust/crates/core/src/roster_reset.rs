use std::path::{Path, PathBuf};

use crate::config::Settings;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResetRosterSummary {
    pub removed_agent_files: usize,
    pub removed_familiar_roster: bool,
    pub cleared_settings_keys: usize,
}

impl ResetRosterSummary {
    pub fn changed(&self) -> bool {
        self.removed_agent_files > 0
            || self.removed_familiar_roster
            || self.cleared_settings_keys > 0
    }

    pub fn message(&self) -> String {
        if !self.changed() {
            return "No saved familiars or agents to reset.".to_string();
        }

        let roster_status = if self.removed_familiar_roster {
            "removed familiar roster"
        } else {
            "no familiar roster"
        };
        format!(
            "Reset familiars and agents: removed {} agent file{}, {}, cleared {} settings key{}.",
            self.removed_agent_files,
            plural(self.removed_agent_files),
            roster_status,
            self.cleared_settings_keys,
            plural(self.cleared_settings_keys)
        )
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

pub fn reset_familiars_and_agents(
    project_root: Option<&Path>,
) -> anyhow::Result<ResetRosterSummary> {
    let mut summary = ResetRosterSummary::default();

    let mut agent_dirs = vec![Settings::config_dir().join("agents")];
    if let Some(root) = project_root {
        agent_dirs.push(root.join(".coven-code").join("agents"));
    }
    for dir in agent_dirs {
        summary.removed_agent_files += remove_agent_markdown_files(&dir)?;
    }

    let familiar_roster = coven_home_path().join("familiars.toml");
    match std::fs::remove_file(&familiar_roster) {
        Ok(()) => summary.removed_familiar_roster = true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }

    if Settings::global_settings_path().exists() {
        let mut settings = Settings::load_sync()?;
        summary.cleared_settings_keys = clear_settings_roster_keys(&mut settings);
        if summary.cleared_settings_keys > 0 {
            settings.save_sync()?;
        }
    }

    Ok(summary)
}

fn remove_agent_markdown_files(dir: &Path) -> anyhow::Result<usize> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(0);
    };

    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md") {
            std::fs::remove_file(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn coven_home_path() -> PathBuf {
    if let Ok(path) = std::env::var("COVEN_HOME") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".coven")
}

fn clear_settings_roster_keys(settings: &mut Settings) -> usize {
    let mut cleared = 0;

    if !settings.agents.is_empty() {
        settings.agents.clear();
        cleared += 1;
    }
    if !settings.config.agents.is_empty() {
        settings.config.agents.clear();
        cleared += 1;
    }
    if settings.familiar.take().is_some() {
        cleared += 1;
    }
    if settings.config.familiar.take().is_some() {
        cleared += 1;
    }
    if settings.managed_agents.take().is_some() {
        cleared += 1;
    }
    if settings.config.managed_agents.take().is_some() {
        cleared += 1;
    }

    cleared
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentDefinition, ManagedAgentConfig, Settings};
    use crate::coven_shared::COVEN_HOME_ENV_LOCK;

    struct HomeGuard {
        old_home: Option<String>,
        old_coven_home: Option<String>,
    }

    impl HomeGuard {
        fn set(home: &Path, coven_home: &Path) -> Self {
            let old_home = std::env::var("HOME").ok();
            let old_coven_home = std::env::var("COVEN_HOME").ok();
            std::env::set_var("HOME", home);
            std::env::set_var("COVEN_HOME", coven_home);
            Self {
                old_home,
                old_coven_home,
            }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.old_coven_home {
                Some(value) => std::env::set_var("COVEN_HOME", value),
                None => std::env::remove_var("COVEN_HOME"),
            }
        }
    }

    fn test_agent() -> AgentDefinition {
        AgentDefinition {
            description: Some("custom".to_string()),
            model: None,
            temperature: None,
            prompt: Some("custom prompt".to_string()),
            access: "full".to_string(),
            visible: true,
            max_turns: None,
            color: None,
        }
    }

    fn test_managed_agents() -> ManagedAgentConfig {
        ManagedAgentConfig {
            enabled: true,
            manager_model: "anthropic/claude-opus-4-6".to_string(),
            executor_model: "anthropic/claude-sonnet-4-6".to_string(),
            executor_max_turns: 10,
            max_concurrent_executors: 2,
            budget_split: Default::default(),
            total_budget_usd: Some(5.0),
            preset_name: Some("test".to_string()),
            executor_isolation: true,
        }
    }

    #[test]
    fn reset_removes_user_roster_state_without_touching_unrelated_files() {
        let _lock = COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let coven_home = temp.path().join("coven");
        let project = temp.path().join("project");
        let global_agents = home.join(".coven-code").join("agents");
        let project_agents = project.join(".coven-code").join("agents");
        std::fs::create_dir_all(&global_agents).expect("global agents dir");
        std::fs::create_dir_all(&project_agents).expect("project agents dir");
        std::fs::create_dir_all(&coven_home).expect("coven home");
        let _guard = HomeGuard::set(&home, &coven_home);

        std::fs::write(global_agents.join("global.md"), "global").expect("global agent");
        std::fs::write(global_agents.join("README.txt"), "keep").expect("global keep");
        std::fs::write(project_agents.join("project.md"), "project").expect("project agent");
        std::fs::write(project_agents.join("notes.txt"), "keep").expect("project keep");
        std::fs::write(
            coven_home.join("familiars.toml"),
            "[[familiar]]\nid = \"nova\"\n",
        )
        .expect("familiars");

        let mut settings = Settings::default();
        settings.agents.insert("global".to_string(), test_agent());
        settings
            .config
            .agents
            .insert("nested".to_string(), test_agent());
        settings.familiar = Some("nova".to_string());
        settings.config.familiar = Some("sage".to_string());
        settings.managed_agents = Some(test_managed_agents());
        settings.config.managed_agents = Some(test_managed_agents());
        settings.save_sync().expect("settings save");

        let summary = reset_familiars_and_agents(Some(&project)).expect("reset");

        assert_eq!(summary.removed_agent_files, 2);
        assert!(summary.removed_familiar_roster);
        assert_eq!(summary.cleared_settings_keys, 6);
        assert!(!global_agents.join("global.md").exists());
        assert!(global_agents.join("README.txt").exists());
        assert!(!project_agents.join("project.md").exists());
        assert!(project_agents.join("notes.txt").exists());
        assert!(!coven_home.join("familiars.toml").exists());

        let updated = Settings::load_sync().expect("load settings");
        assert!(updated.agents.is_empty());
        assert!(updated.config.agents.is_empty());
        assert!(updated.familiar.is_none());
        assert!(updated.config.familiar.is_none());
        assert!(updated.managed_agents.is_none());
        assert!(updated.config.managed_agents.is_none());
    }

    #[test]
    fn reset_reports_no_change_when_roster_state_is_absent() {
        let _lock = COVEN_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let coven_home = temp.path().join("coven");
        std::fs::create_dir_all(&home).expect("home");
        std::fs::create_dir_all(&coven_home).expect("coven home");
        let _guard = HomeGuard::set(&home, &coven_home);

        let summary = reset_familiars_and_agents(None).expect("reset");

        assert_eq!(summary, ResetRosterSummary::default());
        assert!(!summary.changed());
        assert!(!Settings::global_settings_path().exists());
    }
}
