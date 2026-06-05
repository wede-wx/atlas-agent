//! Secret scanning & masking (P0-1 of the agent-principles task plan).
//!
//! Detects high-risk credential material (API keys, tokens, private keys,
//! high-entropy secrets) in text that is about to cross a trust boundary —
//! file writes, command output, commit content, logs, and model context — and
//! either masks it or reports it for the caller to block.
//!
//! Authority basis (doc §10.2 P0-1): GitHub secret-scanning supported patterns,
//! OWASP LLM Top 10. Object shape: `SecretFinding` (doc §8.3). This module is
//! deliberately a pure, side-effect-free classifier (like `command_safety`) so
//! every scan point — `command`, `file_write`, `checkpoint`, `core` context
//! injection, `storage` logging — calls the same logic.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Where the scanned text was about to go. Mirrors `SecretFinding.location`
/// in doc §8.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretLocation {
    FileWrite,
    CommandOutput,
    Commit,
    Log,
    ModelContext,
    /// About to leave the machine over the network (P0-3 outbound boundary).
    Outbound,
}

/// Policy applied at a scan point, and the action recorded on each finding.
/// Mirrors `SecretFinding.action` in doc §8.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretAction {
    /// Redact the secret in the returned text and continue.
    Masked,
    /// Leave the text untouched; the caller refuses the operation.
    Blocked,
    /// Leave the text untouched but flag it for review.
    AllowedWithWarning,
}

/// A single detected secret. Mirrors doc §8.3 `SecretFinding`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretFinding {
    /// Human-facing category, e.g. "OpenAI API key".
    pub kind: String,
    /// Stable rule identifier, e.g. "openai_api_key".
    pub rule_id: String,
    pub location: SecretLocation,
    /// Optional pointer to where it was found (file path, command, ...).
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    pub action: SecretAction,
}

/// Result of a scan: the (possibly redacted) text plus what was found.
#[derive(Debug, Clone)]
pub struct ScanReport {
    /// Redacted text when `policy == Masked` and there were hits; otherwise the
    /// original input unchanged.
    pub text: String,
    pub findings: Vec<SecretFinding>,
}

impl ScanReport {
    /// True when at least one secret was detected.
    pub fn has_secrets(&self) -> bool {
        !self.findings.is_empty()
    }
}

/// Scan `input` for secrets bound for `location`, applying `policy`.
///
/// - `Masked`   → returns text with each secret replaced by `[REDACTED:<rule>]`.
/// - `Blocked` / `AllowedWithWarning` → returns the original text; the caller
///   decides what to do with a non-empty `findings`.
pub fn scan(input: &str, location: SecretLocation, policy: SecretAction) -> ScanReport {
    let mut hits = collect_hits(input);
    // Sort by position, then by rule priority (earlier rule = more specific),
    // and drop hits that overlap an already-kept one so each secret counts once.
    hits.sort_by(|a, b| a.start.cmp(&b.start).then(a.rule_idx.cmp(&b.rule_idx)));
    let mut kept: Vec<Hit> = Vec::new();
    let mut last_end = 0usize;
    for hit in hits {
        if hit.start >= last_end {
            last_end = hit.end;
            kept.push(hit);
        }
    }

    let findings = kept
        .iter()
        .map(|hit| {
            let rule = &rules()[hit.rule_idx];
            SecretFinding {
                kind: rule.kind.to_string(),
                rule_id: rule.rule_id.to_string(),
                location,
                reference: None,
                action: policy,
            }
        })
        .collect();

    let text = if policy == SecretAction::Masked && !kept.is_empty() {
        redact(input, &kept)
    } else {
        input.to_string()
    };

    ScanReport { text, findings }
}

/// Minimum Shannon entropy (bits/char) for the generic "secret-ish assignment"
/// rule to fire. Keeps ordinary placeholder values (`changeme`, repeated chars)
/// from being flagged while catching real high-entropy tokens.
const GENERIC_MIN_ENTROPY: f64 = 3.0;

/// One detected span, before overlap de-duplication.
struct Hit {
    start: usize,
    end: usize,
    rule_idx: usize,
}

struct Rule {
    rule_id: &'static str,
    kind: &'static str,
    re: Regex,
    /// Capture group to redact / measure entropy on (0 = whole match).
    target_group: usize,
    /// When true, only fire if the target group passes the entropy gate.
    entropy_gated: bool,
}

