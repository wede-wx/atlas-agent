use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::{AgentError, AgentRun};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunMode {
    Chat,
    Task,
    Team,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRuntimeConfig {
    pub mode: AgentRunMode,
    pub max_iterations: usize,
    pub max_tool_calls: usize,
    pub max_consecutive_tool_errors: usize,
}

impl Default for AgentRuntimeConfig {
    fn default() -> Self {
        Self {
            mode: AgentRunMode::Chat,
            max_iterations: 10,
            max_tool_calls: 24,
            max_consecutive_tool_errors: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRuntimeState {
    pub run_id: String,
    pub mode: AgentRunMode,
    pub iteration: usize,
    pub tool_call_count: usize,
    pub consecutive_tool_errors: usize,
    pub cancelled: bool,
}

impl AgentRuntimeState {
    pub fn as_agent_run(&self) -> AgentRun {
        AgentRun {
            id: self.run_id.clone(),
            iteration: self.iteration,
            tool_calls: vec![],
            retryable: true,
            cancelled: self.cancelled,
        }
    }
}

pub struct AgentRuntime {
    config: AgentRuntimeConfig,
    state: AgentRuntimeState,
}

impl AgentRuntime {
    pub fn new(config: AgentRuntimeConfig) -> Self {
        Self::new_with_run_id(config, format!("run_{}", Uuid::new_v4()))
    }

    pub fn new_with_run_id(config: AgentRuntimeConfig, run_id: String) -> Self {
        Self {
            state: AgentRuntimeState {
                run_id,
                mode: config.mode.clone(),
                iteration: 0,
                tool_call_count: 0,
                consecutive_tool_errors: 0,
                cancelled: false,
            },
            config,
        }
    }

    pub fn config(&self) -> &AgentRuntimeConfig {
        &self.config
    }

    pub fn state(&self) -> &AgentRuntimeState {
        &self.state
    }

    pub fn run_id(&self) -> &str {
        &self.state.run_id
    }

    pub fn begin_iteration(&mut self) -> Result<usize, AgentError> {
        if self.state.cancelled {
            return Err(AgentError::Cancelled);
        }
        if self.state.iteration >= self.config.max_iterations {
            return Err(AgentError::MaxIterations);
        }
        self.state.iteration += 1;
        Ok(self.state.iteration)
    }

    pub fn record_tool_call(&mut self) -> Result<(), AgentError> {
        self.state.tool_call_count += 1;
        if self.state.tool_call_count > self.config.max_tool_calls {
            return Err(AgentError::Tool(format!(
                "Tool call limit reached: {}",
                self.config.max_tool_calls
            )));
        }
        Ok(())
    }

    pub fn record_tool_success(&mut self) {
        self.state.consecutive_tool_errors = 0;
    }

    pub fn record_tool_error(&mut self) -> Result<(), AgentError> {
        self.state.consecutive_tool_errors += 1;
        if self.state.consecutive_tool_errors >= self.config.max_consecutive_tool_errors {
            return Err(AgentError::Tool(format!(
                "Too many consecutive tool errors: {}",
                self.state.consecutive_tool_errors
            )));
        }
        Ok(())
    }

    pub fn cancel(&mut self) {
        self.state.cancelled = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_enforces_iteration_limit() {
        let mut runtime = AgentRuntime::new(AgentRuntimeConfig {
            max_iterations: 1,
            ..Default::default()
        });
        assert_eq!(runtime.begin_iteration().unwrap(), 1);
        assert!(matches!(
            runtime.begin_iteration().unwrap_err(),
            AgentError::MaxIterations
        ));
    }

    #[test]
    fn runtime_tracks_tool_error_budget() {
        let mut runtime = AgentRuntime::new(AgentRuntimeConfig {
            max_consecutive_tool_errors: 2,
            ..Default::default()
        });
        assert!(runtime.record_tool_error().is_ok());
        assert!(runtime.record_tool_error().is_err());
        runtime.record_tool_success();
        assert!(runtime.record_tool_error().is_ok());
    }
}
