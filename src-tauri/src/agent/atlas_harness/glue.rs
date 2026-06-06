//! Atlas Harness — glue.rs（宿主 ↔ harness 的适配层）。
//!
//! 这是“接线”阶段唯一可以**完全确定写对**的新代码:它只依赖 harness 自己的类型 +
//! serde_json,不碰你的核心结构。它做两件事:
//!   1. 把一次工具调用(name + JSON 参数)转成 ContractGate 能吃的 `ProposedAction`。
//!   2. 从 assistant 文本里抽出 Atlas Skill 打印的 Goal Contract 文本块,供 harness 解析冻结。
//!
//! 真正“插哪一行”的活(在 dispatch 里调用 gate_action、在 session 里持有 AtlasHarness)
//! 见执行计划——那部分必须贴合你真实的 core.rs 调用点,由 Codex 适配。

use super::contract_gate::ProposedAction;

/// 从工具调用构造 ProposedAction。
///
/// 不依赖你工具的确切命名:对 target/command/content 各试一组常见参数键,
/// 工具种类由 `ProposedAction::kind()` 的子串匹配(contains "edit"/"write"/"bash"...)判定。
/// 因此即使你的工具叫 `str_replace` / `apply_patch` / `run_command` 也能正确归类。
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
    }
}

/// 从 assistant 文本里抽出 Atlas Goal Contract 文本块(若存在)。
///
/// 用法:在你捕获 assistant 输出的地方(message 流里),对其文本调用此函数;
/// 拿到块后交给 `AtlasHarness::install_contract_from_skill` 解析并冻结。
/// 识别本地化标题(中英),从标题截到 `ATLAS_STOP`(契约块的结束标记)或文本结尾。
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
    fn maps_edit_tool_call() {
        let a = proposed_action_from_tool_call(
            "str_replace",
            &json!({"path": "src/ui/App.tsx", "new_str": "fn x(){ todo!() }"}),
        );
        assert_eq!(a.target_path.as_deref(), Some("src/ui/App.tsx"));
        assert_eq!(a.content_or_diff.as_deref(), Some("fn x(){ todo!() }"));
    }

    #[test]
    fn maps_command_tool_call() {
        let a = proposed_action_from_tool_call("bash", &json!({"command": "pytest --no-verify"}));
        assert_eq!(a.command.as_deref(), Some("pytest --no-verify"));
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
