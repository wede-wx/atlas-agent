//! Step 1（契约结构化通道）：`atlas_freeze_goal_contract` 工具。
//!
//! 背景：此前 Goal Contract 只能从 assistant 自由文本里按标题刮取
//! （REVIEW_FINDINGS 第 7 条点名的脆弱点）——模型换个格式契约就丢了，
//! harness 整个 session 不设防。本工具给契约一个**结构化提交通道**：
//! 模型在 Gate Mode 收到用户确认后调用本工具，参数即结构化契约。
//!
//! 职责边界（刻意做薄）：
//! - 工具只做「解析 + 校验 + 冻结 + 回传」，**不直接安装进 harness**——
//!   harness 在 Agent 内存里，工具触不到。安装由 core.rs 在消费本工具的
//!   Success 结果时完成（同处触发 A6 的持久化 sink）。
//! - 解析复用 `GoalContract::from_structured`，与文本通道语义严格一致；
//!   文本刮取保留为后备通道，模型不调工具时旧路径照常生效。

use async_trait::async_trait;

use crate::agent::atlas_harness::GoalContract;
use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};

/// 工具名常量：core.rs 消费结果、policy.rs 全模式放行都以它为锚。
pub const ATLAS_FREEZE_GOAL_CONTRACT_TOOL: &str = "atlas_freeze_goal_contract";

pub struct FreezeGoalContractTool;

#[async_trait]
impl Tool for FreezeGoalContractTool {
    fn name(&self) -> &str {
        ATLAS_FREEZE_GOAL_CONTRACT_TOOL
    }

