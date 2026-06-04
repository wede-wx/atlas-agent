use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::storage::BrowserAgentStepRecord;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDomSummary {
    pub total_elements: i64,
    pub interactive_elements: i64,
    pub links: i64,
    pub iframes: i64,
    pub shadow_roots: i64,
    pub images: i64,
    pub text_chars: i64,
    pub empty_page: bool,
    pub skeleton_like: bool,
    pub truncated_elements: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserLoopVerdict {
    pub repeated: bool,
    pub repeat_count: usize,
    pub threshold: usize,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserJudgeResult {
    pub status: String,
    pub reason: String,
    pub blocks_completion: bool,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserActionFingerprintInput {
    pub action: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub dom_summary: Option<BrowserDomSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDownloadTrace {
    pub url: String,
    #[serde(default)]
    pub suggested_filename: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserHarEntry {
    pub url: String,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub status: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTraceExtensionInput {
    #[serde(default)]
    pub page_url: Option<String>,
    #[serde(default)]
    pub downloads: Vec<BrowserDownloadTrace>,
    #[serde(default)]
    pub har_entries: Vec<BrowserHarEntry>,
    #[serde(default)]
    pub captcha_detected: bool,
    #[serde(default)]
    pub domain_policy_violations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTraceExtensionReport {
    pub status: String,
    pub blocks_completion: bool,
    pub download_count: usize,
    pub unresolved_downloads: usize,
    pub network_error_count: usize,
    pub captcha_detected: bool,
    pub cross_domain_hosts: Vec<String>,
    pub evidence: Vec<String>,
    pub required_followups: Vec<String>,
}

pub fn browser_action_fingerprint(input: &BrowserActionFingerprintInput) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize_piece(&input.action).as_bytes());
    hasher.update(b"\0");
    hasher.update(normalize_piece(input.target.as_deref().unwrap_or_default()).as_bytes());
    hasher.update(b"\0");
    hasher.update(normalize_url(input.url.as_deref().unwrap_or_default()).as_bytes());
    hasher.update(b"\0");
    hasher.update(normalize_piece(input.selector.as_deref().unwrap_or_default()).as_bytes());
    hasher.update(b"\0");
    hasher.update(normalize_piece(input.key.as_deref().unwrap_or_default()).as_bytes());
    hasher.update(b"\0");
    if matches!(input.action.trim(), "type") {
        // The exact typed text can contain user content. Keep only a bounded shape.
        hasher.update(input.text.as_deref().unwrap_or_default().len().to_string());
    }
    hasher.update(b"\0");
    if let Some(dom) = &input.dom_summary {
        hasher.update(dom.total_elements.to_string());
        hasher.update(b":");
        hasher.update(dom.interactive_elements.to_string());
        hasher.update(b":");
        hasher.update(dom.links.to_string());
        hasher.update(b":");
        hasher.update((dom.empty_page as u8).to_string());
    }
    format!("{:x}", hasher.finalize())
}

pub fn analyze_browser_trace_extension(
    input: &BrowserTraceExtensionInput,
) -> BrowserTraceExtensionReport {
    let page_host = input.page_url.as_deref().and_then(host_from_url);
    let mut hosts = BTreeSet::new();
    let mut evidence = Vec::new();
    let mut required_followups = Vec::new();
    let unresolved_downloads = input
        .downloads
        .iter()
        .filter(|download| {
            !matches!(
                download.status.as_deref().unwrap_or(""),
                "completed" | "succeeded" | "saved"
            ) || download.path.as_deref().unwrap_or("").trim().is_empty()
        })
        .count();
    if !input.downloads.is_empty() {
        evidence.push(format!(
            "downloads={} unresolved={}",
            input.downloads.len(),
            unresolved_downloads
        ));
    }
    if unresolved_downloads > 0 {
        required_followups
            .push("resolve or explicitly discard pending browser downloads".to_string());
    }

    let mut network_error_count = 0usize;
    for entry in &input.har_entries {
        if let Some(host) = host_from_url(&entry.url) {
            if page_host.as_deref() != Some(host.as_str()) {
                hosts.insert(host);
            }
        }
        let failed_status = entry.status.is_some_and(|status| status >= 400);
        let has_error = !entry.error.as_deref().unwrap_or("").trim().is_empty();
        if failed_status || has_error {
            network_error_count += 1;
        }
    }
    if !input.har_entries.is_empty() {
        evidence.push(format!(
            "harEntries={} networkErrors={network_error_count}",
            input.har_entries.len()
        ));
    }
    if network_error_count > 0 {
        required_followups
            .push("inspect failed HAR entries before claiming browser success".to_string());
    }
    if input.captcha_detected {
        evidence.push("captchaDetected=true".to_string());
        required_followups
            .push("captcha requires explicit human/browser-state handling".to_string());
    }
    for violation in &input.domain_policy_violations {
        evidence.push(format!(
            "domainPolicyViolation={}",
            trim_for_evidence(violation, 160)
        ));
    }
    if !input.domain_policy_violations.is_empty() {
        required_followups.push("resolve browser domain policy violations".to_string());
    }

    let cross_domain_hosts = hosts.into_iter().collect::<Vec<_>>();
    if !cross_domain_hosts.is_empty() {
        evidence.push(format!("crossDomainHosts={}", cross_domain_hosts.join(",")));
    }
    let blocks_completion = unresolved_downloads > 0
        || network_error_count > 0
        || input.captcha_detected
        || !input.domain_policy_violations.is_empty();
    BrowserTraceExtensionReport {
        status: if blocks_completion {
            "needs_review".to_string()
        } else {
            "observed".to_string()
        },
        blocks_completion,
        download_count: input.downloads.len(),
        unresolved_downloads,
        network_error_count,
        captcha_detected: input.captcha_detected,
        cross_domain_hosts,
        evidence,
        required_followups,
    }
}

pub fn detect_browser_action_loop(
    recent_steps: &[BrowserAgentStepRecord],
    fingerprint: &str,
    threshold: usize,
) -> BrowserLoopVerdict {
    let threshold = threshold.max(2);
    let repeat_count = recent_steps
        .iter()
        .rev()
        .take_while(|step| step.fingerprint == fingerprint)
        .count()
        + 1;
    let repeated = repeat_count >= threshold;
    BrowserLoopVerdict {
        repeated,
        repeat_count,
        threshold,
        reason: repeated
            .then(|| format!("same browser action/page fingerprint repeated {repeat_count} times")),
    }
}

pub fn judge_browser_step(
    ok: bool,
    action: &str,
    url: Option<&str>,
    dom_summary: Option<&BrowserDomSummary>,
    loop_verdict: &BrowserLoopVerdict,
    error: Option<&str>,
) -> BrowserJudgeResult {
    let mut evidence = Vec::new();
    if let Some(url) = url.filter(|u| !u.trim().is_empty()) {
        evidence.push(format!("url={}", trim_for_evidence(url, 160)));
    }
    if let Some(dom) = dom_summary {
        evidence.push(format!(
            "dom total={} interactive={} links={} textChars={}",
            dom.total_elements, dom.interactive_elements, dom.links, dom.text_chars
        ));
    }
    if let Some(error) = error.filter(|e| !e.trim().is_empty()) {
        evidence.push(format!("error={}", trim_for_evidence(error, 160)));
    }

    if loop_verdict.repeated {
        return BrowserJudgeResult {
            status: "blocked".to_string(),
            reason: loop_verdict
                .reason
                .clone()
                .unwrap_or_else(|| "browser action loop detected".to_string()),
            blocks_completion: true,
            evidence,
        };
    }
    if !ok {
        return BrowserJudgeResult {
            status: "failed".to_string(),
            reason: error
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("browser action failed")
                .to_string(),
            blocks_completion: true,
            evidence,
        };
    }
    if dom_summary.is_some_and(|dom| dom.empty_page || dom.skeleton_like) {
        return BrowserJudgeResult {
            status: "uncertain".to_string(),
            reason: "browser page rendered, but DOM summary looks empty or skeleton-like"
                .to_string(),
            blocks_completion: true,
            evidence,
        };
    }
    BrowserJudgeResult {
        status: "observed".to_string(),
        reason: format!("{action} completed with observable browser state"),
        blocks_completion: false,
        evidence,
    }
}

fn normalize_piece(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn normalize_url(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    value
        .split('#')
        .next()
        .unwrap_or("")
        .trim_end_matches('/')
        .to_string()
}

fn host_from_url(value: &str) -> Option<String> {
    let value = value.trim();
    let after_scheme = value
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(value);
    let host = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .split('@')
        .next_back()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

fn trim_for_evidence(value: &str, limit: usize) -> String {
    let value = value.trim();
    if value.chars().count() <= limit {
        value.to_string()
    } else {
        let mut trimmed = value.chars().take(limit).collect::<String>();
        trimmed.push('…');
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(fingerprint: &str) -> BrowserAgentStepRecord {
        BrowserAgentStepRecord {
            id: "s".to_string(),
            session_id: None,
            run_id: Some("r".to_string()),
            step_index: 1,
            action: "click".to_string(),
            target: None,
            status: "ready".to_string(),
            title: None,
            url: Some("https://example.com".to_string()),
            screenshot_path: None,
            dom_summary: serde_json::json!({}),
            action_json: serde_json::json!({}),
            result_json: serde_json::json!({}),
            fingerprint: fingerprint.to_string(),
            judge: serde_json::json!({}),
            loop_detected: false,
            created_at: 1,
        }
    }

    #[test]
    fn fingerprint_ignores_url_fragments_and_secret_text() {
        let base = BrowserActionFingerprintInput {
            action: "type".to_string(),
            target: None,
            url: Some("https://Example.com/page#top".to_string()),
            selector: Some("#q".to_string()),
            key: None,
            text: Some("secret value one".to_string()),
            dom_summary: Some(BrowserDomSummary {
                total_elements: 10,
                interactive_elements: 2,
                links: 1,
                ..Default::default()
            }),
        };
        let mut other = base.clone();
        other.url = Some("https://example.com/page".to_string());
        other.text = Some("secret value two".to_string());
        assert_eq!(
            browser_action_fingerprint(&base),
            browser_action_fingerprint(&other),
            "same text length should not hash exact typed content"
        );
    }

    #[test]
    fn loop_detector_blocks_repeated_consecutive_fingerprints() {
        let verdict = detect_browser_action_loop(&[step("a"), step("a")], "a", 3);
        assert!(verdict.repeated);
        assert_eq!(verdict.repeat_count, 3);
        let ok = detect_browser_action_loop(&[step("a"), step("b")], "a", 3);
        assert!(!ok.repeated);
    }

    #[test]
    fn judge_blocks_empty_dom_and_loop() {
        let dom = BrowserDomSummary {
            empty_page: true,
            ..Default::default()
        };
        let loop_verdict = BrowserLoopVerdict {
            repeated: false,
            repeat_count: 1,
            threshold: 3,
            reason: None,
        };
        let judged = judge_browser_step(
            true,
            "open",
            Some("https://x.test"),
            Some(&dom),
            &loop_verdict,
            None,
        );
        assert_eq!(judged.status, "uncertain");
        assert!(judged.blocks_completion);

        let looped = BrowserLoopVerdict {
            repeated: true,
            repeat_count: 3,
            threshold: 3,
            reason: Some("loop".to_string()),
        };
        let judged = judge_browser_step(true, "click", None, None, &looped, None);
        assert_eq!(judged.status, "blocked");
        assert!(judged.blocks_completion);
    }

    #[test]
    fn trace_extension_blocks_unresolved_downloads_network_errors_and_captcha() {
        let report = analyze_browser_trace_extension(&BrowserTraceExtensionInput {
            page_url: Some("https://app.example.test/work".to_string()),
            downloads: vec![BrowserDownloadTrace {
                url: "https://cdn.example.test/export.zip".to_string(),
                suggested_filename: Some("export.zip".to_string()),
                path: None,
                status: Some("started".to_string()),
            }],
            har_entries: vec![BrowserHarEntry {
                url: "https://api.example.test/data".to_string(),
                method: Some("GET".to_string()),
                status: Some(500),
                error: None,
            }],
            captcha_detected: true,
            domain_policy_violations: vec!["blocked private network".to_string()],
        });
        assert_eq!(report.status, "needs_review");
        assert!(report.blocks_completion);
        assert_eq!(report.unresolved_downloads, 1);
        assert_eq!(report.network_error_count, 1);
        assert!(report
            .cross_domain_hosts
            .contains(&"api.example.test".to_string()));
        assert!(report
            .required_followups
            .iter()
            .any(|item| item.contains("captcha")));
    }
}
