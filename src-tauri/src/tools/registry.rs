use crate::agent::{AgentError, AgentEvent, ToolCall, ToolResult, ToolSchema};
use crate::tools::policy::{AgentPermissionMode, PolicyDecision, PolicyEngine, SubAgentRole};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCapability {
    ReadOnly,
    LocalData,
    Network,
    Filesystem,
    Memory,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolSafetyLevel {
    Safe,
    Sensitive,
    Destructive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolAccessPolicy {
    FullAccess,
    Default,
    Plan,
    DenyAll,
}

impl ToolAccessPolicy {
    pub fn from_permission_mode(mode: AgentPermissionMode) -> Self {
        match mode {
            AgentPermissionMode::Plan => ToolAccessPolicy::Plan,
            AgentPermissionMode::Default => ToolAccessPolicy::Default,
            AgentPermissionMode::FullAccess => ToolAccessPolicy::FullAccess,
        }
    }

    pub fn permission_mode(&self) -> Option<AgentPermissionMode> {
        match self {
            ToolAccessPolicy::Plan => Some(AgentPermissionMode::Plan),
            ToolAccessPolicy::Default => Some(AgentPermissionMode::Default),
            ToolAccessPolicy::FullAccess => Some(AgentPermissionMode::FullAccess),
            ToolAccessPolicy::DenyAll => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ToolAccessPolicy::FullAccess => "full_access",
            ToolAccessPolicy::Default => "default",
            ToolAccessPolicy::Plan => "plan",
            ToolAccessPolicy::DenyAll => "deny_all",
        }
    }

    pub fn advertises_tools(&self) -> bool {
        !matches!(self, ToolAccessPolicy::DenyAll)
    }

    pub fn allows_metadata(&self, metadata: &ToolMetadata) -> bool {
        self.metadata_decision(metadata)
            .is_visible_without_runtime_approval()
    }

    pub fn metadata_decision(&self, metadata: &ToolMetadata) -> PolicyDecision {
        match self.permission_mode() {
            Some(mode) => PolicyEngine::new(mode).evaluate_tool_visibility(metadata),
            None => PolicyDecision::Deny {
                reason: "当前模式禁止调用工具。".to_string(),
            },
        }
    }

    pub fn execution_decision(&self, metadata: &ToolMetadata) -> PolicyDecision {
        match self.permission_mode() {
            Some(mode) => PolicyEngine::new(mode).evaluate_tool_execution(metadata),
            None => PolicyDecision::Deny {
                reason: "当前模式禁止调用工具。".to_string(),
            },
        }
    }

    pub fn blocked_summary(&self, tool_name: &str) -> String {
        match self {
            ToolAccessPolicy::FullAccess => {
                format!("工具 {tool_name} 被完全访问模式的高风险安全边界拦截。")
            }
            ToolAccessPolicy::Default => {
                format!("工具 {tool_name} 被默认模式拦截：命令或高风险动作需要确认。")
            }
            ToolAccessPolicy::Plan => {
                format!("工具 {tool_name} 被计划模式拦截：计划模式只读取和整理方案。")
            }
            ToolAccessPolicy::DenyAll => format!("工具 {tool_name} 被当前模式禁止调用。"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub name: String,
    pub description: String,
    #[serde(rename = "labelZh")]
    pub label_zh: String,
    #[serde(rename = "descriptionZh")]
    pub description_zh: String,
    #[serde(rename = "capabilityLabelsZh")]
    pub capability_labels_zh: Vec<String>,
    #[serde(rename = "safetyLabelZh")]
    pub safety_label_zh: String,
    pub capabilities: Vec<ToolCapability>,
    pub safety_level: ToolSafetyLevel,
    pub mutates_state: bool,
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolMetadataIssue {
    pub tool_name: String,
    pub field: String,
    pub message: String,
}

impl ToolMetadata {
    pub fn safe_readonly(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            label_zh: "只读工具".to_string(),
            description_zh: "读取本地允许范围内的信息，不修改数据。".to_string(),
            capability_labels_zh: vec!["只读".to_string()],
            safety_label_zh: "安全".to_string(),
            capabilities: vec![ToolCapability::ReadOnly],
            safety_level: ToolSafetyLevel::Safe,
            mutates_state: false,
            requires_confirmation: false,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> ToolSchema;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata::safe_readonly(self.name(), self.description())
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError>;

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _context: ToolExecutionContext,
    ) -> Result<ToolResult, AgentError> {
        self.execute(args).await
    }
}

#[derive(Clone)]
pub struct ToolExecutionContext {
    pub operation_id: String,
    pub event_tx: Option<Sender<AgentEvent>>,
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn list_schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
    }

    pub fn list_schemas_for_policy(&self, policy: &ToolAccessPolicy) -> Vec<ToolSchema> {
        self.tools
            .values()
            .filter(|tool| policy.allows_metadata(&tool.metadata()))
            .map(|tool| tool.schema())
            .collect()
    }

    pub fn list_schemas_for_policy_and_allowlist(
        &self,
        policy: &ToolAccessPolicy,
        allowlist: Option<&BTreeSet<String>>,
        subagent_role: Option<SubAgentRole>,
    ) -> Vec<ToolSchema> {
        self.tools
            .values()
            .filter(|tool| {
                let metadata = tool.metadata();
                // P3-2: a subagent role tightens visibility on top of the mode
                // policy, so e.g. a reviewer never even sees write tools.
                let decision = match subagent_role {
                    Some(role) => role.restrict(policy.metadata_decision(&metadata), &metadata),
                    None => policy.metadata_decision(&metadata),
                };
                decision.is_visible_without_runtime_approval()
                    && allowlist
                        .map(|allowed| allowed.contains(tool.name()))
                        .unwrap_or(true)
            })
            .map(|tool| tool.schema())
            .collect()
    }

    pub fn list_metadata(&self) -> Vec<ToolMetadata> {
        self.tools.values().map(|t| t.metadata()).collect()
    }

    pub fn list_metadata_for_policy(&self, policy: &ToolAccessPolicy) -> Vec<ToolMetadata> {
        self.tools
            .values()
            .map(|tool| tool.metadata())
            .filter(|metadata| policy.allows_metadata(metadata))
            .collect()
    }

    pub fn list_metadata_for_policy_and_allowlist(
        &self,
        policy: &ToolAccessPolicy,
        allowlist: Option<&BTreeSet<String>>,
    ) -> Vec<ToolMetadata> {
        self.tools
            .values()
            .filter(|tool| {
                policy.allows_metadata(&tool.metadata())
                    && allowlist
                        .map(|allowed| allowed.contains(tool.name()))
                        .unwrap_or(true)
            })
            .map(|tool| tool.metadata())
            .collect()
    }

    pub fn metadata_for(&self, name: &str) -> Option<ToolMetadata> {
        self.tools.get(name).map(|tool| tool.metadata())
    }

    pub fn metadata_issues(&self) -> Vec<ToolMetadataIssue> {
        let mut issues = Vec::new();
        for tool in self.tools.values() {
            let tool_name = tool.name().to_string();
            let schema = tool.schema();
            let metadata = tool.metadata();

            if schema.name != tool_name {
                issues.push(metadata_issue(
                    &tool_name,
                    "schema.name",
                    format!(
                        "Schema name '{}' does not match registered tool name '{}'.",
                        schema.name, tool_name
                    ),
                ));
            }
            if metadata.name != tool_name {
                issues.push(metadata_issue(
                    &tool_name,
                    "metadata.name",
                    format!(
                        "Metadata name '{}' does not match registered tool name '{}'.",
                        metadata.name, tool_name
                    ),
                ));
            }
            if schema.description.trim().is_empty() {
                issues.push(metadata_issue(
                    &tool_name,
                    "schema.description",
                    "Schema description must not be empty.",
                ));
            }
            if metadata.description.trim().is_empty() {
                issues.push(metadata_issue(
                    &tool_name,
                    "metadata.description",
                    "Metadata description must not be empty.",
                ));
            }
            if metadata.label_zh.trim().is_empty() {
                issues.push(metadata_issue(
                    &tool_name,
                    "metadata.label_zh",
                    "Chinese metadata label must not be empty.",
                ));
            }
            if metadata.description_zh.trim().is_empty() {
                issues.push(metadata_issue(
                    &tool_name,
                    "metadata.description_zh",
                    "Chinese metadata description must not be empty.",
                ));
            }
            if metadata.safety_label_zh.trim().is_empty() {
                issues.push(metadata_issue(
                    &tool_name,
                    "metadata.safety_label_zh",
                    "Chinese safety label must not be empty.",
                ));
            }
            if metadata.capability_labels_zh.is_empty()
                || metadata
                    .capability_labels_zh
                    .iter()
                    .any(|label| label.trim().is_empty())
            {
                issues.push(metadata_issue(
                    &tool_name,
                    "metadata.capability_labels_zh",
                    "Chinese capability labels must not be empty.",
                ));
            }
            if metadata.capabilities.is_empty() {
                issues.push(metadata_issue(
                    &tool_name,
                    "metadata.capabilities",
                    "Tool must declare at least one capability.",
                ));
            }
            if metadata.safety_level == ToolSafetyLevel::Safe && metadata.mutates_state {
                issues.push(metadata_issue(
                    &tool_name,
                    "metadata.mutates_state",
                    "Safe tools must not mutate state.",
                ));
            }
            if metadata.safety_level == ToolSafetyLevel::Safe && metadata.requires_confirmation {
                issues.push(metadata_issue(
                    &tool_name,
                    "metadata.requires_confirmation",
                    "Safe tools must not require confirmation.",
                ));
            }
            if metadata.capabilities.contains(&ToolCapability::ReadOnly) && metadata.mutates_state {
                issues.push(metadata_issue(
                    &tool_name,
                    "metadata.capabilities",
                    "ReadOnly tools must not mutate state.",
                ));
            }
            if metadata.safety_level == ToolSafetyLevel::Destructive && !metadata.mutates_state {
                issues.push(metadata_issue(
                    &tool_name,
                    "metadata.mutates_state",
                    "Destructive tools must mutate state.",
                ));
            }
            if metadata.safety_level == ToolSafetyLevel::Destructive
                && !metadata.requires_confirmation
            {
                issues.push(metadata_issue(
                    &tool_name,
                    "metadata.requires_confirmation",
                    "Destructive tools must require confirmation.",
                ));
            }
        }
        issues
    }

    pub async fn execute(&self, tool_call: &ToolCall) -> Result<ToolResult, AgentError> {
        self.execute_with_context(
            tool_call,
            ToolExecutionContext {
                operation_id: tool_call.id.clone(),
                event_tx: None,
            },
        )
        .await
    }

    pub async fn execute_with_context(
        &self,
        tool_call: &ToolCall,
        context: ToolExecutionContext,
    ) -> Result<ToolResult, AgentError> {
        let tool = self
            .tools
            .get(&tool_call.name)
            .ok_or_else(|| AgentError::Tool(format!("未找到工具：{}", tool_call.name)))?;

        tool.execute_with_context(tool_call.arguments.clone(), context)
            .await
    }
}

fn metadata_issue(
    tool_name: &str,
    field: impl Into<String>,
    message: impl Into<String>,
) -> ToolMetadataIssue {
    ToolMetadataIssue {
        tool_name: tool_name.to_string(),
        field: field.into(),
        message: message.into(),
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MetadataTool;

    #[async_trait]
    impl Tool for MetadataTool {
        fn name(&self) -> &str {
            "metadata_tool"
        }

        fn description(&self) -> &str {
            "metadata test tool"
        }

        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: self.name().to_string(),
                description: self.description().to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            }
        }

        fn metadata(&self) -> ToolMetadata {
            ToolMetadata {
                name: self.name().to_string(),
                description: self.description().to_string(),
                label_zh: "元数据测试工具".to_string(),
                description_zh: "用于测试工具元数据。".to_string(),
                capability_labels_zh: vec!["记忆".to_string()],
                safety_label_zh: "敏感".to_string(),
                capabilities: vec![ToolCapability::Memory],
                safety_level: ToolSafetyLevel::Sensitive,
                mutates_state: true,
                requires_confirmation: false,
            }
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult, AgentError> {
            Ok(ToolResult::success("ok", serde_json::json!({})))
        }
    }

    struct BrokenTool {
        name: String,
        schema_name: String,
        metadata: ToolMetadata,
    }

    #[async_trait]
    impl Tool for BrokenTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "broken test tool"
        }

        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: self.schema_name.clone(),
                description: String::new(),
                parameters: serde_json::json!({ "type": "object" }),
            }
        }

        fn metadata(&self) -> ToolMetadata {
            self.metadata.clone()
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult, AgentError> {
            Ok(ToolResult::success("ok", serde_json::json!({})))
        }
    }

    #[test]
    fn registry_lists_tool_metadata() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MetadataTool));
        let metadata = registry.list_metadata();
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].capabilities, vec![ToolCapability::Memory]);
        assert!(metadata[0].mutates_state);
    }

    #[test]
    fn access_policy_filters_schemas_from_metadata() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MetadataTool));

        assert_eq!(
            registry
                .list_schemas_for_policy(&ToolAccessPolicy::FullAccess)
                .len(),
            1
        );
        assert_eq!(
            registry
                .list_schemas_for_policy(&ToolAccessPolicy::Default)
                .len(),
            1
        );
        assert!(registry
            .list_schemas_for_policy(&ToolAccessPolicy::Plan)
            .is_empty());
        assert!(registry
            .list_schemas_for_policy(&ToolAccessPolicy::DenyAll)
            .is_empty());
    }

    #[test]
    fn subagent_role_hides_write_tools_from_read_only_role() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MetadataTool)); // mutates_state = true (a write tool)

        // Full-access session would advertise the write tool...
        assert_eq!(
            registry
                .list_schemas_for_policy_and_allowlist(&ToolAccessPolicy::FullAccess, None, None)
                .len(),
            1
        );
        // ...but a read-only reviewer subagent never sees it.
        assert!(registry
            .list_schemas_for_policy_and_allowlist(
                &ToolAccessPolicy::FullAccess,
                None,
                Some(SubAgentRole::Reviewer),
            )
            .is_empty());
        // An executor subagent inherits the session and still sees it.
        assert_eq!(
            registry
                .list_schemas_for_policy_and_allowlist(
                    &ToolAccessPolicy::FullAccess,
                    None,
                    Some(SubAgentRole::Executor),
                )
                .len(),
            1
        );
    }

    #[test]
    fn access_policy_filters_metadata_from_metadata() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MetadataTool));

        assert_eq!(
            registry
                .list_metadata_for_policy(&ToolAccessPolicy::FullAccess)
                .len(),
            1
        );
        assert_eq!(
            registry
                .list_metadata_for_policy(&ToolAccessPolicy::Default)
                .len(),
            1
        );
        assert!(registry
            .list_metadata_for_policy(&ToolAccessPolicy::Plan)
            .is_empty());
        assert!(registry
            .list_metadata_for_policy(&ToolAccessPolicy::DenyAll)
            .is_empty());
    }

    #[test]
    fn registry_reports_metadata_consistency_issues() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(BrokenTool {
            name: "broken_tool".to_string(),
            schema_name: "wrong_schema".to_string(),
            metadata: ToolMetadata {
                name: "wrong_metadata".to_string(),
                description: String::new(),
                label_zh: String::new(),
                description_zh: String::new(),
                capability_labels_zh: vec![String::new()],
                safety_label_zh: String::new(),
                capabilities: vec![ToolCapability::ReadOnly],
                safety_level: ToolSafetyLevel::Safe,
                mutates_state: true,
                requires_confirmation: true,
            },
        }));

        let issues = registry.metadata_issues();
        assert!(issues.iter().any(|issue| issue.field == "schema.name"));
        assert!(issues.iter().any(|issue| issue.field == "metadata.name"));
        assert!(issues
            .iter()
            .any(|issue| issue.field == "metadata.mutates_state"));
        assert!(issues
            .iter()
            .any(|issue| issue.field == "metadata.requires_confirmation"));
        assert!(issues
            .iter()
            .any(|issue| issue.field == "metadata.capabilities"));
    }
}