    fn description(&self) -> &str {
        "Submit the confirmed Atlas Goal Contract as structured data and freeze it as the execution baseline."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Freeze the Atlas Goal Contract through a structured channel. \
                Call this ONCE, only after the user has confirmed the Goal Contract in Gate Mode. \
                Submit the contract as structured fields instead of printing a text block — \
                this is the reliable channel; the text block is only a fallback. \
                The contract becomes the frozen execution baseline: hard items can only be \
                changed afterwards through a Deviation Notice confirmed by the user."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "required": ["goal"],
                "properties": {
                    "goal": {
                        "type": "string",
                        "description": "One-sentence goal, in the user's own framing."
                    },
                    "must_do": {
                        "type": "array",
                        "description": "Things that MUST be done. Items: {id?, text, hard?=true, source_quote?, verify?}.",
                        "items": {
                            "type": "object",
                            "required": ["text"],
                            "properties": {
                                "id": { "type": "string", "description": "Stable id like M1; auto-assigned if omitted." },
                                "text": { "type": "string" },
                                "hard": { "type": "boolean", "description": "Defaults to true. Soft items can be traded off after disclosure." },
                                "source_quote": { "type": "string", "description": "Verbatim user words anchoring this item." },
                                "verify": { "type": "string", "description": "How to verify: a command, test, or observable check." }
                            }
                        }
                    },
                    "must_not_do": {
                        "type": "array",
                        "description": "Things that MUST NOT be done without disclosure. Same item shape as must_do.",
                        "items": { "type": "object", "required": ["text"], "properties": {
                            "id": { "type": "string" }, "text": { "type": "string" },
                            "hard": { "type": "boolean" }, "source_quote": { "type": "string" },
                            "verify": { "type": "string" }
                        } }
                    },
                    "preserve": {
                        "type": "array",
                        "description": "Existing things that must be preserved. Items: {id?, text, kind, path_glob?}.",
                        "items": {
                            "type": "object",
                            "required": ["text", "kind"],
                            "properties": {
                                "id": { "type": "string" },
                                "text": { "type": "string" },
                                "kind": {
                                    "type": "string",
                                    "enum": ["behavior", "layout_structure", "api_contract", "scope", "data", "file"]
                                },
                                "path_glob": { "type": "string", "description": "Path glob like src/ui/** for file/layout_structure kinds." }
                            }
                        }
                    },
                    "constraints": {
                        "type": "array",
                        "description": "Constraints. Same item shape as must_do.",
                        "items": { "type": "object", "required": ["text"], "properties": {
                            "id": { "type": "string" }, "text": { "type": "string" },
                            "hard": { "type": "boolean" }, "source_quote": { "type": "string" },
                            "verify": { "type": "string" }
                        } }
                    },
                    "acceptance_criteria": {
                        "type": "array",
                        "description": "Completion checks. Same item shape as must_do.",
                        "items": { "type": "object", "required": ["text"], "properties": {
                            "id": { "type": "string" }, "text": { "type": "string" },
                            "hard": { "type": "boolean" }, "source_quote": { "type": "string" },
                            "verify": { "type": "string" }
                        } }
                    },
                    "in_scope": { "type": "array", "items": { "type": "string" } },
                    "out_of_scope": { "type": "array", "items": { "type": "string" } },
                    "reference": {
                        "type": "object",
                        "description": "When a reference image/design exists: layout structure must match; layout wins over style.",
                        "properties": {
                            "layout_structure": { "type": "array", "items": { "type": "string" } },
                            "style": { "type": "array", "items": { "type": "string" } }
                        }
                    }
                }
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "冻结目标契约".to_string(),
            description_zh: "把用户确认后的目标契约结构化提交并冻结为执行基线。".to_string(),
            capability_labels_zh: vec!["规划状态".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::Memory],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let parse = GoalContract::from_structured(&args);
        if !parse.is_usable() {
            return Ok(ToolResult::recoverable_error(
                format!(
                    "目标契约不可用：{}",
                    if parse.diagnostics.is_empty() {
                        "缺少 goal".to_string()
                    } else {
                        parse.diagnostics.join("；")
                    }
                ),
                vec![
                    "补全 goal（一句话目标）后重新提交".to_string(),
                    "不要在契约未冻结的情况下开始执行写动作".to_string(),
                ],
            ));
        }

        let mut contract = parse.contract;
        contract.freeze();
        let summary = format!(
            "目标契约已冻结：{}（must_do {} 项 / must_not_do {} 项 / preserve {} 项）。",
            contract.goal,
            contract.must_do.len(),
            contract.must_not_do.len(),
            contract.preserve.len(),
        );
        Ok(ToolResult::success(
            summary,
            serde_json::json!({
                "contract": contract,
                "diagnostics": parse.diagnostics,
                "frozen": true,
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn freezes_structured_contract_with_default_guards() {
        let tool = FreezeGoalContractTool;
        let result = tool
            .execute(serde_json::json!({
                "goal": "ship X",
                "must_do": [ { "id": "M1", "text": "implement X" } ],
                "preserve": [ { "text": "keep src/ui/**", "kind": "layout_structure", "path_glob": "src/ui/**" } ]
            }))
            .await
            .unwrap();

        assert!(matches!(
            result.status,
            crate::agent::ToolResultStatus::Success
        ));
        let contract: GoalContract =
            serde_json::from_value(result.data.get("contract").unwrap().clone()).unwrap();
        assert!(contract.frozen, "tool returns the contract already frozen");
        assert!(contract.has_hard_constraints());
        assert!(
            contract.must_not_do.iter().any(|i| i.id == "N-hide"),
            "default betrayal guards must be injected on the structured channel too"
        );
    }

    #[tokio::test]
    async fn rejects_contract_without_goal_as_recoverable() {
        let tool = FreezeGoalContractTool;
        let result = tool
            .execute(serde_json::json!({ "must_do": [ { "text": "x" } ] }))
            .await
            .unwrap();
        assert!(matches!(
            result.status,
            crate::agent::ToolResultStatus::Error
        ));
        assert!(
            result.recoverable,
            "model should restate, not abort the run"
        );
        assert!(result.summary.contains("目标契约不可用"));
    }

    #[test]
    fn metadata_passes_registry_consistency_rules() {
        // Sensitive + mutates_state 组合符合 registry 元数据校验
        // （Safe + mutates_state / ReadOnly + mutates_state 会被标记）。
        let metadata = FreezeGoalContractTool.metadata();
        assert_eq!(metadata.safety_level, ToolSafetyLevel::Sensitive);
        assert!(metadata.mutates_state);
        assert!(!metadata.capabilities.contains(&ToolCapability::ReadOnly));
    }
}
