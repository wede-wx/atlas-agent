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
pub fn proposed_action_from_tool_call(tool_name: &str, args: &serde_json::Value) -> ProposedAction {
    let pick = |keys: &[&str]| -> Option<String> {
        for k in keys {
            if let Some(v) = args.get(*k).and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
        None
    };
    ProposedAction {
        kind_raw: tool_name.to_string(),
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
    fn extracts_contract_block_zh() {
        let text = "好的，下面是契约：\n\nAtlas 目标合同\n目标：\n- 实现 X\n\nATLAS_STOP: 等待确认。\n之后的闲聊";
        let block = extract_contract_block(text).unwrap();
        assert!(block.contains("Atlas 目标合同"));
        assert!(block.contains("实现 X"));
        assert!(!block.contains("闲聊"));
    }

    #[test]
    fn no_contract_returns_none() {
        assert!(extract_contract_block("just a normal reply").is_none());
    }
}
