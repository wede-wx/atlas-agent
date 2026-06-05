//! Atlas Harness — ContractGate（动作前契约闸）。
//!
//! 原理：现有的 `policy.rs::evaluate_tool_execution` 是**权限闸**（这个模式能不能写/能不能跑命令）。
//! 偏移用的却是**已授权**的能力去做错的事——把功能注释掉的那次 write 在权限闸面前完全合法。
//! ContractGate 补的是**目标保真闸**：在工具执行前，把 proposed action 的结构（改哪个文件/
//! 跑什么命令/diff 里有什么模式）和**已冻结的 Goal Contract** 做机械比对。
//!
//! 关键点：它**不问 agent “这危不危险”**（那个判断正是坏掉的器官），而是结构匹配。
//! 命中 preserve / must_not_do → Block 或 RequireDisclosure。镜像 policy 的三态决策。

use super::goal_contract::{GoalContract, PreserveKind};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    WriteFile,
    EditFile,
    DeleteFile,
    RunCommand,
    Other,
}

/// 从一次 ToolCall 抽出的、与目标保真相关的最小描述。
/// INTEGRATION：在 registry/core 的 dispatch 处，由 ToolCall(name,args) 构造此结构
/// （例：edit_file → kind=EditFile, target_path=args.path, content_or_diff=args.new_str）。
#[derive(Debug, Clone, Default)]
pub struct ProposedAction {
    pub kind_raw: String, // 工具名，便于诊断
    pub target_path: Option<String>,
    pub command: Option<String>,
    /// 写入内容或 diff 文本——用于检测 stub/mock/注释掉/删测试等模式。
    pub content_or_diff: Option<String>,
}

impl ProposedAction {
    pub fn kind(&self) -> ActionKind {
        let n = self.kind_raw.to_lowercase();
        if n.contains("delete") || n.contains("remove") || n.contains("rm") {
            ActionKind::DeleteFile
        } else if n.contains("edit") || n.contains("str_replace") || n.contains("patch") {
            ActionKind::EditFile
        } else if n.contains("write") || n.contains("create_file") {
            ActionKind::WriteFile
        } else if n.contains("command")
            || n.contains("bash")
            || n.contains("shell")
            || n.contains("exec")
        {
            ActionKind::RunCommand
        } else {
            ActionKind::Other
        }
    }