/// Detection rules, in priority order (more specific first). Compiled once.
fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            Rule {
                rule_id: "anthropic_api_key",
                kind: "Anthropic API key",
                re: Regex::new(r"sk-ant-[A-Za-z0-9_-]{20,}").unwrap(),
                target_group: 0,
                entropy_gated: false,
            },
            Rule {
                rule_id: "openai_api_key",
                kind: "OpenAI API key",
                re: Regex::new(r"sk-(?:proj-)?[A-Za-z0-9]{20,}").unwrap(),
                target_group: 0,
                entropy_gated: false,
            },
            Rule {
                rule_id: "aws_access_key_id",
                kind: "AWS access key ID",
                re: Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(),
                target_group: 0,
                entropy_gated: false,
            },
            Rule {
                rule_id: "github_token",
                kind: "GitHub token",
                re: Regex::new(r"gh[pousr]_[A-Za-z0-9]{36,}").unwrap(),
                target_group: 0,
                entropy_gated: false,
            },
            Rule {
                rule_id: "github_fine_grained_pat",
                kind: "GitHub fine-grained PAT",
                re: Regex::new(r"github_pat_[A-Za-z0-9_]{22,}").unwrap(),
                target_group: 0,
                entropy_gated: false,
            },
            Rule {
                rule_id: "google_api_key",
                kind: "Google API key",
                re: Regex::new(r"AIza[0-9A-Za-z_-]{35}").unwrap(),
                target_group: 0,
                entropy_gated: false,
            },
            Rule {
                rule_id: "slack_token",
                kind: "Slack token",
                re: Regex::new(r"xox[baprs]-[A-Za-z0-9-]{10,}").unwrap(),
                target_group: 0,
                entropy_gated: false,
            },
            Rule {
                rule_id: "private_key",
                kind: "Private key block",
                // Match the whole PEM block (BEGIN..END) when an END marker is
                // present so the key body is redacted, not just the header; fall
                // back to the BEGIN line alone for truncated / streamed fragments.
                re: Regex::new(
                    r"(?s)-----BEGIN (?:RSA |EC |OPENSSH |DSA |PGP )?PRIVATE KEY-----(?:.*?-----END (?:RSA |EC |OPENSSH |DSA |PGP )?PRIVATE KEY-----)?",
                )
                .unwrap(),
                target_group: 0,
                entropy_gated: false,
            },
            Rule {
                rule_id: "generic_secret_assignment",
                kind: "High-entropy secret",
                re: Regex::new(
                    r#"(?i)(?:api[_-]?key|secret|token|password|passwd|pwd|access[_-]?key|auth)["']?\s*[:=]\s*["']?([A-Za-z0-9_./+-]{12,})"#,
                )
                .unwrap(),
                target_group: 1,
                entropy_gated: true,
            },
        ]
    })
}

fn collect_hits(input: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (rule_idx, rule) in rules().iter().enumerate() {
        for caps in rule.re.captures_iter(input) {
            let m = match caps.get(rule.target_group) {
                Some(m) => m,
                None => continue,
            };
            if rule.entropy_gated && shannon_entropy(m.as_str()) < GENERIC_MIN_ENTROPY {
                continue;
            }
            hits.push(Hit {
                start: m.start(),
                end: m.end(),
                rule_idx,
            });
        }
    }
    hits
}

/// Rebuild `input` with each kept span replaced by `[REDACTED:<rule_id>]`.
/// `kept` must be sorted ascending by `start` and non-overlapping.
fn redact(input: &str, kept: &[Hit]) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;
    for hit in kept {
        out.push_str(&input[cursor..hit.start]);
        out.push_str(&format!("[REDACTED:{}]", rules()[hit.rule_idx].rule_id));
        cursor = hit.end;
    }
    out.push_str(&input[cursor..]);
    out
}

