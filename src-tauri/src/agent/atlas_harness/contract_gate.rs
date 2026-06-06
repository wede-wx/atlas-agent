//! Atlas Harness — ContractGate (pre-action goal-fidelity gate), hardened.
//!
//! Relationship to the permission gate
//! -----------------------------------
//! `policy.rs::evaluate_tool_execution` is the **permission** gate (may this
//! mode write / run commands at all). Drift uses *already-authorized*
//! capabilities to do the wrong thing — commenting a feature out is a perfectly
//! legal write to the permission gate. ContractGate is the orthogonal
//! **goal-fidelity** gate: before a tool runs, it compares the *structure* of
//! the proposed action (which file, which command, what patterns are in the
//! diff) against the **frozen Goal Contract**. It never asks the model "is this
//! dangerous?" (that judgment is the broken organ) — it matches structurally.
//!
//! What changed in the hardening pass (see docs/REVIEW_FINDINGS.md)
//! ---------------------------------------------------------------
//! 1. **Path matching** now goes through `path_match::path_matches_glob`, which
//!    normalizes `.`/`..`/`\`/`//` and matches path suffixes. This closes the
//!    silent bypasses of a *hard* Preserve via `./`, `..`, backslashes, or an
//!    absolute path. (Previously a raw-string regex.)
//! 2. **`is_mutating()` is fail-closed.** Tool-kind was inferred only from the
//!    tool *name*; a write tool with an unrecognized name (e.g. an MCP tool)
//!    classified as `Other` and skipped every check. We now also treat an
//!    action as mutating when its *arguments* look writeful (a path + content,
//!    or a command), so unknown-but-writeful tools can no longer slip through.
//! 3. **Command matching is tokenized and high-precision.** The old bare
//!    substrings `skip` / `disable` matched `--skipLibCheck`, `--disable-foo`,
//!    `skipper`, etc. (constant false blocks) and were trivially evadable. We
//!    now tokenize and match exact flags/sequences, and reserve `hard` only for
//!    verification-defeating commands (`--no-verify`, `xfail`) — the one class
//!    that must never be silent, because CompletionGate relies on that
//!    verification.
//! 4. **Mass-deletion detection works on the common edit path.** For
//!    `str_replace`-style edits the gate now compares `prior_content` (old_str)
//!    against the replacement, instead of only scanning a unified diff for `-`
//!    lines (which an edit's replacement text never contains).
//! 5. **Content stub detection is word-aware.** Bare `// todo` / `# todo` were
//!    dropped (every codebase has TODOs; a TODO comment is not a false
//!    completion — `todo!()` the macro is). `mock`/`stub`/`fake`/`dummy` now
//!    match as identifier segments with a benign denylist, so `mockup` no
//!    longer trips the gate while `mockUser` still does.
//!
//! The matcher logic was validated against the full bypass + false-positive
//! corpus before this file was written.

use super::goal_contract::{GoalContract, PreserveKind};
use super::path_match::path_matches_glob;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    WriteFile,
    EditFile,
    DeleteFile,
    RunCommand,
    Other,
}

/// Minimal goal-fidelity-relevant description extracted from one ToolCall.
/// INTEGRATION: built by `glue::proposed_action_from_tool_call` at the dispatch
/// site. `prior_content` is the pre-edit text (old_str) when the tool exposes
/// it — used for mass-deletion detection on edits.
#[derive(Debug, Clone, Default)]
pub struct ProposedAction {
    pub kind_raw: String,
    pub target_path: Option<String>,
    pub command: Option<String>,
    /// Replacement text, new file content, or a unified diff.
    pub content_or_diff: Option<String>,
    /// Pre-edit content (old_str / old_string) when available.
    pub prior_content: Option<String>,
}

impl ProposedAction {
    /// Name-based kind *hint*. Kept for callers that branch on it (e.g. the
    /// read-scan recorder). It is intentionally only a hint — real
    /// mutation-ness is decided by [`Self::is_mutating`].
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