    fn is_mutating(&self) -> bool {
        !matches!(self.kind(), ActionKind::Other)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Violation {
    pub item_id: String,
    pub item_text: String,
    /// 命中原因（给用户看的人话）。
    pub why: String,
}

/// 三态决策，镜像 `policy::PolicyDecision`。
/// - Allow：与契约不冲突，放行。
/// - RequireDisclosure：可能偏离，但需先**披露 + 证据**（交给 ImpactEvidenceGate / Deviation Notice）。
/// - Block：硬冲突，直接挡住，等用户决策。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum ContractDecision {
    Allow,
    RequireDisclosure {
        violations: Vec<Violation>,
        reason: String,
    },
    Block {
        violations: Vec<Violation>,
        reason: String,
    },
}

impl ContractDecision {
    pub fn is_block(&self) -> bool {
        matches!(self, ContractDecision::Block { .. })
    }
    pub fn allows_silent_execution(&self) -> bool {
        matches!(self, ContractDecision::Allow)
    }
}

/// 危险代码模式——在写入内容/diff 里命中即视为可能的伪完成/降级。
/// 这是“按动作签名停”的代码级版本：不依赖 agent 主观判断。
const DOWNGRADE_PATTERNS: &[(&str, &str)] = &[
    ("unimplemented!", "占位实现 unimplemented!()"),
    ("todo!(", "占位实现 todo!()"),
    ("notimplemented", "占位实现 NotImplemented"),
    ("// todo", "TODO 占位"),
    ("# todo", "TODO 占位"),
    ("placeholder", "placeholder 占位"),
    ("mock", "mock 替换真实实现的嫌疑"),
    ("fakedata", "假数据"),
    ("return null; // stub", "stub 返回"),
    ("throw new error(\"not", "抛 not-implemented"),
];

/// 命令层的破坏性模式（删测试 / 跳过校验 / 强推）。
const DANGEROUS_COMMAND_PATTERNS: &[(&str, &str)] = &[
    ("--no-verify", "跳过校验钩子 (--no-verify)"),
    ("rm -rf", "递归删除"),
    ("git reset --hard", "硬重置丢弃改动"),
    ("git push --force", "强推覆盖历史"),
    ("skip", "疑似跳过（skip）测试/校验"),
    ("xfail", "把测试标记为预期失败"),
    (".skip(", "测试 .skip()"),
    ("disable", "禁用（disable）某项行为"),
];

/// 核心：把一次 proposed action 和契约做结构比对。纯函数，无 LLM、无 IO。
pub fn evaluate(action: &ProposedAction, contract: &GoalContract) -> ContractDecision {
    if !action.is_mutating() {
        return ContractDecision::Allow; // 只读动作不受目标保真闸约束
    }

    let mut violations: Vec<Violation> = Vec::new();
    let mut hard = false;

    // 1) Preserve（File / LayoutStructure）：动到被保留路径 → 硬冲突。
    if let Some(path) = action.target_path.as_deref() {
        for p in &contract.preserve {
            if matches!(p.kind, PreserveKind::File | PreserveKind::LayoutStructure) {
                if let Some(glob) = &p.path_glob {
                    if glob_matches(glob, path) {
                        hard = true;
                        violations.push(Violation {
                            item_id: p.id.clone(),
                            item_text: p.text.clone(),
                            why: format!("修改了被 Preserve 的路径 `{path}`（匹配 `{glob}`）"),
                        });
                    }
                }
            }
        }
    }

    // 2) 内容/diff 里的降级模式（stub/mock/占位/删测试）。
    if let Some(body) = action.content_or_diff.as_deref() {
        let lower = body.to_lowercase();
        for (pat, why) in DOWNGRADE_PATTERNS {
            if lower.contains(pat) {
                // 命中 must_not_do 的 mock/stub 防线 → 需披露（可能是合理的 test-only mock，交给用户/证据判断）
                let item = contract.must_not_do.iter().find(|i| {
                    i.id == "N-mock" || i.text.contains("mock") || i.text.contains("stub")
                });
                violations.push(Violation {
                    item_id: item
                        .map(|i| i.id.clone())
                        .unwrap_or_else(|| "N-mock".into()),
                    item_text: item.map(|i| i.text.clone()).unwrap_or_default(),
                    why: format!("写入内容包含 {why}"),
                });
            }
        }
        // 删除导出符号 / 大段删除的粗检（diff 以 '-' 开头的行占比高）
        if looks_like_mass_deletion(body) {
            violations.push(Violation {
                item_id: "N-hide".into(),
                item_text: "未经披露不得隐藏/移除请求的功能".into(),
                why: "diff 中存在大段删除，可能在移除已实现行为".into(),
            });
        }
    }

    // 3) 命令层破坏性模式。
    if let Some(cmd) = action.command.as_deref() {
        let lower = cmd.to_lowercase();
        for (pat, why) in DANGEROUS_COMMAND_PATTERNS {
            if lower.contains(pat) {
                // 删/弱化测试命中 must_not_do 的 N-test → 硬
                let test_related = matches!(*pat, "skip" | "xfail" | ".skip(" | "disable");
                if test_related {
                    hard = true;
                }
                violations.push(Violation {
                    item_id: "N-test".into(),
                    item_text: "未经披露不得删除/弱化保护契约项的测试".into(),
                    why: format!("命令包含 {why}"),
                });
            }
        }
    }

    // 4) 范围越界：动到既不在 in_scope、又明确属于 out_of_scope 的目标 → 需披露。
    if let Some(path) = action.target_path.as_deref() {
        if !contract.scope.out_of_scope.is_empty()
            && contract
                .scope
                .out_of_scope
                .iter()
                .any(|s| path.contains(s.as_str()))
        {
            violations.push(Violation {
                item_id: "scope".into(),
                item_text: "范围外目标".into(),
                why: format!("`{path}` 落在声明的 out_of_scope 内"),
            });
        }
    }

    if violations.is_empty() {
        ContractDecision::Allow
    } else if hard {
        ContractDecision::Block {
            reason: "动作与硬性契约项冲突，已拦截，等待用户决策".into(),
            violations,
        }
    } else {
        ContractDecision::RequireDisclosure {
            reason: "动作可能偏离契约，需先披露并给出证据后才能继续".into(),
            violations,
        }
    }
}

/// 极简 glob：支持 `**`（任意层）/ `*`（单层内任意）/ 字面量。够 ContractGate 用，避免引新依赖。
fn glob_matches(glob: &str, path: &str) -> bool {
    fn to_regex(glob: &str) -> String {
        let mut re = String::from("^");
        let bytes: Vec<char> = glob.chars().collect();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                '*' => {
                    if i + 1 < bytes.len() && bytes[i + 1] == '*' {
                        re.push_str(".*");
                        i += 2;
                        continue;
                    }
                    re.push_str("[^/]*");
                }
                '.' => re.push_str("\\."),
                '/' => re.push('/'),
                c => re.push(c),
            }
            i += 1;
        }
        re.push('$');
        re
    }
    match regex::Regex::new(&to_regex(glob)) {
        Ok(re) => re.is_match(path),
        Err(_) => path.contains(glob.trim_end_matches(['*', '/'])),
    }
}

