//! Atlas Harness — orchestration（执行层总入口）。
//!
//! 双层管制中的**执行层**:Atlas Skill(声明层)产出 Goal Contract 文本并治理对话;
//! Atlas Harness 把它结构化、冻结,并在工具执行前后做机械强制。
//!
//! 一句话分工:**Skill 让偏移可见;Harness 让偏移被挡住或被强制留证。**
//!
//! 四个组件:
//!   1. ContractGate         —— 动作前结构比对(preserve/must_not_do/scope)。
//!   2. ImpactEvidenceGate   —— 禁止无证据的“无影响”断言(scope 外先 grep)。
//!   3. Verifier             —— task/phase 完成前的独立对抗式复查。
//!   4. CompletionGate       —— done 必须绑定真实验证产物。
//!
//! 设计取向:**松耦合**。本模块只依赖 std + serde + regex,所有与宿主架构的接点都标了
//! `INTEGRATION:`,由调用方提供 ToolCall→ProposedAction 的转换、契约的读写、reviewer 的派发。

pub mod completion_gate;
pub mod contract_gate;
pub mod glue;
pub mod goal_contract;
pub mod impact_evidence;
pub mod verifier;

pub use completion_gate::{can_mark_done, CompletionDecision, TaskEvidenceRef};
pub use contract_gate::{ContractDecision, ProposedAction, Violation};
pub use goal_contract::{GoalContract, ParseResult};
pub use impact_evidence::{EvidenceRequirement, ImpactLedger};
pub use verifier::{build_review_prompt, parse_verdict, VerifierVerdict};

/// 一次动作经过 harness 后的综合裁决。
#[derive(Debug, Clone)]
pub enum HarnessGate {
    /// 放行,可静默执行。
    Allow,
    /// 需先出影响证据(scope 外、未扫过)。
    NeedEvidence(EvidenceRequirement),
    /// 需披露偏离(软冲突),记 Deviation Notice 后可继续。
    Disclose {
        violations: Vec<Violation>,
        reason: String,
    },
    /// 硬冲突,拦截,等用户决策。
    Block {
        violations: Vec<Violation>,
        reason: String,
    },
}

impl HarnessGate {
    pub fn permits_silent_execution(&self) -> bool {
        matches!(self, HarnessGate::Allow)
    }
}

/// 执行层运行时状态。每个 session 一个,契约冻结后驻留。
pub struct AtlasHarness {
    contract: Option<GoalContract>,
    ledger: ImpactLedger,
}

impl Default for AtlasHarness {
    fn default() -> Self {
        Self {
            contract: None,
            ledger: ImpactLedger::default(),
        }
    }
}

impl AtlasHarness {
    pub fn new() -> Self {
        Self::default()
    }

    /// 从 Skill 文本块装入并冻结契约。
    /// INTEGRATION:在 Gate Mode 用户确认 Goal Contract 后调用;把返回的契约持久化到 storage。
    pub fn install_contract_from_skill(&mut self, skill_block: &str) -> ParseResult {
        let mut result = GoalContract::parse_from_skill_block(skill_block);
        result.contract.freeze();
        self.contract = Some(result.contract.clone());
        result
    }

    pub fn contract(&self) -> Option<&GoalContract> {
        self.contract.as_ref()
    }

    /// 该 session 是否需要全程开启 harness 强制。
    pub fn is_active(&self) -> bool {
        self.contract
            .as_ref()
            .is_some_and(|c| c.has_hard_constraints())
    }

    /// 动作前总闸:ContractGate ⊕ ImpactEvidenceGate。
    /// INTEGRATION:在 tools 的 dispatch 处、`policy::evaluate_tool_execution` **之后**调用
    /// (权限闸先过,目标保真闸再过)。返回非 Allow 时:Block→拒绝并 emit Deviation Notice;
    /// NeedEvidence→把 suggested_command 回灌给 agent 让它先 grep;Disclose→记录后放行。
    pub fn gate_action(&self, action: &ProposedAction) -> HarnessGate {
        let Some(contract) = &self.contract else {
            return HarnessGate::Allow; // 无契约(低风险/Inline)→不拦
        };

        // 1) 结构比对
        match contract_gate::evaluate(action, contract) {
            ContractDecision::Block { violations, reason } => {
                return HarnessGate::Block { violations, reason }
            }
            ContractDecision::RequireDisclosure { violations, reason } => {
                return HarnessGate::Disclose { violations, reason }
            }
            ContractDecision::Allow => {}
        }

        // 2) 影响证据(scope 外先 grep)
        if let Some(req) = impact_evidence::requires_evidence(action, contract, &self.ledger) {
            return HarnessGate::NeedEvidence(req);
        }

        HarnessGate::Allow
    }

    /// agent 出具了对某目标的 usage-scan 证据后调用,解锁后续对该目标的动作。
    pub fn record_impact_scan(&mut self, target: impl Into<String>) {
        self.ledger.record_scan(target);
    }

    /// 完成闸:done 前校验证据绑定。
    /// INTEGRATION:接进 BeforeTaskDone hook(原生)。
    pub fn check_completion(&self, task: &TaskEvidenceRef) -> CompletionDecision {
        match &self.contract {
            Some(c) => can_mark_done(task, c),
            None => CompletionDecision::Allow,
        }
    }

    /// 生成独立审查 prompt。
    /// INTEGRATION:把它喂给 team_runtime 起的只读 "atlas-verifier" 子 agent,
    /// 输出用 `parse_verdict` 解析;blocks_completion()=true 则阻止 done 并把 deviation 转成披露。
    pub fn build_verifier_prompt(&self, diff: &str, test_evidence: &str) -> Option<String> {
        self.contract
            .as_ref()
            .map(|c| build_review_prompt(c, diff, test_evidence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn harness() -> AtlasHarness {
        let mut h = AtlasHarness::new();
        h.install_contract_from_skill(
            "Goal:\n- ship X\nMust Do:\n- [M1] implement X (hard)\nPreserve:\n- [P1] keep src/ui/** (layout)\nIn Scope:\n- src/x\n",
        );
        h
    }

    #[test]
    fn blocks_edit_to_preserved_path() {
        let h = harness();
        let a = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/ui/App.tsx".into()),
            ..Default::default()
        };
        assert!(matches!(h.gate_action(&a), HarnessGate::Block { .. }));
    }

    #[test]
    fn needs_evidence_for_out_of_scope_then_clears() {
        let mut h = harness();
        let a = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/shared/util.rs".into()),
            ..Default::default()
        };
        assert!(matches!(h.gate_action(&a), HarnessGate::NeedEvidence(_)));
        h.record_impact_scan("src/shared/util.rs");
        assert!(h.gate_action(&a).permits_silent_execution());
    }

    #[test]
    fn active_only_with_hard_constraints() {
        assert!(harness().is_active());
        assert!(!AtlasHarness::new().is_active());
    }
}
