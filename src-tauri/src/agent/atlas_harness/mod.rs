//! Atlas Harness — orchestration (execution-layer entry point).
//!
//! Two-layer control: the Atlas Skill (declaration layer) produces the Goal
//! Contract text and governs the conversation; the Atlas Harness structures it,
//! freezes it, and mechanically enforces it before and after tool execution.
//!
//! In one line: **the Skill makes drift visible; the Harness makes drift get
//! blocked or forced to leave evidence.**
//!
//! Four components:
//!   1. ContractGate         — pre-action structural comparison (preserve / must_not_do / scope).
//!   2. ImpactEvidenceGate   — forbids no-impact claims without evidence (grep before out-of-scope edits).
//!   3. Verifier             — independent adversarial review before task/phase completion.
//!   4. CompletionGate       — `done` must bind a real verification artifact.
//!
//! Plus `path_match`: dependency-free, normalization-aware path/glob matching
//! shared by ContractGate (added in the hardening pass; see
//! docs/REVIEW_FINDINGS.md).
//!
//! Design: **loose coupling.** This module depends only on std + serde; every
//! host touch-point is marked `INTEGRATION:` and supplied by the caller.

pub mod completion_gate;
pub mod contract_gate;
pub mod glue;
pub mod goal_contract;
pub mod impact_evidence;
pub mod path_match;
pub mod verifier;

pub use completion_gate::{can_mark_done, CompletionDecision, TaskEvidenceRef};
pub use contract_gate::{ContractDecision, ProposedAction, Violation};
pub use goal_contract::{GoalContract, ParseResult};
pub use impact_evidence::{EvidenceRequirement, ImpactLedger};
pub use verifier::{build_review_prompt, parse_verdict, VerifierVerdict};

/// Combined verdict for one action after passing through the harness.
#[derive(Debug, Clone)]
pub enum HarnessGate {
    /// Allowed; may execute silently.
    Allow,
    /// Needs impact evidence first (out-of-scope, not yet scanned).
    NeedEvidence(EvidenceRequirement),
    /// Needs a Deviation Notice (soft conflict); record it, then continue.
    Disclose {
        violations: Vec<Violation>,
        reason: String,
    },
    /// Hard conflict; intercept and wait for the user's decision.
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

/// Execution-layer runtime state. One per session; persists once the contract
/// is frozen.
#[derive(Default)]
pub struct AtlasHarness {
    contract: Option<GoalContract>,
    ledger: ImpactLedger,
}

impl AtlasHarness {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse and freeze a contract from a Skill text block.
    /// INTEGRATION: call once the user confirms the Goal Contract in Gate Mode;
    /// persist the returned contract to storage.
    pub fn install_contract_from_skill(&mut self, skill_block: &str) -> ParseResult {
        let mut result = GoalContract::parse_from_skill_block(skill_block);
        result.contract.freeze();
        self.contract = Some(result.contract.clone());
        result
    }

    pub fn contract(&self) -> Option<&GoalContract> {
        self.contract.as_ref()
    }

    /// Whether this session needs harness enforcement throughout.
    pub fn is_active(&self) -> bool {
        self.contract
            .as_ref()
            .is_some_and(|c| c.has_hard_constraints())
    }

    /// Pre-action gate: ContractGate ⊕ ImpactEvidenceGate.
    /// INTEGRATION: call at the tools dispatch site, **after**
    /// `policy::evaluate_tool_execution` (permission gate first, fidelity gate
    /// next). On non-Allow: Block → refuse + emit a Deviation Notice;
    /// NeedEvidence → feed `suggested_command` back so the agent greps first;
    /// Disclose → record, then allow.
    pub fn gate_action(&self, action: &ProposedAction) -> HarnessGate {
        let Some(contract) = &self.contract else {
            return HarnessGate::Allow; // no contract (low-risk / Inline) → do not gate
        };

        // 1) structural comparison
        match contract_gate::evaluate(action, contract) {
            ContractDecision::Block { violations, reason } => {
                return HarnessGate::Block { violations, reason }
            }
            ContractDecision::RequireDisclosure { violations, reason } => {
                return HarnessGate::Disclose { violations, reason }
            }
            ContractDecision::Allow => {}
        }

        // 2) impact evidence (grep before out-of-scope edits)
        if let Some(req) = impact_evidence::requires_evidence(action, contract, &self.ledger) {
            return HarnessGate::NeedEvidence(req);
        }

        HarnessGate::Allow
    }

    /// Call after the agent produces a usage-scan for a target; unlocks
    /// subsequent actions on that target.
    pub fn record_impact_scan(&mut self, target: impl Into<String>) {
        self.ledger.record_scan(target);
    }

    /// Completion gate: verify evidence binding before `done`.
    /// INTEGRATION: wire into the (native) BeforeTaskDone hook.
    pub fn check_completion(&self, task: &TaskEvidenceRef) -> CompletionDecision {
        match &self.contract {
            Some(c) => can_mark_done(task, c),
            None => CompletionDecision::Allow,
        }
    }

    /// Build the independent review prompt.
    /// INTEGRATION: feed it to a read-only "atlas-verifier" subagent spawned via
    /// team_runtime; parse the output with `parse_verdict`; if
    /// `blocks_completion()` is true, prevent `done` and convert the deviation
    /// into a disclosure.
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
            content_or_diff: Some("export const y = 2".into()),
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
