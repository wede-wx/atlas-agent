//! Outbound network boundary (P0-3 of the agent-principles task plan).
//!
//! Five independent sub-boundaries — `provider API`, `MCP server`,
//! `browser/web`, `telemetry`, `shell network command` — each get their own
//! allow / scope / consent decision, and every outbound payload is screened for
//! secrets before egress (reusing the P0-1 `secret_scan` scanner).
//!
//! Authority basis (doc §10.2 P0-3 / §10.3.2): MCP Security Best Practices,
//! OWASP MCP Top 10, Tauri Capabilities. The card is explicit that a single
//! master switch is wrong, because the channels carry different risk:
//!
//! - **provider API** and **MCP server** MAY target loopback / private hosts —
//!   a local model server (Ollama on `127.0.0.1:11434`) or a local MCP
//!   (`http://127.0.0.1:8765/mcp`) are legitimate.
//! - **web tools** and **telemetry** MUST refuse loopback / private hosts —
//!   that is the SSRF / data-exfiltration surface.
//!
//! So loopback is *allowed* on some channels and *denied* on others. That
//! asymmetry is the whole reason this module classifies per channel instead of
//! flipping one global flag.
//!
//! This module is a pure, side-effect-free policy (like `command_safety` /
//! `secret_scan`); the only impurity is `OutboundAudit::emit`, which writes a
//! host-only (never payload, never secret) trace line to stderr. A queryable
//! decision table is P0-4's job.

use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv6Addr};
use std::sync::RwLock;

use crate::tools::secret_scan::{scan, SecretAction, SecretLocation};

/// The five outbound channels. Each is gated independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundChannel {
    ProviderApi,
    McpServer,
    WebTool,
    Telemetry,
    ShellNetwork,
}

impl OutboundChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProviderApi => "provider_api",
            Self::McpServer => "mcp_server",
            Self::WebTool => "web_tool",
            Self::Telemetry => "telemetry",
            Self::ShellNetwork => "shell_network",
        }
    }

    pub fn label_zh(self) -> &'static str {
        match self {
            Self::ProviderApi => "模型 API",
            Self::McpServer => "MCP 服务",
            Self::WebTool => "网页工具",
            Self::Telemetry => "遥测",
            Self::ShellNetwork => "命令行联网",
        }
    }

    /// Channels that may legitimately reach loopback / private hosts (local
    /// model server, local MCP). Web tools and telemetry never do.
    fn allows_private_host(self) -> bool {
        matches!(self, Self::ProviderApi | Self::McpServer)
    }
}

/// Classification of a target host for SSRF decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostClass {
    Loopback,
    Private,
    LinkLocal,
    Unspecified,
    Public,
}

impl HostClass {
    /// True for any host that a public-web / telemetry channel must never reach.
    pub fn is_internal(self) -> bool {
        !matches!(self, HostClass::Public)
    }
}

/// Classify a host string. Recognizes the special names `localhost`,
/// `*.localhost`, `*.local`, and IPv4 / IPv6 literals (with or without the
/// `[...]` URL brackets). Anything else is treated as a public DNS name.
pub fn classify_host(host: &str) -> HostClass {
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return HostClass::Loopback;
    }
    match host.parse::<IpAddr>() {
        Ok(ip) => classify_ip(ip),
        Err(_) => HostClass::Public,
    }
}

fn classify_ip(ip: IpAddr) -> HostClass {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                HostClass::Loopback
            } else if v4.is_unspecified() {
                HostClass::Unspecified
            } else if v4.is_link_local() {
                HostClass::LinkLocal
            } else if v4.is_private() || v4.is_broadcast() {
                HostClass::Private
            } else {
                HostClass::Public
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                HostClass::Loopback
            } else if v6.is_unspecified() {
                HostClass::Unspecified
            } else if is_unicast_link_local_v6(v6) {
                HostClass::LinkLocal
            } else if is_unique_local_v6(v6) {
                HostClass::Private
            } else {
                HostClass::Public
            }
        }
    }
}

