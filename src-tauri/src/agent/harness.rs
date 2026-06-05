use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use crate::agent::{
    Agent, AgentError, AgentEvent, AgentGuidanceMessage, AgentRuntimeConfig, AgentToolAuditEvent,
    ChatResponse, LLMClient, Message, ToolAccessPolicy, ToolResult, ToolSchema,
};
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolRegistry, ToolSafetyLevel};

#[derive(Debug, Clone)]
pub struct ScriptedLlmCall {
    pub messages: Vec<Message>,
    pub tools: Option<Vec<ToolSchema>>,
}

#[derive(Clone)]
pub struct ScriptedLlm {
    responses: Arc<Mutex<VecDeque<ChatResponse>>>,
    calls: Arc<Mutex<Vec<ScriptedLlmCall>>>,
}

impl ScriptedLlm {
    pub fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub fn calls(&self) -> Vec<ScriptedLlmCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl LLMClient for ScriptedLlm {
    async fn chat_completion(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
    ) -> Result<ChatResponse, AgentError> {
        self.calls
            .lock()
            .unwrap()
            .push(ScriptedLlmCall { messages, tools });

        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AgentError::Llm("测试模型没有更多回复。".to_string()))
    }
}

#[derive(Clone)]
pub struct RecordedToolCalls {
    calls: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl RecordedToolCalls {
    pub fn entries(&self) -> Vec<serde_json::Value> {
        self.calls.lock().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub struct RecordingTool {
    name: String,
    description: String,
    result: ToolResult,
    metadata: ToolMetadata,
    calls: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl RecordingTool {
    pub fn success(
        name: impl Into<String>,
        description: impl Into<String>,
        result: ToolResult,
    ) -> (Self, RecordedToolCalls) {
        let name = name.into();
        let description = description.into();
        let metadata = ToolMetadata {
            name: name.clone(),
            description: description.clone(),
            label_zh: "Harness 记录工具".to_string(),
            description_zh: "测试夹具工具，只记录调用并返回脚本化结果。".to_string(),
            capability_labels_zh: vec!["只读".to_string(), "测试".to_string()],
            safety_label_zh: "安全".to_string(),
            capabilities: vec![ToolCapability::ReadOnly],
            safety_level: ToolSafetyLevel::Safe,
            mutates_state: false,
            requires_confirmation: false,
        };
        Self::success_with_metadata(name, description, result, metadata)
    }

    pub fn success_with_metadata(
        name: impl Into<String>,
        description: impl Into<String>,
        result: ToolResult,
        metadata: ToolMetadata,
    ) -> (Self, RecordedToolCalls) {
        let name = name.into();
        let description = description.into();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let handle = RecordedToolCalls {
            calls: calls.clone(),
        };
        let tool = Self {
            metadata,
            name,
            description,
            result,
            calls,
        };
        (tool, handle)
    }
}

#[async_trait]
impl Tool for RecordingTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: serde_json::json!({ "type": "object" }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        self.metadata.clone()
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        self.calls.lock().unwrap().push(args);
        Ok(self.result.clone())
    }
}

pub struct AgentHarness {
    agent: Agent,
    channel_size: usize,
}

pub struct AgentHarnessRun {
    pub result: Result<String, AgentError>,
    pub events: Vec<AgentEvent>,
}

#[derive(Clone)]
pub struct RecordedToolAuditEvents {
    events: Arc<Mutex<Vec<AgentToolAuditEvent>>>,
}

impl RecordedToolAuditEvents {
    pub fn entries(&self) -> Vec<AgentToolAuditEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl AgentHarnessRun {
    pub fn event_names(&self) -> Vec<&'static str> {
        self.events
            .iter()
            .map(|event| match event {
                AgentEvent::SubAgentStarted { .. } => "subagent_started",
                AgentEvent::SubAgentFinished { .. } => "subagent_finished",
                AgentEvent::SubAgentFailed { .. } => "subagent_failed",
                AgentEvent::Thinking { .. } => "thinking",
                AgentEvent::OperationPreparing { .. } => "operation_preparing",
                AgentEvent::OperationProgress { .. } => "operation_progress",
                AgentEvent::OperationStarted { .. } => "operation_started",
                AgentEvent::OperationOutput { .. } => "operation_output",
                AgentEvent::OperationFinished { .. } => "operation_finished",
                AgentEvent::OperationFailed { .. } => "operation_failed",
                AgentEvent::ToolCall { .. } => "tool_call",
                AgentEvent::ToolResult { .. } => "tool_result",
                AgentEvent::ToolVisibilityDecision { .. } => "tool_visibility_decision",
                AgentEvent::ModelToolParseDiagnostic { .. } => "model_tool_parse_diagnostic",
                AgentEvent::UnknownToolRequested { .. } => "unknown_tool_requested",
                AgentEvent::ToolNormalizationApplied { .. } => "tool_normalization_applied",
                AgentEvent::RunEvent { event } => match event {
                    crate::agent::AgentRunEvent::Started { .. } => "run_started",
                    crate::agent::AgentRunEvent::Iteration { .. } => "run_iteration",
                    crate::agent::AgentRunEvent::ToolResult { .. } => "run_tool_result",
                    crate::agent::AgentRunEvent::GuidanceMerged { .. } => "run_guidance_merged",
                    crate::agent::AgentRunEvent::GuidanceQueued { .. } => "run_guidance_queued",
                    crate::agent::AgentRunEvent::Finished { .. } => "run_finished",
                    crate::agent::AgentRunEvent::Blocked { .. } => "run_blocked",
                    crate::agent::AgentRunEvent::Paused { .. } => "run_paused",
                    crate::agent::AgentRunEvent::Resumed { .. } => "run_resumed",
                    crate::agent::AgentRunEvent::Cancelled { .. } => "run_cancelled",
                    crate::agent::AgentRunEvent::Failed { .. } => "run_failed",
                },
                AgentEvent::ResponseStarted { .. } => "response_started",
                AgentEvent::ResponseDelta { .. } => "response_delta",
                AgentEvent::ResponseCompleted { .. } => "response_completed",
                AgentEvent::ResponseFallbackStarted { .. } => "response_fallback_started",
                AgentEvent::Response { .. } => "response",
                AgentEvent::FinalAudit { .. } => "final_audit",
            })
            .collect()
    }

    pub fn failed_event(&self) -> Option<(&str, bool)> {
        self.events.iter().find_map(|event| {
            if let AgentEvent::RunEvent {
                event:
                    crate::agent::AgentRunEvent::Failed {
                        error, retryable, ..
                    },
            } = event
            {
                Some((error.as_str(), *retryable))
            } else {
                None
            }
        })
    }
}

impl AgentHarness {
    pub fn new(llm_client: Box<dyn LLMClient>, tool_registry: ToolRegistry) -> Self {
        Self {
            agent: Agent::new(llm_client, tool_registry),
            // Keep the default large enough for event-order assertions; use
            // with_channel_size in tests that intentionally exercise drops.
            channel_size: 1024,
        }
    }

    pub fn with_runtime_config(mut self, runtime_config: AgentRuntimeConfig) -> Self {
        self.agent = self.agent.with_runtime_config(runtime_config);
        self
    }

    pub fn with_tools_enabled(mut self, tools_enabled: bool) -> Self {
        self.agent = self.agent.with_tools_enabled(tools_enabled);
        self
    }

    pub fn with_tool_access_policy(mut self, tool_access_policy: ToolAccessPolicy) -> Self {
        self.agent = self.agent.with_tool_access_policy(tool_access_policy);
        self
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.agent = self.agent.with_run_id(run_id.into());
        self
    }

    pub fn with_guidance_queues(
        mut self,
        queues: Arc<AsyncMutex<HashMap<String, Vec<AgentGuidanceMessage>>>>,
    ) -> Self {
        self.agent = self.agent.with_guidance_queues(queues);
        self
    }

    pub fn with_tool_audit_recorder(mut self) -> (Self, RecordedToolAuditEvents) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = RecordedToolAuditEvents {
            events: events.clone(),
        };
        self.agent = self.agent.with_tool_audit_sink(move |event| {
            events.lock().unwrap().push(event);
        });
        (self, handle)
    }

    pub fn with_channel_size(mut self, channel_size: usize) -> Self {
        self.channel_size = channel_size;
        self
    }

    pub async fn run(self, user_input: impl Into<String>) -> AgentHarnessRun {
        self.run_with_history(user_input, Vec::new()).await
    }

    pub async fn run_with_history(
        mut self,
        user_input: impl Into<String>,
        history: Vec<Message>,
    ) -> AgentHarnessRun {
        let (tx, mut rx) = mpsc::channel(self.channel_size);
        let result = self
            .agent
            .chat_with_history(user_input.into(), history, tx)
            .await;
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        AgentHarnessRun { result, events }
    }
}