fn looks_like_mass_deletion(diff: &str) -> bool {
    let mut minus = 0usize;
    let mut plus = 0usize;
    for l in diff.lines() {
        if l.starts_with('-') && !l.starts_with("---") {
            minus += 1;
        } else if l.starts_with('+') && !l.starts_with("+++") {
            plus += 1;
        }
    }
    minus >= 15 && minus > plus * 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::atlas_harness::goal_contract::GoalContract;

    fn contract() -> GoalContract {
        let text = r#"
Goal:
- ship feature X
Must Do:
- [M1] implement feature X (hard)
Preserve:
- [P1] keep layout in src/ui/** (layout)
Out Of Scope:
- billing
"#;
        GoalContract::parse_from_skill_block(text).contract
    }

    #[test]
    fn editing_preserved_path_is_blocked() {
        let a = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/ui/Canvas.tsx".into()),
            ..Default::default()
        };
        assert!(evaluate(&a, &contract()).is_block());
    }

    #[test]
    fn stub_content_requires_disclosure() {
        let a = ProposedAction {
            kind_raw: "write_file".into(),
            target_path: Some("src/api/auth.rs".into()),
            content_or_diff: Some("fn login() { todo!() }".into()),
            ..Default::default()
        };
        let d = evaluate(&a, &contract());
        assert!(matches!(d, ContractDecision::RequireDisclosure { .. }));
    }

    #[test]
    fn deleting_tests_via_command_is_blocked() {
        let a = ProposedAction {
            kind_raw: "command".into(),
            command: Some("pytest --skip-broken && sed -i 's/test_/xtest_/'".into()),
            ..Default::default()
        };
        assert!(evaluate(&a, &contract()).is_block());
    }

    #[test]
    fn clean_in_scope_edit_is_allowed() {
        let a = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/feature_x.rs".into()),
            content_or_diff: Some("pub fn x() -> i32 { 42 }".into()),
            ..Default::default()
        };
        assert_eq!(evaluate(&a, &contract()), ContractDecision::Allow);
    }

    #[test]
    fn out_of_scope_target_requires_disclosure() {
        let a = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/billing/charge.rs".into()),
            ..Default::default()
        };
        assert!(matches!(
            evaluate(&a, &contract()),
            ContractDecision::RequireDisclosure { .. }
        ));
    }
}