    /// Fail-closed mutation test. True when the name looks mutating, **or** when
    /// the arguments look writeful regardless of the name. This is what closes
    /// the unknown-write-tool bypass; it is `pub` so the dispatch site can use
    /// the same definition when deciding whether a successful call counts as a
    /// read scan.
    pub fn is_mutating(&self) -> bool {
        if !matches!(self.kind(), ActionKind::Other) {
            return true;
        }
        let writeful_args = self.target_path.is_some() && self.content_or_diff.is_some();
        writeful_args || self.command.is_some()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Violation {
    pub item_id: String,
    pub item_text: String,
    /// Human-readable reason (shown to the user).
    pub why: String,
}

/// Three-state decision, mirroring `policy::PolicyDecision`.
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

/// Unambiguous stub markers (substring match is safe — these almost never
/// appear benignly). Disclosure-level by default.
const STUB_MARKERS: &[(&str, &str)] = &[
    ("unimplemented!", "placeholder unimplemented!()"),
    ("todo!(", "placeholder todo!()"),
    ("unreachable!(", "placeholder unreachable!()"),
    ("notimplementederror", "NotImplementedError placeholder"),
    ("raise notimplemented", "raise NotImplemented"),
    ("not implemented", "\"not implemented\" placeholder"),
    ("placeholder", "explicit placeholder"),
];

/// Mock-family identifier roots, matched as identifier segments (not raw
/// substrings) with a benign denylist so `mockup` does not trip the gate.
const MOCK_ROOTS: &[&str] = &["mock", "stub", "fake", "dummy"];
const MOCK_BENIGN: &[&str] = &["mockup", "mockups", "smock", "hammock", "mockingbird"];

/// Core: structurally compare one proposed action against the contract.
/// Pure function — no LLM, no I/O.
pub fn evaluate(action: &ProposedAction, contract: &GoalContract) -> ContractDecision {
    if !action.is_mutating() {
        return ContractDecision::Allow; // read-only actions are out of scope here
    }

    let mut violations: Vec<Violation> = Vec::new();
    let mut hard = false;

    // 1) Preserve (File / LayoutStructure): touching a preserved path is a hard conflict.
    if let Some(path) = action.target_path.as_deref() {
        for p in &contract.preserve {
            if matches!(p.kind, PreserveKind::File | PreserveKind::LayoutStructure) {
                if let Some(glob) = &p.path_glob {
                    if path_matches_glob(glob, path) {
                        hard = true;
                        violations.push(Violation {
                            item_id: p.id.clone(),
                            item_text: p.text.clone(),
                            why: format!("modifies Preserve path `{path}` (matches `{glob}`)"),
                        });
                    }
                }
            }
        }
    }

    // 2) Downgrade patterns in the written content / diff (stub / mock / placeholder).
    if let Some(body) = action.content_or_diff.as_deref() {
        let lower = body.to_lowercase();
        for (pat, why) in STUB_MARKERS {
            if lower.contains(pat) {
                violations.push(downgrade_violation(
                    contract,
                    &format!("written content contains {why}"),
                ));
            }
        }
        if let Some(tok) = first_mock_like(&lower) {
            violations.push(downgrade_violation(
                contract,
                &format!("written content references `{tok}` (possible mock/stub replacing a real implementation)"),
            ));
        }
        if looks_like_mass_deletion(action) {
            violations.push(Violation {
                item_id: "N-hide".into(),
                item_text: "must not hide/remove requested functionality without disclosure".into(),
                why: "edit removes a large block — may be deleting implemented behavior".into(),
            });
        }
    }

    // 3) Destructive / verification-defeating commands.
    if let Some(cmd) = action.command.as_deref() {
        for hit in command_hits(cmd) {
            if hit.hard {
                hard = true;
            }
            violations.push(Violation {
                item_id: hit.item_id.into(),
                item_text: hit.item_text.into(),
                why: hit.why,
            });
        }
    }

    // 4) Out-of-scope target → disclosure (segment/glob aware, not naive substring).
    if let Some(path) = action.target_path.as_deref() {
        for entry in &contract.scope.out_of_scope {
            if out_of_scope_hit(entry, path) {
                violations.push(Violation {
                    item_id: "scope".into(),
                    item_text: "out-of-scope target".into(),
                    why: format!("`{path}` falls under declared out_of_scope `{entry}`"),
                });
                break;
            }
        }
    }

    if violations.is_empty() {
        ContractDecision::Allow
    } else if hard {
        ContractDecision::Block {
            reason: "action conflicts with a hard contract item; blocked pending user decision"
                .into(),
            violations,
        }
    } else {
        ContractDecision::RequireDisclosure {
            reason: "action may deviate from the contract; disclose and provide evidence before continuing".into(),
            violations,
        }
    }
}

fn downgrade_violation(contract: &GoalContract, why: &str) -> Violation {
    let item = contract
        .must_not_do
        .iter()
        .find(|i| i.id == "N-mock" || i.text.contains("mock") || i.text.contains("stub"));
    Violation {
        item_id: item
            .map(|i| i.id.clone())
            .unwrap_or_else(|| "N-mock".into()),
        item_text: item.map(|i| i.text.clone()).unwrap_or_default(),
        why: why.to_string(),
    }
}

/// Return the first mock-family identifier segment that is not in the benign
/// denylist, scanning identifier-like tokens (alphanumeric + `_`).
fn first_mock_like(lower: &str) -> Option<String> {
    for token in lower.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        if token.is_empty() || MOCK_BENIGN.contains(&token) {
            continue;
        }
        if MOCK_ROOTS.iter().any(|root| token.contains(root)) {
            return Some(token.to_string());
        }
    }
    None
}

struct CommandHit {
    item_id: &'static str,
    item_text: &'static str,
    why: String,
    hard: bool,
}

/// Tokenized command analysis. High precision: exact flags / command sequences,
/// no bare `skip` / `disable`. Only verification-defeating commands are `hard`.
fn command_hits(cmd: &str) -> Vec<CommandHit> {
    let toks: Vec<String> = cmd
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '|' | '&'))
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();
    let has = |t: &str| toks.iter().any(|x| x == t);
    let mut hits = Vec::new();

    if has("--no-verify") {
        hits.push(CommandHit {
            item_id: "N-test",
            item_text: "must not skip/weaken verification protecting contract items",
            why: "command skips verification hooks (--no-verify)".into(),
            hard: true,
        });
    }
    if toks.iter().any(|t| t == "xfail") {
        hits.push(CommandHit {
            item_id: "N-test",
            item_text: "must not skip/weaken verification protecting contract items",
            why: "command marks tests as expected-fail (xfail)".into(),
            hard: true,
        });
    }
    // Destructive but sometimes legitimate (e.g. `rm -rf build/`): disclosure, not hard.
    let rm_force = toks.iter().any(|t| {
        let f = t.trim_start_matches('-');
        t.starts_with('-') && f.contains('r') && f.contains('f')
    });
    if has("rm") && (rm_force || has("--recursive")) {
        hits.push(CommandHit {
            item_id: "N-hide",
            item_text: "must not destroy work without disclosure",
            why: "recursive force delete (rm -rf)".into(),
            hard: false,
        });
    }
    if has("git") && has("reset") && has("--hard") {
        hits.push(CommandHit {
            item_id: "N-hide",
            item_text: "must not destroy work without disclosure",
            why: "discards working changes (git reset --hard)".into(),
            hard: false,
        });
    }
    if has("git") && has("push") && (has("--force") || has("-f")) {
        // --force-with-lease is the safe form and is not flagged.
        hits.push(CommandHit {
            item_id: "N-hide",
            item_text: "must not destroy work without disclosure",
            why: "force pushes / rewrites history (git push --force)".into(),
            hard: false,
        });
    }
    hits
}