fn is_unique_local_v6(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

fn is_unicast_link_local_v6(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

/// The decision for one outbound attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboundDecision {
    Allow,
    NeedsConsent { reason: String },
    Deny { reason: String },
}

impl OutboundDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Allow => None,
            Self::NeedsConsent { reason } | Self::Deny { reason } => Some(reason),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Per-channel outbound policy. Defaults are permissive-safe and match the
/// card's rollback posture: configured providers / MCP allowed, public web
/// allowed (private denied), telemetry OFF, shell network reviewed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboundPolicy {
    #[serde(default = "default_true")]
    pub provider_api_enabled: bool,
    #[serde(default = "default_true")]
    pub mcp_enabled: bool,
    #[serde(default = "default_true")]
    pub web_enabled: bool,
    /// Extra host-suffix allowlist for the web channel. Empty = any *public*
    /// host allowed (loopback / private still denied). Non-empty = the host
    /// must match an entry.
    #[serde(default)]
    pub web_allowlist: Vec<String>,
    /// Telemetry is OFF by default and never carries file content (see
    /// `telemetry_payload`). Aura ships no telemetry sender today; this is the
    /// guard any future sender must pass through.
    #[serde(default)]
    pub telemetry_enabled: bool,
    /// Shell network commands (`curl`/`wget`/...) require explicit review and
    /// can never be auto-allowlisted.
    #[serde(default = "default_true")]
    pub shell_network_review: bool,
}

impl Default for OutboundPolicy {
    fn default() -> Self {
        Self {
            provider_api_enabled: true,
            mcp_enabled: true,
            web_enabled: true,
            web_allowlist: Vec::new(),
            telemetry_enabled: false,
            shell_network_review: true,
        }
    }
}

impl OutboundPolicy {
    fn channel_enabled(&self, channel: OutboundChannel) -> bool {
        match channel {
            OutboundChannel::ProviderApi => self.provider_api_enabled,
            OutboundChannel::McpServer => self.mcp_enabled,
            OutboundChannel::WebTool => self.web_enabled,
            OutboundChannel::Telemetry => self.telemetry_enabled,
            // Shell network is gated by command review, not a channel switch.
            OutboundChannel::ShellNetwork => true,
        }
    }

    /// Evaluate an outbound HTTP(S) request to `url` on `channel`.
    ///
    /// `allowed_hosts` is a channel-specific host-suffix allowlist (the web
    /// allowlist, configured provider hosts, ...). Empty means "judge by host
    /// class only" — no explicit domain restriction.
    pub fn evaluate_url(
        &self,
        channel: OutboundChannel,
        url: &str,
        allowed_hosts: &[String],
    ) -> OutboundDecision {
        if !self.channel_enabled(channel) {
            return OutboundDecision::Deny {
                reason: format!("{}通道当前已关闭，不允许外发。", channel.label_zh()),
            };
        }
        let parsed = match Url::parse(url.trim()) {
            Ok(parsed) => parsed,
            Err(_) => {
                return OutboundDecision::Deny {
                    reason: format!("出站地址无法解析为合法 URL：{url}"),
                }
            }
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            return OutboundDecision::Deny {
                reason: format!("只允许 http/https 出站，已拒绝协议 `{}`。", parsed.scheme()),
            };
        }
        let Some(host) = parsed.host_str() else {
            return OutboundDecision::Deny {
                reason: "出站地址缺少主机名。".to_string(),
            };
        };
        self.evaluate_host(channel, host, allowed_hosts)
    }

    /// Host-level decision, used when only a host (not a full URL) is known.
    pub fn evaluate_host(
        &self,
        channel: OutboundChannel,
        host: &str,
        allowed_hosts: &[String],
    ) -> OutboundDecision {
        if !self.channel_enabled(channel) {
            return OutboundDecision::Deny {
                reason: format!("{}通道当前已关闭，不允许外发。", channel.label_zh()),
            };
        }
        if classify_host(host).is_internal() && !channel.allows_private_host() {
            return OutboundDecision::Deny {
                reason: format!(
                    "{}通道禁止访问内网/本机地址（{host}），已按 SSRF 规则拒绝。",
                    channel.label_zh()
                ),
            };
        }
        if !allowed_hosts.is_empty() && !host_matches_allowlist(host, allowed_hosts) {
            return OutboundDecision::Deny {
                reason: format!("{host} 不在{}通道的允许域名清单内。", channel.label_zh()),
            };
        }
        OutboundDecision::Allow
    }
}

