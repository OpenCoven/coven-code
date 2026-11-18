//! Finite model-round budgets, independent of spend, tokens and permissions.
use std::num::NonZeroU32;

use serde::Serialize;

use crate::QueryConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnBudgetSource {
    Explicit,
    Agent,
    Configured,
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EffectiveTurnBudget {
    pub max_turns: NonZeroU32,
    pub source: TurnBudgetSource,
}

impl QueryConfig {
    /// Explicit caller selection wins, including an explicit default value.
    /// Agent null inherits; zero is invalid and never means unlimited.
    pub fn effective_turn_budget(&self) -> Result<EffectiveTurnBudget, &'static str> {
        let (value, source) = if let Some(limit) = self.max_turns_override {
            (limit.get(), TurnBudgetSource::Explicit)
        } else if let Some(limit) = self.agent_definition.as_ref().and_then(|a| a.max_turns) {
            (limit, TurnBudgetSource::Agent)
        } else {
            (
                self.max_turns,
                if self.max_turns == claurst_core::constants::MAX_TURNS_DEFAULT {
                    TurnBudgetSource::Default
                } else {
                    TurnBudgetSource::Configured
                },
            )
        };
        Ok(EffectiveTurnBudget {
            max_turns: NonZeroU32::new(value).ok_or("Model-round budget must be positive")?,
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_and_invalid_inherited_limits() {
        let mut config = QueryConfig::default();
        assert_eq!(
            config.effective_turn_budget().unwrap().source,
            TurnBudgetSource::Default
        );
        config.max_turns = 40;
        assert_eq!(config.effective_turn_budget().unwrap().max_turns.get(), 40);
        let agent = claurst_core::AgentDefinition {
            max_turns: Some(3),
            ..claurst_core::AgentDefinition::default()
        };
        config.agent_definition = Some(agent);
        assert_eq!(config.effective_turn_budget().unwrap().max_turns.get(), 3);
        config.max_turns_override = NonZeroU32::new(10);
        assert_eq!(
            config.effective_turn_budget().unwrap(),
            EffectiveTurnBudget {
                max_turns: NonZeroU32::new(10).unwrap(),
                source: TurnBudgetSource::Explicit
            }
        );
        config.agent_definition.as_mut().unwrap().max_turns = Some(0);
        assert!(
            config.effective_turn_budget().is_ok(),
            "explicit budget wins"
        );
        config.max_turns_override = None;
        assert!(config.effective_turn_budget().is_err());
        config.agent_definition.as_mut().unwrap().max_turns = None;
        assert_eq!(
            config.effective_turn_budget().unwrap().source,
            TurnBudgetSource::Configured
        );
        config.max_turns = 0;
        assert!(config.effective_turn_budget().is_err());
    }
}
