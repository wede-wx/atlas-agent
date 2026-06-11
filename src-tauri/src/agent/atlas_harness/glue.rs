//! Atlas Harness — glue (host ↔ harness adapter).
//!
//! This is the one piece of wiring code we can write with full confidence: it
//! depends only on the harness's own types + `serde_json`, never on the host's
//! core structs. It does two things:
//!   1. Turn a tool call (name + JSON args) into a `ProposedAction` the
//!      ContractGate can consume.
//!   2. Extract the Atlas Goal Contract text block printed by the Skill, so the
//!      harness can parse and freeze it.
//!
//! The actual call sites (holding an `AtlasHarness` per session, calling
//! `gate_action` in dispatch) live in `core.rs`; see
//! `patches/core_rs_atlas_harness.patch`.

use super::contract_gate::ProposedAction;

/// Build a `ProposedAction` from a tool call.
///
/// Does not depend on exact tool names: it tries a set of common argument keys
/// for target / command / content / prior-content. The name only provides a
/// *hint* for `ProposedAction::kind()`; `is_mutating()` additionally infers
/// write-ness from the arguments, so an unrecognized write tool is still gated.
///
/// MCP / plugin invocations (`invoke_mcp_tool`, `invoke_plugin_capability`,
/// registered `plugin_*` tools) wrap their real arguments one level deep
/// (`arguments` / `input`); those are unwrapped here and the whole call is
/// treated as mutating by default (`assume_mutating`), because an external
/// tool's side effects cannot be inferred from its argument shape.
pub fn proposed_action_from_tool_call(tool_name: &str, args: &serde_json::Value) -> ProposedAction {
    // ── 修复（高）：MCP / 插件入口的参数是嵌套的 ──────────────────────────
    // `invoke_mcp_tool`            { serverId, toolName, arguments: {...} }
    // `invoke_plugin_capability` / `plugin_*` { pluginId, capabilityId, input: {...} }
    // 旧实现只在顶层取 path/command/content，对这两类调用全部取到 None，
    // is_mutating() 因此返回 false，ContractGate / ImpactEvidenceGate 静默放行——
    // 这正好击穿了“未知写工具 fail-closed”的承诺。这里先解包一层，再用
    // 内层工具名补充 kind 提示；同时把这类外部调用标记为 assume_mutating，
    // 即使内层参数键不认识也不会被当成只读。
    let mut effective_name = tool_name.to_string();
    let mut effective_args = args;
    let mut external_invocation = false;
    for wrapper_key in ["arguments", "input"] {
        if let Some(inner) = args.get(wrapper_key).filter(|v| v.is_object()) {
            effective_args = inner;
            external_invocation = true;
            let inner_name = args
                .get("toolName")
                .or_else(|| args.get("tool_name"))
                .or_else(|| args.get("capabilityId"))
                .or_else(|| args.get("capability_id"))
                .and_then(|v| v.as_str());
            if let Some(inner_name) = inner_name {
                effective_name = format!("{tool_name}::{inner_name}");
            }
            break;
        }
    }
    let pick = |keys: &[&str]| -> Option<String> {
        for k in keys {
            // 先看解包后的内层参数，再兜底看顶层（两者相同时只查一次）。
            for source in [effective_args, args] {
                if let Some(v) = source.get(*k).and_then(|v| v.as_str()) {
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
                if std::ptr::eq(effective_args, args) {
                    break;
                }
            }
        }
        None
    };
    ProposedAction {
        kind_raw: effective_name,
        assume_mutating: external_invocation,
        target_path: pick(&[
            "path",
            "file_path",
            "filename",
            "file",
            "target",
            "target_path",
            "old_path",
        ]),
        command: pick(&["command", "cmd", "script", "shell_command", "bash", "run"]),
        content_or_diff: pick(&[
            "content",
            "new_str",
            "new_string",
            "text",
            "new_content",
            "patch",
            "diff",
            "replacement",
            "file_text",
        ]),
        // Pre-edit text, so mass-deletion detection works on str_replace edits
        // (whose `new_str` never contains unified-diff `-` lines).
        prior_content: pick(&["old_str", "old_string", "old_content"]),
    }
}

/// Extract the Atlas Goal Contract text block from assistant text, if present.
///
/// Recognizes localized headers (EN/ZH); reads from the header to the
/// `ATLAS_STOP` end marker or end of text.
///
/// NOTE (known limitation, see docs/REVIEW_FINDINGS.md item 7): scraping a
/// sentinel out of free-form prose is brittle to model formatting drift. The
/// recommended longer-term fix is a structured channel (a dedicated tool call
/// or a fenced block with its own parser). Left as-is here because changing it
/// is a design decision, not a bug fix.
pub fn extract_contract_block(assistant_text: &str) -> Option<&str> {
    const HEADERS: &[&str] = &[
        "Atlas Goal Contract",
        "Atlas 目标合同",
        "Atlas 目标契约",
        "Goal Contract",
        "目标合同",
    ];
    let start = HEADERS
        .iter()
        .filter_map(|h| assistant_text.find(h))
        .min()?;
    let rest = &assistant_text[start..];
    let end = rest
        .find("ATLAS_STOP")
        .map(|e| start + e)
        .unwrap_or(assistant_text.len());
    Some(assistant_text[start..end].trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_edit_tool_call_with_prior_content() {
        let a = proposed_action_from_tool_call(
            "str_replace",
            &json!({"path": "src/ui/App.tsx", "old_str": "old body", "new_str": "fn x(){ todo!() }"}),
        );
        assert_eq!(a.target_path.as_deref(), Some("src/ui/App.tsx"));
        assert_eq!(a.content_or_diff.as_deref(), Some("fn x(){ todo!() }"));
        assert_eq!(a.prior_content.as_deref(), Some("old body"));
    }

    #[test]
    fn maps_command_tool_call() {
        let a = proposed_action_from_tool_call("bash", &json!({"command": "pytest --no-verify"}));
        assert_eq!(a.command.as_deref(), Some("pytest --no-verify"));
    }

    #[test]
    fn unknown_write_tool_infers_mutation_from_args() {
        let a = proposed_action_from_tool_call(
            "mcp_fs_apply",
            &json!({"path": "src/x.ts", "content": "export const x = 1"}),
        );
        assert!(a.is_mutating());
    }

    #[test]
    fn mcp_invocation_unwraps_nested_arguments_and_is_gated() {
        // invoke_mcp_tool 的真实参数在 `arguments` 里；旧实现取不到 path/content，
        // 会被当成只读放行。
        let a = proposed_action_from_tool_call(
            "invoke_mcp_tool",
            &json!({
                "serverId": "fs",
                "toolName": "apply_edit",
                "arguments": { "path": "src/ui/App.tsx", "content": "x" }
            }),
        );
        assert_eq!(a.target_path.as_deref(), Some("src/ui/App.tsx"));
        assert_eq!(a.kind_raw, "invoke_mcp_tool::apply_edit");
        assert!(a.is_mutating());
    }

    #[test]
    fn mcp_invocation_with_unknown_arg_keys_is_still_mutating() {
        // 即使内层参数键完全不认识，外部调用也不能被当成只读（fail-closed）。
        let a = proposed_action_from_tool_call(
            "invoke_mcp_tool",
            &json!({
                "serverId": "db",
                "toolName": "execute_sql",
                "arguments": { "statement": "DROP TABLE users" }
            }),
        );
        assert!(a.target_path.is_none());
        assert!(a.is_mutating());
    }

    #[test]
    fn plugin_invocation_unwraps_input() {
        let a = proposed_action_from_tool_call(
            "invoke_plugin_capability",
            &json!({
                "pluginId": "p1",
                "capabilityId": "writer",
                "input": { "path": "src/x.ts", "content": "y" }
            }),
        );
        assert_eq!(a.target_path.as_deref(), Some("src/x.ts"));
        assert_eq!(a.kind_raw, "invoke_plugin_capability::writer");
        assert!(a.is_mutating());
    }

    #[test]
    fn no_contract_returns_none() {
        assert!(extract_contract_block("just a normal reply").is_none());
    }

    #[test]
    fn extracts_contract_block_zh() {
        let text = "好的，下面是契约：\n\nAtlas 目标合同\n目标：\n- 实现 X\n\nATLAS_STOP: 等待确认。\n之后的闲聊";
        let block = extract_contract_block(text).unwrap();
        assert!(block.contains("Atlas 目标合同"));
        assert!(block.contains("实现 X"));
        assert!(!block.contains("闲聊"));
    }
}
