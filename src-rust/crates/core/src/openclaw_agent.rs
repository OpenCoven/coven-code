//! Read-only OpenClaw agent installation detection.
//!
//! A leftover `~/.openclaw` directory is not enough to prove a usable
//! OpenClaw agent exists. Coven Code integrations use this module to
//! distinguish recoverable old data from loadable agent profiles.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenClawAgentDetectionBranch {
    NoOpenClawDir,
    ValidAgentFound,
    StaleOpenClawData,
    StaleAgentRecords,
    MalformedConfig,
}

impl OpenClawAgentDetectionBranch {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoOpenClawDir => "no_openclaw_dir",
            Self::ValidAgentFound => "valid_agent_found",
            Self::StaleOpenClawData => "stale_openclaw_data",
            Self::StaleAgentRecords => "stale_agent_records",
            Self::MalformedConfig => "malformed_config",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenClawAgentSummary {
    pub id: String,
    pub name: Option<String>,
    pub profile_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenClawAgentDetection {
    pub branch: OpenClawAgentDetectionBranch,
    pub home: PathBuf,
    pub valid_agents: Vec<OpenClawAgentSummary>,
    pub stale_agents: Vec<String>,
    pub guidance: String,
}

impl OpenClawAgentDetection {
    pub fn has_loadable_agent(&self) -> bool {
        !self.valid_agents.is_empty()
    }
}

#[derive(Debug, Deserialize)]
struct OpenClawConfigFile {
    #[serde(default)]
    agents: Option<OpenClawAgentsConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenClawAgentsConfig {
    Wrapped {
        #[serde(default)]
        list: Vec<OpenClawConfigAgent>,
    },
    List(Vec<OpenClawConfigAgent>),
}

impl OpenClawAgentsConfig {
    fn into_list(self) -> Vec<OpenClawConfigAgent> {
        match self {
            Self::Wrapped { list } => list,
            Self::List(list) => list,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct OpenClawConfigAgent {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    workspace_path: Option<String>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    profile_path: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenClawRuntimeRegistry {
    #[serde(default)]
    agents: Vec<OpenClawRuntimeAgent>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenClawRuntimeAgent {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

pub fn detect_openclaw_agent_installation() -> OpenClawAgentDetection {
    let home = std::env::var_os("OPENCLAW_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".openclaw")))
        .unwrap_or_else(|| PathBuf::from(".openclaw"));
    detect_openclaw_agent_installation_at(&home)
}

pub fn detect_openclaw_agent_installation_at(home: &Path) -> OpenClawAgentDetection {
    let home = home.to_path_buf();
    if !home.is_dir() {
        return detection(
            OpenClawAgentDetectionBranch::NoOpenClawDir,
            home,
            Vec::new(),
            Vec::new(),
        );
    }

    let mut config_agents = match load_config_agents(&home) {
        Ok(agents) => agents,
        Err(_) => {
            return detection(
                OpenClawAgentDetectionBranch::MalformedConfig,
                home,
                Vec::new(),
                Vec::new(),
            );
        }
    };
    config_agents.extend(load_runtime_agents(&home));

    let mut valid_agents = Vec::new();
    let mut stale_agents = Vec::new();
    for agent in config_agents {
        let id = agent.id.trim();
        if id.is_empty() {
            continue;
        }
        match resolve_agent_profile(&home, &agent) {
            Some(profile_path) => valid_agents.push(OpenClawAgentSummary {
                id: id.to_string(),
                name: agent.name.filter(|name| !name.trim().is_empty()),
                profile_path,
            }),
            None => stale_agents.push(id.to_string()),
        }
    }

    valid_agents.sort_by(|a, b| a.id.cmp(&b.id));
    valid_agents.dedup_by(|a, b| a.id == b.id);
    stale_agents.sort();
    stale_agents.dedup();
    stale_agents.retain(|id| !valid_agents.iter().any(|agent| agent.id == *id));

    let branch = if !valid_agents.is_empty() {
        OpenClawAgentDetectionBranch::ValidAgentFound
    } else if !stale_agents.is_empty() {
        OpenClawAgentDetectionBranch::StaleAgentRecords
    } else {
        OpenClawAgentDetectionBranch::StaleOpenClawData
    };
    detection(branch, home, valid_agents, stale_agents)
}

fn detection(
    branch: OpenClawAgentDetectionBranch,
    home: PathBuf,
    valid_agents: Vec<OpenClawAgentSummary>,
    stale_agents: Vec<String>,
) -> OpenClawAgentDetection {
    tracing::info!(
        openclaw_agent_detection_branch = branch.as_str(),
        valid_agent_count = valid_agents.len(),
        stale_agent_count = stale_agents.len(),
        openclaw_home = %home.display(),
        "checked OpenClaw agent installation"
    );
    OpenClawAgentDetection {
        branch,
        home,
        valid_agents,
        stale_agents,
        guidance: guidance_for(branch),
    }
}

fn guidance_for(branch: OpenClawAgentDetectionBranch) -> String {
    match branch {
        OpenClawAgentDetectionBranch::NoOpenClawDir => {
            "No OpenClaw data found. Create or import an OpenClaw agent to use OpenClaw-backed flows."
                .to_string()
        }
        OpenClawAgentDetectionBranch::ValidAgentFound => {
            "OpenClaw agent found. OpenClaw-backed flows can use the existing agent."
                .to_string()
        }
        OpenClawAgentDetectionBranch::StaleOpenClawData => {
            "OpenClaw data found, but no loadable OpenClaw agent was found. Create or import an agent; existing data will be preserved."
                .to_string()
        }
        OpenClawAgentDetectionBranch::StaleAgentRecords => {
            "OpenClaw data found, but registered agent records are missing their profile/workspace. Repair, import, or create a new agent; existing data will be preserved."
                .to_string()
        }
        OpenClawAgentDetectionBranch::MalformedConfig => {
            "OpenClaw config could not be read. Use repair or import to recover; existing .openclaw data should be preserved."
                .to_string()
        }
    }
}

fn load_config_agents(home: &Path) -> Result<Vec<OpenClawConfigAgent>, serde_json::Error> {
    let path = home.join("openclaw.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(Vec::new()),
    };
    let config: OpenClawConfigFile = serde_json::from_slice(&bytes)?;
    Ok(config
        .agents
        .map(OpenClawAgentsConfig::into_list)
        .unwrap_or_default())
}

fn load_runtime_agents(home: &Path) -> Vec<OpenClawConfigAgent> {
    let path = home.join("registry").join("agents.json");
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(registry) = serde_json::from_slice::<OpenClawRuntimeRegistry>(&bytes) else {
        return Vec::new();
    };
    registry
        .agents
        .into_iter()
        .map(|agent| OpenClawConfigAgent {
            id: agent.id,
            name: agent.name,
            workspace: None,
            workspace_path: None,
            profile: None,
            profile_path: None,
            cwd: agent.cwd,
        })
        .collect()
}

fn resolve_agent_profile(home: &Path, agent: &OpenClawConfigAgent) -> Option<PathBuf> {
    explicit_agent_paths(home, agent)
        .into_iter()
        .chain(default_agent_paths(home, agent.id.trim()))
        .find(|path| is_loadable_agent_profile(path))
}

fn explicit_agent_paths(home: &Path, agent: &OpenClawConfigAgent) -> Vec<PathBuf> {
    [
        agent.workspace.as_deref(),
        agent.workspace_path.as_deref(),
        agent.profile.as_deref(),
        agent.profile_path.as_deref(),
        agent.cwd.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|path| !path.trim().is_empty())
    .map(|path| {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            home.join(path)
        }
    })
    .collect()
}

fn default_agent_paths(home: &Path, id: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if id == "main" {
        paths.push(home.join("workspace"));
    }
    paths.push(home.join("workspace").join(id));
    paths.push(home.join("agents").join(id));
    paths
}

fn is_loadable_agent_profile(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    ["IDENTITY.md", "SOUL.md", "USER.md", "AGENTS.md"]
        .iter()
        .any(|file| path.join(file).is_file())
        || path.join("agent").join("models.json").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn openclaw_home(root: &TempDir) -> std::path::PathBuf {
        root.path().join(".openclaw")
    }

    #[test]
    fn missing_openclaw_home_is_not_installed() {
        let root = tempfile::tempdir().unwrap();

        let detection = detect_openclaw_agent_installation_at(&openclaw_home(&root));

        assert_eq!(
            detection.branch,
            OpenClawAgentDetectionBranch::NoOpenClawDir
        );
        assert!(!detection.has_loadable_agent());
    }

    #[test]
    fn empty_openclaw_home_is_data_only_not_agent() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(openclaw_home(&root)).unwrap();

        let detection = detect_openclaw_agent_installation_at(&openclaw_home(&root));

        assert_eq!(
            detection.branch,
            OpenClawAgentDetectionBranch::StaleOpenClawData
        );
        assert!(!detection.has_loadable_agent());
        assert!(detection
            .guidance
            .contains("OpenClaw data found, but no loadable OpenClaw agent"));
    }

    #[test]
    fn malformed_openclaw_config_is_broken_not_agent() {
        let root = tempfile::tempdir().unwrap();
        let home = openclaw_home(&root);
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("openclaw.json"), "{not json").unwrap();

        let detection = detect_openclaw_agent_installation_at(&home);

        assert_eq!(
            detection.branch,
            OpenClawAgentDetectionBranch::MalformedConfig
        );
        assert!(!detection.has_loadable_agent());
        assert!(detection.guidance.contains("repair or import"));
    }

    #[test]
    fn configured_agent_with_missing_profile_is_stale() {
        let root = tempfile::tempdir().unwrap();
        let home = openclaw_home(&root);
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("openclaw.json"),
            r#"{"agents":{"list":[{"id":"cody","name":"Cody"}]}}"#,
        )
        .unwrap();

        let detection = detect_openclaw_agent_installation_at(&home);

        assert_eq!(
            detection.branch,
            OpenClawAgentDetectionBranch::StaleAgentRecords
        );
        assert!(!detection.has_loadable_agent());
        assert_eq!(detection.stale_agents, vec!["cody"]);
    }

    #[test]
    fn valid_agent_wins_when_other_records_are_stale() {
        let root = tempfile::tempdir().unwrap();
        let home = openclaw_home(&root);
        fs::create_dir_all(home.join("workspace").join("cody")).unwrap();
        fs::write(
            home.join("workspace").join("cody").join("IDENTITY.md"),
            "# Cody\n",
        )
        .unwrap();
        fs::write(
            home.join("openclaw.json"),
            r#"{"agents":{"list":[{"id":"cody","name":"Cody"},{"id":"missing"}]}}"#,
        )
        .unwrap();

        let detection = detect_openclaw_agent_installation_at(&home);

        assert_eq!(
            detection.branch,
            OpenClawAgentDetectionBranch::ValidAgentFound
        );
        assert!(detection.has_loadable_agent());
        assert_eq!(detection.valid_agents[0].id, "cody");
        assert_eq!(detection.stale_agents, vec!["missing"]);
    }

    #[test]
    fn runtime_registry_can_identify_existing_agent_profile() {
        let root = tempfile::tempdir().unwrap();
        let home = openclaw_home(&root);
        let profile = home.join("workspace").join("cody");
        fs::create_dir_all(home.join("registry")).unwrap();
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join("SOUL.md"), "# Cody\n").unwrap();
        fs::write(
            home.join("registry").join("agents.json"),
            format!(
                r#"{{"agents":[{{"id":"cody","name":"Cody","cwd":{}}}]}}"#,
                serde_json::to_string(profile.to_str().unwrap()).unwrap()
            ),
        )
        .unwrap();

        let detection = detect_openclaw_agent_installation_at(&home);

        assert_eq!(
            detection.branch,
            OpenClawAgentDetectionBranch::ValidAgentFound
        );
        assert!(detection.has_loadable_agent());
        assert_eq!(detection.valid_agents[0].id, "cody");
    }
}
