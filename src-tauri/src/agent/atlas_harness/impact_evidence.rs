//! Atlas Harness — ImpactEvidenceGate（禁止“无证据的无影响断言”）。
//!
//! 这是“自裁行为”的直接解法。agent 最常见的偏移形态是：
//! “改这个不影响 X,所以我就改了”——一个**没去查就下的无影响断言**。
//!
//! 规则（机械、不依赖 NLP）：当一次 mutating action 触碰**未在当前 allowed scope 明确列出**
//! 的目标时,要求先有一条针对该目标的 usage-scan 证据(grep 所有引用/相关测试),
//! 记入 ledger 后才放行。把“我觉得没事”强制变成“这是没事的证据”。

use super::contract_gate::ProposedAction;
use super::goal_contract::GoalContract;
use std::collections::HashSet;

/// 记录哪些目标已经有 usage-scan 证据。随 run 维护(可持久化到 storage)。
#[derive(Debug, Default)]
pub struct ImpactLedger {
    scanned: HashSet<String>,
}

impl ImpactLedger {
    pub fn record_scan(&mut self, target: impl Into<String>) {
        self.scanned.insert(normalize(target.into()));
    }
    pub fn has_scan(&self, target: &str) -> bool {
        self.scanned.contains(&normalize(target.to_string()))
    }
}

/// 当一次动作需要先出证据时返回。`suggested_command` 直接可执行。
#[derive(Debug, Clone)]
pub struct EvidenceRequirement {
    pub target: String,
    pub reason: String,
    pub suggested_command: String,
}

/// 是否需要先出影响证据。None = 不需要(scope 内,或已扫过,或非 mutating)。
pub fn requires_evidence(
    action: &ProposedAction,
    contract: &GoalContract,
    ledger: &ImpactLedger,
) -> Option<EvidenceRequirement> {
    let path = action.target_path.as_ref()?;
    // 修复（高）：原先用名字子串 is_mutating_name() 判定，一个名字不含
    // write/edit/delete 的写工具（典型：MCP 写工具）会整体跳过本闸——
    // 与 ContractGate 修过的同一个 fail-open 洞。改用统一的 fail-closed
    // 判定 ProposedAction::is_mutating()（含 assume_mutating）。
    if !action.is_mutating() {
        return None;
    }
    // 在 in_scope 明确列出 → 不要求(契约已为它背书)。
    // 修复（中）：原先 path.contains(entry) 是裸子串，in_scope "src/x" 会
    // 背书 "tests/src/xylophone.rs"（fail-open 方向的过宽）。改为与
    // out_of_scope 同一套边界感知匹配。
    let in_scope = contract
        .scope
        .in_scope
        .iter()
        .any(|s| super::path_match::path_under_entry(s, path))
        || contract
            .must_do
            .iter()
            .any(|m| m.text.contains(path.as_str()));
    if in_scope || ledger.has_scan(path) {
        return None;
    }
    let symbol = file_stem(path);
    Some(EvidenceRequirement {
        target: path.clone(),
        reason: format!(
            "`{path}` 不在当前 allowed scope 内。动它之前必须先证明影响面,不能凭判断断言“安全/隔离”。"
        ),
        suggested_command: format!("grep -rn \"{symbol}\" --include=*.rs --include=*.ts ."),
    })
}

fn file_stem(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(path)
        .to_string()
}

fn normalize(s: String) -> String {
    // 与 ContractGate 同源的词法归一化：`./`、`..`、反斜杠、绝对前缀。
    // 旧实现只剥 "./"，`src\x.rs` 扫过之后再用 `src/x.rs` 来改仍然要求重扫。
    let n = super::path_match::normalize_rel_path(s.trim());
    n.trim_start_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::atlas_harness::goal_contract::GoalContract;

    fn c() -> GoalContract {
        GoalContract::parse_from_skill_block("Goal:\n- x\nIn Scope:\n- src/feature\n").contract
    }

    #[test]
    fn out_of_scope_edit_requires_scan_then_clears() {
        let a = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/shared/enums.rs".into()),
            ..Default::default()
        };
        let mut ledger = ImpactLedger::default();
        let req = requires_evidence(&a, &c(), &ledger);
        assert!(req.is_some());
        assert!(req.unwrap().suggested_command.contains("enums"));
        ledger.record_scan("src/shared/enums.rs");
        assert!(requires_evidence(&a, &c(), &ledger).is_none());
    }

    #[test]
    fn unknown_named_write_tool_is_gated_for_evidence() {
        // 名字不含 write/edit/...，但参数 writeful → 必须出证据。
        let a = ProposedAction {
            kind_raw: "mcp_fs_apply".into(),
            target_path: Some("src/shared/util.rs".into()),
            content_or_diff: Some("x".into()),
            ..Default::default()
        };
        assert!(requires_evidence(&a, &c(), &ImpactLedger::default()).is_some());
    }

    #[test]
    fn in_scope_substring_lookalike_is_not_endorsed() {
        // "src/feature" 不能背书 "tests/src/featurex.rs"。
        let a = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/featurex/x.rs".into()),
            content_or_diff: Some("x".into()),
            ..Default::default()
        };
        assert!(requires_evidence(&a, &c(), &ImpactLedger::default()).is_some());
    }

    #[test]
    fn scan_record_survives_path_obfuscation() {
        let a = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/shared/enums.rs".into()),
            ..Default::default()
        };
        let mut ledger = ImpactLedger::default();
        ledger.record_scan("./src/shared/enums.rs");
        assert!(requires_evidence(&a, &c(), &ledger).is_none());
    }

    #[test]
    fn in_scope_edit_needs_no_evidence() {
        let a = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/feature/x.rs".into()),
            ..Default::default()
        };
        assert!(requires_evidence(&a, &c(), &ImpactLedger::default()).is_none());
    }
}
