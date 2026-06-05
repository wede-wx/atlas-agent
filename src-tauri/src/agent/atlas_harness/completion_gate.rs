//! Atlas Harness — EvidenceBoundCompletion（证据绑定完成闸）。
//!
//! 现状:`final_audit.rs` 里 `evidence_status=verified` 与真实 `verification_id` 的绑定是 TODO
//! (“目前为空,由 M3.4 repair loop 填充”)。在补上之前,一个 task 可能没有验证产物就被标 verified,
//! 自评的乐观能漏过。
//!
//! 本闸把“我觉得完成了”换成“这是完成的证据”:硬性 Must Do 覆盖的 task 要标 done,
//! 必须 evidence_status=verified **且** 至少一条 verification_id 指向真实 run_verify 产物。
//! INTEGRATION:把它接进现有的 BeforeTaskDone hook(改为原生 Rust 检查,而非用户 shell)。

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
