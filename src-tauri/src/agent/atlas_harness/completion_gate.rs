//! Atlas Harness — EvidenceBoundCompletion（证据绑定完成闸）。
//!
//! 现状:`final_audit.rs` 里 `evidence_status=verified` 与真实 `verification_id` 的绑定是 TODO
//! (“目前为空,由 M3.4 repair loop 填充”)。在补上之前,一个 task 可能没有验证产物就被标 verified,
//! 自评的乐观能漏过。
//!
//! 本闸把“我觉得完成了”换成“这是完成的证据”:硬性 Must Do 覆盖的 task 要标 done,
//! 必须 evidence_status=verified **且** 至少一条 verification_id 指向真实 run_verify 产物。
//! INTEGRATION:把它接进现有的 BeforeTaskDone hook(改为原生 Rust 检查,而非用户 shell)。
//!
//! 接线状态（本次修复）
//! --------------------
//! 之前 `can_mark_done` 只有测试调用、没有生产调用方——“verified 必须绑定真实
//! 验证产物”因此是个全开的后门：模型在 `update_plan_task` 里直接传
//! `evidence_status=verified` 就能落库。现在通用绑定规则由
//! [`verified_claim_is_bound`] 表达，并在 `tools/plan_tasks.rs` 的
//! `update_plan_task`（“done/verified”唯一的模型侧写入点）强制执行：落库前
//! 查询 `run_task_verifications`，没有 passed 记录的 verified 声明一律拒绝。
//! `waived` 仍是显式、可审计的逃生口。契约感知的 `can_mark_done`（硬性项
//! 覆盖检查）保留为库函数，供 harness 持有契约的调用点使用。

use super::goal_contract::GoalContract;
use serde::Serialize;

/// 一个 task 的证据引用(来自现有 final_audit / storage 的 task 记录)。
#[derive(Debug, Clone, Default)]
pub struct TaskEvidenceRef {
    pub task_id: String,
    /// "verified" / "pending" / "failed" / "blocked"
    pub evidence_status: String,
    /// 指向真实 run_verify 运行的 id。空 = 无硬证据。
    pub verification_ids: Vec<String>,
    /// 该 task 覆盖的契约项 id(M.../N.../P...)。
    pub covered_items: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum CompletionDecision {
    Allow,
    Block { reason: String },
}

impl CompletionDecision {
    pub fn is_block(&self) -> bool {
        matches!(self, CompletionDecision::Block { .. })
    }
}

/// 契约无关的通用绑定规则：声称 `verified` 必须至少绑定一条真实验证产物。
/// 这是 CompletionGate 的最低线,不需要 GoalContract 也必须成立——
/// 所以单独抽出来,让没有契约上下文的调用点（如 update_plan_task）也能强制。
pub fn verified_claim_is_bound(evidence_status: &str, passed_verification_count: usize) -> bool {
    !evidence_status.eq_ignore_ascii_case("verified") || passed_verification_count > 0
}

/// 能否把该 task 标记为 done。
pub fn can_mark_done(task: &TaskEvidenceRef, contract: &GoalContract) -> CompletionDecision {
    let covers_hard = task.covered_items.iter().any(|id| {
        contract.must_do.iter().any(|m| &m.id == id && m.hard)
            || contract.preserve.iter().any(|p| &p.id == id)
    });

    // 覆盖硬性项的 task:必须 verified + 有真实 verification_id。
    if covers_hard {
        if task.evidence_status != "verified" {
            return CompletionDecision::Block {
                reason: format!(
                    "task `{}` 覆盖硬性契约项,但 evidence_status = {} (≠ verified),不能标 done",
                    task.task_id, task.evidence_status
                ),
            };
        }
        if task.verification_ids.is_empty() {
            return CompletionDecision::Block {
                reason: format!(
                    "task `{}` 标了 verified 但没有任何 verification_id 绑定真实验证产物——拒绝凭自评完成",
                    task.task_id
                ),
            };
        }
    }

    // 任何 failed/blocked 都不能 done。
    if matches!(task.evidence_status.as_str(), "failed" | "blocked") {
        return CompletionDecision::Block {
            reason: format!(
                "task `{}` evidence_status = {}",
                task.task_id, task.evidence_status
            ),
        };
    }

    CompletionDecision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::atlas_harness::goal_contract::GoalContract;

    fn c() -> GoalContract {
        GoalContract::parse_from_skill_block("Goal:\n- x\nMust Do:\n- [M1] real backend (hard)\n")
            .contract
    }

    #[test]
    fn hard_task_without_verification_id_blocked() {
        let t = TaskEvidenceRef {
            task_id: "t1".into(),
            evidence_status: "verified".into(),
            verification_ids: vec![],
            covered_items: vec!["M1".into()],
        };
        assert!(can_mark_done(&t, &c()).is_block());
    }

    #[test]
    fn hard_task_with_real_verification_allowed() {
        let t = TaskEvidenceRef {
            task_id: "t1".into(),
            evidence_status: "verified".into(),
            verification_ids: vec!["verify-42".into()],
            covered_items: vec!["M1".into()],
        };
        assert_eq!(can_mark_done(&t, &c()), CompletionDecision::Allow);
    }

    #[test]
    fn verified_claim_requires_at_least_one_artifact() {
        assert!(!verified_claim_is_bound("verified", 0));
        assert!(verified_claim_is_bound("verified", 1));
        assert!(verified_claim_is_bound("pending", 0));
        assert!(verified_claim_is_bound("waived", 0));
    }

    #[test]
    fn pending_hard_task_blocked() {
        let t = TaskEvidenceRef {
            task_id: "t1".into(),
            evidence_status: "pending".into(),
            verification_ids: vec![],
            covered_items: vec!["M1".into()],
        };
        assert!(can_mark_done(&t, &c()).is_block());
    }
}