fn host_matches_allowlist(host: &str, allowed: &[String]) -> bool {
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    allowed.iter().any(|entry| {
        let entry = entry.trim().trim_start_matches('.').to_ascii_lowercase();
        !entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}")))
    })
}

/// Leading executables that make a network request. `command_safety` uses this
/// to force such commands through review — the shell-network sub-boundary.
const NETWORK_COMMAND_HEADS: &[&str] = &[
    "curl",
    "wget",
    "nc",
    "ncat",
    "netcat",
    "telnet",
    "ssh",
    "scp",
    "sftp",
    "ftp",
    "rsync",
    "iwr",
    "irm",
    "invoke-webrequest",
    "invoke-restmethod",
    "start-bitstransfer",
];

/// True when the command's leading executable makes a network request.
/// Path prefixes (`/usr/bin/curl`), quotes, and `.exe`/`.cmd` suffixes are
/// stripped before matching. Multi-space variants are handled by the caller's
/// token split.
pub fn is_network_command(command: &str) -> bool {
    let first = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c| c == '"' || c == '\'');
    let base = first.rsplit(['/', '\\']).next().unwrap_or(first);
    let base = base.to_ascii_lowercase();
    let base = base
        .strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".cmd"))
        .unwrap_or(base.as_str());
    NETWORK_COMMAND_HEADS.contains(&base)
}

/// Result of screening an outbound payload for secrets before egress.
pub struct EgressScreen {
    /// Payload with any detected secret masked (`[REDACTED:<rule>]`).
    pub masked: String,
    /// Number of distinct secrets detected — never the secret itself.
    pub secret_count: usize,
}

/// Screen an outbound payload for secrets before it leaves the machine. Reuses
/// the P0-1 scanner with the `Outbound` location so findings are attributed to
/// the network egress boundary.
pub fn screen_egress(payload: &str) -> EgressScreen {
    let report = scan(payload, SecretLocation::Outbound, SecretAction::Masked);
    EgressScreen {
        secret_count: report.findings.len(),
        masked: report.text,
    }
}

/// Host of `url` for audit lines — never the full path/query (which could carry
/// a secret). Returns `<unparsed>` when the URL can't be parsed.
pub fn audit_target_host(url: &str) -> String {
    Url::parse(url.trim())
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_else(|| "<unparsed>".to_string())
}

/// One auditable outbound attempt: channel + target + summary + decision.
/// The full URL/query is never recorded, only the host (+ optional server id),
/// so the audit line itself can't leak a secret-bearing path.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboundAudit {
    pub channel: OutboundChannel,
    pub target: String,
    pub allowed: bool,
    pub secret_hits: usize,
    pub summary: String,
}

impl OutboundAudit {
    pub fn log_line(&self) -> String {
        format!(
            "[outbound] channel={} target={} allowed={} secret_hits={} :: {}",
            self.channel.as_str(),
            self.target,
            self.allowed,
            self.secret_hits,
            self.summary
        )
    }

    /// Emit a host-only trace line to stderr. Keeps the egress trail visible
    /// until P0-4 lands a queryable decision table.
    pub fn emit(&self) {
        eprintln!("{}", self.log_line());
    }
}

/// Active policy, set from `Config.outbound` at startup and on config save, so
/// every channel (including the zero-config tool boundaries) honors the user's
/// settings. Mirrors the `agent::hooks` global-config pattern. Falls back to
/// the permissive-safe `Default` before it is initialized (e.g. in unit tests).
static ACTIVE_POLICY: RwLock<Option<OutboundPolicy>> = RwLock::new(None);