/// Out-of-scope hit: glob entries match via the path matcher; plain entries
/// match as a path-segment prefix (boundary aware), not a raw substring.
fn out_of_scope_hit(entry: &str, path: &str) -> bool {
    if entry.contains('*') || entry.contains('?') {
        return path_matches_glob(entry, path);
    }
    let np = super::path_match::normalize_rel_path(path);
    let ne = super::path_match::normalize_rel_path(entry);
    let np = np.trim_start_matches('/');
    let ne = ne.trim_start_matches('/');
    if ne.is_empty() {
        return false;
    }
    // segment-boundary prefix: `src/legacy` matches `src/legacy/x.rs` but not `src/legacyx`.
    np == ne || np.starts_with(&format!("{ne}/")) || np.split('/').any(|seg| seg == ne)
}

fn looks_like_mass_deletion(action: &ProposedAction) -> bool {
    let body = match action.content_or_diff.as_deref() {
        Some(b) => b,
        None => return false,
    };
    let is_unified_diff = body
        .lines()
        .any(|l| l.starts_with("@@") || l.starts_with("--- ") || l.starts_with("+++ "));
    if is_unified_diff {
        let mut minus = 0usize;
        let mut plus = 0usize;
        for l in body.lines() {
            if l.starts_with('-') && !l.starts_with("---") {
                minus += 1;
            } else if l.starts_with('+') && !l.starts_with("+++") {
                plus += 1;
            }
        }
        return minus >= 15 && minus > plus * 3;
    }
    // Edit path: compare prior content line count to the replacement.
    if let Some(prior) = action.prior_content.as_deref() {
        let old_lines = prior.lines().count();
        let new_lines = body.lines().count();
        return old_lines.saturating_sub(new_lines) >= 15
            && old_lines > new_lines.saturating_mul(3);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::atlas_harness::goal_contract::GoalContract;

    fn contract_with_preserve() -> GoalContract {
        let mut c = GoalContract::default();
        c.preserve.push(super::super::goal_contract::PreserveItem {
            id: "P1".into(),
            text: "keep UI layout".into(),
            kind: PreserveKind::LayoutStructure,
            path_glob: Some("src/ui/**".into()),
        });
        c.must_not_do
            .push(super::super::goal_contract::ContractItem {
                id: "N-mock".into(),
                text: "no mock replacing real impl".into(),
                hard: true,
                source_quote: None,
                verify: None,
            });
        c
    }

    #[test]
    fn preserve_block_survives_path_obfuscation() {
        let c = contract_with_preserve();
        for p in [
            "src/ui/App.tsx",
            "./src/ui/App.tsx",
            "/abs/root/src/ui/App.tsx",
            "src\\ui\\App.tsx",
        ] {
            let a = ProposedAction {
                kind_raw: "edit_file".into(),
                target_path: Some(p.into()),
                ..Default::default()
            };
            assert!(
                evaluate(&a, &c).is_block(),
                "should block obfuscated path {p}"
            );
        }
    }

    #[test]
    fn unknown_named_write_tool_is_still_gated() {
        // Tool name does not look mutating, but args are writeful → must be gated.
        let c = contract_with_preserve();
        let a = ProposedAction {
            kind_raw: "mcp_filesystem_apply".into(), // classifies as Other
            target_path: Some("src/ui/App.tsx".into()),
            content_or_diff: Some("export const x = 1".into()),
            ..Default::default()
        };
        assert!(a.is_mutating());
        assert!(evaluate(&a, &c).is_block());
    }

    #[test]
    fn skiplibcheck_is_not_flagged() {
        let c = contract_with_preserve();
        let a = ProposedAction {
            kind_raw: "bash".into(),
            command: Some("tsc --skipLibCheck && npm run build".into()),
            ..Default::default()
        };
        assert_eq!(evaluate(&a, &c), ContractDecision::Allow);
    }

    #[test]
    fn no_verify_is_a_hard_block() {
        let c = contract_with_preserve();
        let a = ProposedAction {
            kind_raw: "bash".into(),
            command: Some("git commit --no-verify -m wip".into()),
            ..Default::default()
        };
        assert!(evaluate(&a, &c).is_block());
    }

    #[test]
    fn mockup_does_not_trip_but_mock_user_does() {
        let c = contract_with_preserve();
        let benign = ProposedAction {
            kind_raw: "write".into(),
            target_path: Some("src/x.ts".into()),
            content_or_diff: Some("// render the mockup preview".into()),
            ..Default::default()
        };
        assert_eq!(evaluate(&benign, &c), ContractDecision::Allow);

        let mocky = ProposedAction {
            kind_raw: "write".into(),
            target_path: Some("src/x.ts".into()),
            content_or_diff: Some("const user = mockUser();".into()),
            ..Default::default()
        };
        assert!(matches!(
            evaluate(&mocky, &c),
            ContractDecision::RequireDisclosure { .. }
        ));
    }

    #[test]
    fn mass_deletion_detected_on_edit_via_prior_content() {
        let c = contract_with_preserve();
        let prior = (0..40)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let a = ProposedAction {
            kind_raw: "str_replace".into(),
            target_path: Some("src/service.rs".into()),
            command: None,
            content_or_diff: Some("// removed".into()),
            prior_content: Some(prior),
        };
        assert!(matches!(
            evaluate(&a, &c),
            ContractDecision::RequireDisclosure { .. }
        ));
    }

    #[test]
    fn read_only_is_allowed() {
        let c = contract_with_preserve();
        let a = ProposedAction {
            kind_raw: "read_file".into(),
            target_path: Some("src/ui/App.tsx".into()),
            ..Default::default()
        };
        assert!(!a.is_mutating());
        assert_eq!(evaluate(&a, &c), ContractDecision::Allow);
    }
}
