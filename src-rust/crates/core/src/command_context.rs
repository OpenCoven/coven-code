use serde::{Deserialize, Serialize};

/// Signals derived from the current session and environment for command discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandContextSignal {
    Unauthenticated,
    EmptySession,
    MissingAgentsFile,
    GitDirty,
    HasCompletedEdits,
    RunningTurn,
    HighContextUsage,
    HasBackgroundTasks,
    UpdateAvailable,
    FamiliarAvailable,
    CovenDaemonAvailable,
}