/// Replace the active outbound policy (call after loading / saving config).
pub fn set_active_policy(policy: OutboundPolicy) {
    if let Ok(mut guard) = ACTIVE_POLICY.write() {
        *guard = Some(policy);
    }
}

/// The active outbound policy, or the safe default if none was set yet.
pub fn active_policy() -> OutboundPolicy {
    ACTIVE_POLICY
        .read()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_loopback_private_and_public_hosts() {
        assert_eq!(classify_host("localhost"), HostClass::Loopback);
        assert_eq!(classify_host("api.localhost"), HostClass::Loopback);
        assert_eq!(classify_host("printer.local"), HostClass::Loopback);
        assert_eq!(classify_host("127.0.0.1"), HostClass::Loopback);
        assert_eq!(classify_host("[::1]"), HostClass::Loopback);
        assert_eq!(classify_host("10.0.0.5"), HostClass::Private);
        assert_eq!(classify_host("192.168.1.2"), HostClass::Private);
        assert_eq!(classify_host("172.16.9.9"), HostClass::Private);
        assert_eq!(classify_host("169.254.1.1"), HostClass::LinkLocal);
        assert_eq!(classify_host("0.0.0.0"), HostClass::Unspecified);
        assert_eq!(classify_host("fc00::1"), HostClass::Private);
        assert_eq!(classify_host("fe80::1"), HostClass::LinkLocal);
        assert_eq!(classify_host("8.8.8.8"), HostClass::Public);
        assert_eq!(classify_host("api.openai.com"), HostClass::Public);
    }

    #[test]
    fn web_channel_denies_loopback_and_private_but_allows_public() {
        let policy = OutboundPolicy::default();
        assert!(matches!(
            policy.evaluate_url(OutboundChannel::WebTool, "http://127.0.0.1:5173/x", &[]),
            OutboundDecision::Deny { .. }
        ));
        assert!(matches!(
            policy.evaluate_url(OutboundChannel::WebTool, "http://192.168.0.10/admin", &[]),
            OutboundDecision::Deny { .. }
        ));
        assert!(policy
            .evaluate_url(OutboundChannel::WebTool, "https://example.com/page", &[])
            .is_allowed());
    }

    #[test]
    fn provider_and_mcp_channels_allow_loopback_for_local_servers() {
        let policy = OutboundPolicy::default();
        // Ollama / LM Studio / local MCP all live on loopback — must be allowed.
        assert!(policy
            .evaluate_url(
                OutboundChannel::ProviderApi,
                "http://127.0.0.1:11434/v1",
                &[]
            )
            .is_allowed());
        assert!(policy
            .evaluate_url(OutboundChannel::McpServer, "http://127.0.0.1:8765/mcp", &[])
            .is_allowed());
    }

    #[test]
    fn non_http_scheme_is_denied_on_every_channel() {
        let policy = OutboundPolicy::default();
        for channel in [
            OutboundChannel::ProviderApi,
            OutboundChannel::McpServer,
            OutboundChannel::WebTool,
        ] {
            assert!(matches!(
                policy.evaluate_url(channel, "file:///C:/secret.txt", &[]),
                OutboundDecision::Deny { .. }
            ));
            assert!(matches!(
                policy.evaluate_url(channel, "ftp://example.com/x", &[]),
                OutboundDecision::Deny { .. }
            ));
        }
    }

    #[test]
    fn unparseable_url_is_denied() {
        let policy = OutboundPolicy::default();
        assert!(matches!(
            policy.evaluate_url(OutboundChannel::WebTool, "not a url", &[]),
            OutboundDecision::Deny { .. }
        ));
    }

    #[test]
    fn web_allowlist_restricts_hosts_when_set() {
        let policy = OutboundPolicy::default();
        let allow = vec!["example.com".to_string()];
        assert!(policy
            .evaluate_url(
                OutboundChannel::WebTool,
                "https://docs.example.com/a",
                &allow
            )
            .is_allowed());
        assert!(policy
            .evaluate_url(OutboundChannel::WebTool, "https://example.com/a", &allow)
            .is_allowed());
        assert!(matches!(
            policy.evaluate_url(OutboundChannel::WebTool, "https://evil.test/a", &allow),
            OutboundDecision::Deny { .. }
        ));
    }

    #[test]
    fn telemetry_is_denied_by_default_and_allowed_only_when_enabled() {
        let off = OutboundPolicy::default();
        assert!(matches!(
            off.evaluate_url(OutboundChannel::Telemetry, "https://t.example.com/e", &[]),
            OutboundDecision::Deny { .. }
        ));
        let on = OutboundPolicy {
            telemetry_enabled: true,
            ..OutboundPolicy::default()
        };
        assert!(on
            .evaluate_url(OutboundChannel::Telemetry, "https://t.example.com/e", &[])
            .is_allowed());
        // Even enabled, telemetry must never reach a private host.
        assert!(matches!(
            on.evaluate_url(OutboundChannel::Telemetry, "http://127.0.0.1/e", &[]),
            OutboundDecision::Deny { .. }
        ));
    }

    #[test]
    fn disabling_a_channel_denies_it_independently() {
        let policy = OutboundPolicy {
            web_enabled: false,
            ..OutboundPolicy::default()
        };
        // Web is off...
        assert!(matches!(
            policy.evaluate_url(OutboundChannel::WebTool, "https://example.com", &[]),
            OutboundDecision::Deny { .. }
        ));
        // ...but the provider channel is unaffected (not one master switch).
        assert!(policy
            .evaluate_url(
                OutboundChannel::ProviderApi,
                "https://api.openai.com/v1",
                &[]
            )
            .is_allowed());
    }

    #[test]
    fn screen_egress_masks_secret_and_counts_it() {
        let payload = "POST body token=sk-ant-AAAAAAAAAAAAAAAAAAAAAAAAA end";
        let screen = screen_egress(payload);
        assert!(screen.secret_count >= 1);
        assert!(!screen.masked.contains("sk-ant-AAAAAAAAAAAAAAAAAAAAAAAAA"));
        assert!(screen.masked.contains("[REDACTED"));
    }

    #[test]
    fn screen_egress_leaves_clean_payload_untouched() {
        let payload = r#"{"name":"echo","arguments":{"message":"hello"}}"#;
        let screen = screen_egress(payload);
        assert_eq!(screen.secret_count, 0);
        assert_eq!(screen.masked, payload);
    }

    #[test]
    fn detects_network_commands() {
        assert!(is_network_command("curl https://example.com -d @secret"));
        assert!(is_network_command("wget http://host/file"));
        assert!(is_network_command("ssh user@host"));
        assert!(is_network_command("Invoke-WebRequest https://x"));
        assert!(is_network_command("iwr https://x"));
        assert!(is_network_command("/usr/bin/curl https://x"));
        assert!(is_network_command("curl.exe https://x"));
        // Not network commands:
        assert!(!is_network_command("ls -la"));
        assert!(!is_network_command("git status"));
        assert!(!is_network_command("cargo build"));
    }

    #[test]
    fn audit_line_carries_channel_target_and_secret_count() {
        let audit = OutboundAudit {
            channel: OutboundChannel::McpServer,
            target: "api.example.com (srv_1)".to_string(),
            allowed: true,
            secret_hits: 0,
            summary: "method=tools/call".to_string(),
        };
        let line = audit.log_line();
        assert!(line.contains("channel=mcp_server"));
        assert!(line.contains("target=api.example.com (srv_1)"));
        assert!(line.contains("allowed=true"));
        assert!(line.contains("secret_hits=0"));
    }

    #[test]
    fn active_policy_defaults_to_safe_before_initialization() {
        // Without set_active_policy, the getter returns the permissive-safe
        // default (telemetry off, web on with SSRF deny).
        let policy = active_policy();
        assert!(!policy.telemetry_enabled);
        assert!(policy.web_enabled);
        assert!(policy.shell_network_review);
    }
}
