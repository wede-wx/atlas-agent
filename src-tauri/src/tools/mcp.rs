use async_trait::async_trait;

use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::mcp::invoke_mcp_tool;
use crate::storage::LocalDb;
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};

pub struct InvokeMcpTool {
    db: LocalDb,
}

impl InvokeMcpTool {
    pub fn new(db: LocalDb) -> Self {
        Self { db }
    }
}

#[async_trait]
impl Tool for InvokeMcpTool {
    fn name(&self) -> &str {
        "invoke_mcp_tool"
    }

    fn description(&self) -> &str {
        "Invoke an enabled MCP tool through Atlas's MCP permission and audit boundary."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Call an MCP tool. Sensitive MCP tools require confirmed=true after user confirmation.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "serverId": { "type": "string", "description": "MCP server id" },
                    "toolName": { "type": "string", "description": "MCP tool name" },
                    "arguments": { "type": "object", "description": "Tool arguments" },
                    "confirmed": { "type": "boolean", "description": "Whether the user confirmed this sensitive MCP call" }
                },
                "required": ["serverId", "toolName"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "调用 MCP 工具".to_string(),
            description_zh: "通过 Atlas 的 MCP 权限和审计边界调用已启用的 MCP 工具。".to_string(),
            capability_labels_zh: vec![
                "MCP".to_string(),
                "外部工具".to_string(),
                "需确认".to_string(),
            ],
            capabilities: vec![ToolCapability::Network],
            safety_label_zh: "敏感".to_string(),
            safety_level: ToolSafetyLevel::Sensitive,
            // 修复（中高）：MCP 工具可以产生任意副作用（写文件、发请求、改外部
            // 系统）。mutates_state=false 会让 infer_tool_action 把它分类成
            // Read：只读子代理（Reviewer/Planner）因此能调 MCP 写工具，权限层
            // 对它也完全不设防。按 fail-closed 原则按写处理；真正只读的 MCP
            // 调用多走一次结构比对没有代价（无路径/命令参数时不会触发任何违例）。
            mutates_state: true,
            requires_confirmation: true,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let server_id = args
            .get("serverId")
            .or_else(|| args.get("server_id"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| AgentError::Tool("缺少 serverId 参数。".to_string()))?;
        let tool_name = args
            .get("toolName")
            .or_else(|| args.get("tool_name"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| AgentError::Tool("缺少 toolName 参数。".to_string()))?;
        let arguments = args
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let confirmed = args
            .get("confirmed")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let result = invoke_mcp_tool(&self.db, server_id, tool_name, arguments, confirmed)
            .await
            .map_err(AgentError::Tool)?;
        let data =
            bound_mcp_result_data(&result, crate::tools::output_limit::MAX_TOOL_OUTPUT_CHARS);
        Ok(ToolResult::success(
            format!("MCP 工具已调用：{server_id}/{tool_name}"),
            data,
        ))
    }
}

/// P1-5: an MCP server's output is arbitrary, untrusted, potentially huge external
/// JSON. Bound the `output` field (the dominant payload) with the shared head+tail
/// limiter and attach `truncation` meta, so it can't blow the model context and
/// reads the same shape as every other tool. Pure (no I/O) for direct testing.
fn bound_mcp_result_data(
    result: &crate::mcp::McpInvokeResult,
    max_chars: usize,
) -> serde_json::Value {
    let bounded =
        crate::tools::output_limit::truncate_middle(&result.output.to_string(), max_chars);
    let mut data = serde_json::json!(result);
    if bounded.truncated {
        data["output"] = serde_json::Value::String(bounded.text.clone());
    }
    data["truncation"] = bounded.meta();
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpInvokeResult;

    fn result_with_output(output: serde_json::Value) -> McpInvokeResult {
        McpInvokeResult {
            server_id: "srv".to_string(),
            tool_name: "tool".to_string(),
            status: "ok".to_string(),
            output,
            untrusted_external: true,
            requires_confirmation: false,
            server_trusted: true,
        }
    }

    #[test]
    fn small_mcp_output_keeps_structure_with_meta() {
        let result = result_with_output(serde_json::json!({ "content": "hi" }));
        let data = bound_mcp_result_data(&result, 10_000);
        // Structured output preserved; uniform truncation meta attached, not truncated.
        assert_eq!(data["output"], serde_json::json!({ "content": "hi" }));
        assert_eq!(data["truncation"]["truncated"], serde_json::json!(false));
        assert_eq!(data["serverTrusted"], serde_json::json!(true));
    }

    #[test]
    fn huge_mcp_output_is_bounded_to_preview_string_with_meta() {
        let result = result_with_output(serde_json::json!({ "blob": "x".repeat(50_000) }));
        let data = bound_mcp_result_data(&result, 1_000);
        // Oversized external JSON collapses to a bounded head+tail preview string.
        let output = data["output"].as_str().expect("bounded to string");
        assert!(output.chars().count() <= 1_000, "bounded within cap");
        assert!(output.contains("已省略"), "elision marker present");
        assert_eq!(data["truncation"]["truncated"], serde_json::json!(true));
        assert!(
            data["truncation"]["originalChars"].as_u64().unwrap() > 50_000,
            "meta reports the real original size"
        );
    }
}
