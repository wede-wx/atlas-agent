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
    /// B1: a hard conflict the user has explicitly approved for this
    /// (item, target) pair. Execution proceeds, but the deviation is
    /// mechanically disclosed (audit trail) — approval is not amnesia.
    ApprovedDeviation {
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
    /// B1: user-approved deviations as (violation item_id, action target
    /// signature) pairs. Only the commands layer (a user-side Tauri command)
    /// can populate this — the model has no tool that writes here.
    approvals: std::collections::BTreeSet<(String, String)>,
}

/// B1: stable target signature for approval matching. Deliberately strict —
/// paths match verbatim, commands match the full trimmed command line — so an
/// approval never covers more than the exact action the user looked at.
pub fn action_target_signature(action: &ProposedAction) -> String {
    if let Some(path) = action.target_path.as_deref() {
        return format!("path:{path}");
    }
    if let Some(command) = action.command.as_deref() {
        return format!("command:{}", command.trim());
    }
    format!("kind:{}", action.kind_raw)
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

    /// A6: install an already-structured contract — the reinstall path when a
    /// persisted contract is reloaded from storage after AgentCore rebuild /
    /// session re-entry. Deliberately does NOT re-parse and does NOT re-inject
    /// default guards: the contract is restored exactly as it was frozen.
    /// Freezes defensively (a no-op for correctly persisted contracts) so a
    /// reinstalled contract can never be weaker than the original.
    pub fn install_contract(&mut self, mut contract: GoalContract) {
        contract.freeze();
        self.contract = Some(contract);
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
                // B1: a Block downgrades to ApprovedDeviation only when EVERY
                // violated item has been user-approved for this exact target.
                // Any unapproved violation keeps the full original Block —
                // partial approval never weakens the gate.
                let target = action_target_signature(action);
                let all_approved = !violations.is_empty()
                    && violations.iter().all(|violation| {
                        self.approvals
                            .contains(&(violation.item_id.clone(), target.clone()))
                    });
                if all_approved {
                    return HarnessGate::ApprovedDeviation {
                        violations,
                        reason: format!("用户已批准的契约偏离（目标：{target}）：{reason}"),
                    };
                }
                return HarnessGate::Block { violations, reason };
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

    /// B1: install user-approved deviations (the rehydration path — commands
    /// layer loads them from storage alongside the contract). Replaces the
    /// current set so a revoked approval disappears on the next run.
    pub fn install_approvals(&mut self, approvals: impl IntoIterator<Item = (String, String)>) {
        self.approvals = approvals.into_iter().collect();
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

    #[test]
    fn reinstalled_contract_gates_identically_to_parser_install() {
        // A6: install_contract (the rehydration path) must arm the harness
        // exactly like install_contract_from_skill did originally.
        let original = harness();
        let contract = original.contract().unwrap().clone();

        let mut rebuilt = AtlasHarness::new();
        rebuilt.install_contract(contract);

        assert!(rebuilt.is_active());
        let action = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/ui/App.tsx".into()),
            ..Default::default()
        };
        assert!(matches!(
            rebuilt.gate_action(&action),
            HarnessGate::Block { .. }
        ));
    }

    #[test]
    fn contract_survives_serde_round_trip_and_still_gates() {
        // A6 end-to-end at the type level: freeze → serialize (what storage
        // persists) → deserialize → reinstall → same Block decision.
        let original = harness();
        let json = serde_json::to_value(original.contract().unwrap()).unwrap();
        let restored: GoalContract = serde_json::from_value(json).unwrap();
        assert!(restored.frozen, "frozen flag must survive persistence");

        let mut rebuilt = AtlasHarness::new();
        rebuilt.install_contract(restored);
        assert!(rebuilt.is_active());
        let action = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/ui/App.tsx".into()),
            ..Default::default()
        };
        assert!(matches!(
            rebuilt.gate_action(&action),
            HarnessGate::Block { .. }
        ));
    }

    #[test]
    fn approved_deviation_downgrades_block_with_evidence_not_to_allow() {
        // B1：精确 (item, target) 批准 → ApprovedDeviation（留痕放行档），
        // 绝不落到 Allow（permits_silent_execution 必须仍为 false）。
        let mut h = harness();
        let action = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/ui/App.tsx".into()),
            ..Default::default()
        };
        let HarnessGate::Block { violations, .. } = h.gate_action(&action) else {
            panic!("baseline must Block");
        };
        let target = action_target_signature(&action);
        h.install_approvals(
            violations
                .iter()
                .map(|violation| (violation.item_id.clone(), target.clone())),
        );

        let gate = h.gate_action(&action);
        assert!(matches!(gate, HarnessGate::ApprovedDeviation { .. }));
        assert!(
            !gate.permits_silent_execution(),
            "approval must keep the disclosure obligation"
        );
    }

    #[test]
    fn approval_is_target_and_item_exact() {
        // B1：换目标 / 换条款都不在批准范围内——部分批准不弱化闸。
        let mut h = harness();
        let approved_action = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/ui/App.tsx".into()),
            ..Default::default()
        };
        let HarnessGate::Block { violations, .. } = h.gate_action(&approved_action) else {
            panic!("baseline must Block");
        };
        let target = action_target_signature(&approved_action);
        h.install_approvals(
            violations
                .iter()
                .map(|violation| (violation.item_id.clone(), target.clone())),
        );

        // 同条款、不同文件 → 仍 Block。
        let other_file = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/ui/Other.tsx".into()),
            ..Default::default()
        };
        assert!(matches!(
            h.gate_action(&other_file),
            HarnessGate::Block { .. }
        ));

        // 批准被替换为空集（撤销路径语义）→ 原动作恢复 Block。
        h.install_approvals(std::iter::empty::<(String, String)>());
        assert!(matches!(
            h.gate_action(&approved_action),
            HarnessGate::Block { .. }
        ));
    }

    #[test]
    fn target_signature_is_stable_and_strict() {
        let path_action = ProposedAction {
            kind_raw: "edit_file".into(),
            target_path: Some("src/ui/App.tsx".into()),
            ..Default::default()
        };
        assert_eq!(action_target_signature(&path_action), "path:src/ui/App.tsx");

        let command_action = ProposedAction {
            kind_raw: "run_command".into(),
            command: Some("  rm -rf build  ".into()),
            ..Default::default()
        };
        assert_eq!(
            action_target_signature(&command_action),
            "command:rm -rf build",
            "full command line, not just the program head — approvals stay narrow"
        );

        let bare = ProposedAction {
            kind_raw: "invoke_mcp_tool::apply_edit".into(),
            ..Default::default()
        };
        assert_eq!(
            action_target_signature(&bare),
            "kind:invoke_mcp_tool::apply_edit"
        );
    }

    #[test]
    fn install_contract_freezes_defensively() {
        // A legacy/hand-built blob with frozen=false must not produce an
        // unfrozen (weaker) harness after reinstall.
        let mut unfrozen = harness().contract().unwrap().clone();
        unfrozen.frozen = false;
        let mut h = AtlasHarness::new();
        h.install_contract(unfrozen);
        assert!(h.contract().unwrap().frozen);
    }
}