/// Shannon entropy in bits per character over the byte distribution.
fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts: HashMap<u8, u32> = HashMap::new();
    for b in s.bytes() {
        *counts.entry(b).or_insert(0) += 1;
    }
    let len = s.len() as f64;
    counts
        .values()
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Obvious-fake samples (never real credentials).
    const OPENAI_KEY: &str = concat!("sk", "-abcd1234abcd1234abcd1234abcd1234");
    const AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
    const GITHUB_TOKEN: &str = "ghp_0123456789abcdefghijklmnopqrstuvwxyzABCD";
    const ANTHROPIC_KEY: &str = concat!("sk", "-ant-api03-abcd1234abcd1234abcd1234abcd1234efgh");

    #[test]
    fn detects_and_masks_openai_key() {
        let text = format!("export OPENAI_API_KEY={OPENAI_KEY}");
        let report = scan(&text, SecretLocation::Log, SecretAction::Masked);
        assert_eq!(report.findings.len(), 1, "should find exactly one secret");
        assert_eq!(report.findings[0].rule_id, "openai_api_key");
        assert!(
            !report.text.contains(OPENAI_KEY),
            "masked text must not contain the raw key"
        );
        assert!(report.text.contains("[REDACTED:openai_api_key]"));
    }

    #[test]
    fn detects_aws_access_key() {
        let report = scan(AWS_KEY, SecretLocation::FileWrite, SecretAction::Masked);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].rule_id, "aws_access_key_id");
    }

    #[test]
    fn detects_github_token() {
        let report = scan(GITHUB_TOKEN, SecretLocation::Commit, SecretAction::Blocked);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].rule_id, "github_token");
    }

    #[test]
    fn detects_private_key_block() {
        let text = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...\n";
        let report = scan(text, SecretLocation::FileWrite, SecretAction::Masked);
        assert!(report.has_secrets());
        assert_eq!(report.findings[0].rule_id, "private_key");
    }

    #[test]
    fn masks_entire_private_key_block_when_end_marker_present() {
        let text = "before\n-----BEGIN PRIVATE KEY-----\nMIIBVgIBADANBgkqhkiG9w0BAQEF\nAASCAUEwgg... body lines ...\n-----END PRIVATE KEY-----\nafter";
        let report = scan(text, SecretLocation::FileWrite, SecretAction::Masked);
        assert_eq!(report.findings.len(), 1, "the whole block is one secret");
        assert_eq!(report.findings[0].rule_id, "private_key");
        assert!(
            !report.text.contains("MIIBVgIBAD"),
            "key body must be redacted, not just the header"
        );
        assert!(report.text.contains("[REDACTED:private_key]"));
        assert!(report.text.starts_with("before\n"));
        assert!(report.text.ends_with("after"));
    }

    #[test]
    fn detects_generic_high_entropy_assignment() {
        let text = "db_password = \"S3cr3tPasswordValue123\"";
        let report = scan(text, SecretLocation::FileWrite, SecretAction::Masked);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].rule_id, "generic_secret_assignment");
        assert!(!report.text.contains("S3cr3tPasswordValue123"));
        // the key name stays; only the value is redacted
        assert!(report.text.contains("db_password"));
    }

    #[test]
    fn ignores_normal_prose_and_code() {
        let text = "The quick brown fox jumps over the lazy dog. \
                    Visit https://example.com/docs for details. \
                    let count = items.len(); // number of items";
        let report = scan(text, SecretLocation::ModelContext, SecretAction::Masked);
        assert!(
            report.findings.is_empty(),
            "no secrets expected, got {:?}",
            report.findings
        );
        assert_eq!(report.text, text, "clean text must pass through untouched");
    }

    #[test]
    fn ignores_low_entropy_and_short_values() {
        let low = "password = \"aaaaaaaaaaaaaaaa\""; // 16 chars, entropy ~0
        let short = "token = \"short\""; // below min length
        assert!(scan(low, SecretLocation::FileWrite, SecretAction::Masked)
            .findings
            .is_empty());
        assert!(scan(short, SecretLocation::FileWrite, SecretAction::Masked)
            .findings
            .is_empty());
    }

    #[test]
    fn block_policy_keeps_text_but_reports() {
        let text = format!("aws={AWS_KEY}");
        let report = scan(&text, SecretLocation::Commit, SecretAction::Blocked);
        assert!(report.has_secrets());
        assert_eq!(report.text, text, "block policy must not alter the text");
        assert_eq!(report.findings[0].action, SecretAction::Blocked);
    }

    #[test]
    fn mask_policy_records_masked_action_and_location() {
        let report = scan(AWS_KEY, SecretLocation::CommandOutput, SecretAction::Masked);
        assert_eq!(report.findings[0].action, SecretAction::Masked);
        assert_eq!(report.findings[0].location, SecretLocation::CommandOutput);
    }

    #[test]
    fn counts_multiple_distinct_secrets() {
        let text = format!("a={AWS_KEY} and b={GITHUB_TOKEN}");
        let report = scan(&text, SecretLocation::Log, SecretAction::Masked);
        assert_eq!(report.findings.len(), 2);
    }

    #[test]
    fn anthropic_key_counted_once_not_double_matched() {
        let report = scan(ANTHROPIC_KEY, SecretLocation::Log, SecretAction::Masked);
        assert_eq!(report.findings.len(), 1, "must not double-count sk- prefix");
        assert_eq!(report.findings[0].rule_id, "anthropic_api_key");
    }

    #[test]
    fn finding_serializes_with_doc_schema_shape() {
        let finding = SecretFinding {
            kind: "AWS access key ID".to_string(),
            rule_id: "aws_access_key_id".to_string(),
            location: SecretLocation::FileWrite,
            reference: None,
            action: SecretAction::Masked,
        };
        let json = serde_json::to_string(&finding).unwrap();
        assert!(json.contains("\"location\":\"file_write\""));
        assert!(json.contains("\"action\":\"masked\""));
        assert!(!json.contains("\"ref\""), "None ref must be omitted");

        let with_ref = SecretFinding {
            reference: Some("src/config.rs".to_string()),
            ..finding
        };
        let json = serde_json::to_string(&with_ref).unwrap();
        assert!(json.contains("\"ref\":\"src/config.rs\""));
    }
}
