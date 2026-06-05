//! HTTP capability probes.
//!
//! `probe_endpoint` pings model-list endpoints and can only justify
//! `source=probed`. `probe_capability_dry_run` sends minimal chat completion
//! requests that exercise tool, vision, and JSON-mode inputs; conclusive dry-run
//! results can justify `source=verified`.
//!
//! Timeout: hard 5 seconds per request. On transport failures the caller MUST
//! preserve the existing/builtin capabilities instead of pretending the probe
//! proved anything.

use crate::agent::capabilities::ProviderCapabilities;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use thiserror::Error;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointProbe {
    /// Endpoint responded 2xx within timeout.
    pub reachable: bool,
    /// Model IDs returned by the provider. Empty if endpoint returns nothing.
    pub models: Vec<String>,
    /// Set when the queried model is found in `models`. UI can use this to
    /// decide whether to set source=`probed`.
    pub queried_model_found: bool,
    /// Short human-readable line for logs / UI.
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDryRunReport {
    pub attempted: bool,
    pub protocol: String,
    pub verified: bool,
    pub vision: Option<bool>,
    pub tool_calls: Option<bool>,
    pub json_mode: Option<bool>,
    pub checks: Vec<CapabilityDryRunCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDryRunCheck {
    pub capability: String,
    pub attempted: bool,
    pub supported: Option<bool>,
    pub conclusive: bool,
    pub status: Option<u16>,
    pub detail: String,
}

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("probe timed out after 5s")]
    Timeout,
    #[error("probe http error: {0}")]
    Http(String),
    #[error("probe response parse error: {0}")]
    Parse(String),
    #[error("unsupported protocol for probing: {0}")]
    UnsupportedProtocol(String),
}

#[derive(Debug, Clone)]
pub struct ProbeInput {
    pub protocol: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub auth_header: Option<String>,
}

impl CapabilityDryRunReport {
    fn from_checks(protocol: String, checks: Vec<CapabilityDryRunCheck>) -> Self {
        let attempted = checks.iter().any(|check| check.attempted);
        let verified = attempted && checks.iter().all(|check| check.conclusive);
        Self {
            attempted,
            protocol,
            verified,
            vision: capability_value(&checks, "vision"),
            tool_calls: capability_value(&checks, "tool_calls"),
            json_mode: capability_value(&checks, "json_mode"),
            checks,
        }
    }

    fn unsupported(protocol: String, detail: &str, baseline: &ProviderCapabilities) -> Self {
        Self {
            attempted: false,
            protocol,
            verified: false,
            vision: Some(baseline.vision),
            tool_calls: Some(baseline.tool_calls),
            json_mode: Some(baseline.json_mode),
            checks: vec![CapabilityDryRunCheck {
                capability: "dry_run".to_string(),
                attempted: false,
                supported: None,
                conclusive: false,
                status: None,
                detail: detail.to_string(),
            }],
        }
    }
}

fn capability_value(checks: &[CapabilityDryRunCheck], capability: &str) -> Option<bool> {
    checks
        .iter()
        .find(|check| check.capability == capability && check.conclusive)
        .and_then(|check| check.supported)
}

pub async fn probe_endpoint(input: &ProbeInput) -> Result<EndpointProbe, ProbeError> {
    match input.protocol.as_str() {
        "openai" | "openai-compatible" | "deepseek" | "qwen" | "xiaomi-mimo" => {
            probe_openai_compatible(input).await
        }
        "ollama" => probe_ollama(input).await,
        "anthropic" => Ok(EndpointProbe {
            reachable: false,
            models: Vec::new(),
            queried_model_found: false,
            detail: "anthropic has no public model-list endpoint; relying on builtin".into(),
        }),
        other => Err(ProbeError::UnsupportedProtocol(other.to_string())),
    }
}

pub async fn probe_capability_dry_run(
    input: &ProbeInput,
    baseline: &ProviderCapabilities,
) -> CapabilityDryRunReport {
    match input.protocol.as_str() {
        "openai" | "openai-compatible" | "deepseek" | "qwen" | "xiaomi-mimo" => {
            probe_openai_compatible_dry_run(input).await
        }
        "anthropic" => probe_anthropic_dry_run(input).await,
        "ollama" => probe_ollama_dry_run(input).await,
        other => CapabilityDryRunReport::unsupported(
            other.to_string(),
            "unsupported protocol for capability dry-run",
            baseline,
        ),
    }
}

/// P0-3: provider-API outbound boundary for the probe path. The provider
/// channel allows loopback (local model servers); only malformed or
/// non-http(s) endpoints are refused.
fn guard_provider_probe(url: &str) -> Result<(), ProbeError> {
    if let crate::tools::outbound::OutboundDecision::Deny { reason } =
        crate::tools::outbound::active_policy().evaluate_url(
            crate::tools::outbound::OutboundChannel::ProviderApi,
            url,
            &[],
        )
    {
        return Err(ProbeError::Http(reason));
    }
    Ok(())
}

fn apply_probe_auth(
    request: reqwest::RequestBuilder,
    input: &ProbeInput,
) -> reqwest::RequestBuilder {
    if input.api_key.trim().is_empty() {
        return request;
    }
    let header_name = input
        .auth_header
        .clone()
        .unwrap_or_else(|| "Authorization".into());
    let header_val = if header_name.eq_ignore_ascii_case("authorization") {
        format!("Bearer {}", input.api_key)
    } else {
        input.api_key.clone()
    };
    request.header(&header_name, &header_val)
}

async fn probe_openai_compatible(input: &ProbeInput) -> Result<EndpointProbe, ProbeError> {
    let url = format!("{}/models", input.base_url.trim_end_matches('/'));
    guard_provider_probe(&url)?;
    let client = Client::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
        .map_err(|e| ProbeError::Http(e.to_string()))?;

    let resp = apply_probe_auth(client.get(&url), input)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                ProbeError::Timeout
            } else {
                ProbeError::Http(e.to_string())
            }
        })?;

    let status = resp.status();
    if !status.is_success() {
        return Ok(EndpointProbe {
            reachable: false,
            models: Vec::new(),
            queried_model_found: false,
            detail: format!("HTTP {} from {}", status.as_u16(), url),
        });
    }

    let body: OpenAIModelsResponse = resp
        .json()
        .await
        .map_err(|e| ProbeError::Parse(e.to_string()))?;

    let models: Vec<String> = body.data.into_iter().map(|m| m.id).collect();
    let queried_model_found = models.iter().any(|m| m == &input.model);
    let detail = format!("{} models returned", models.len());

    Ok(EndpointProbe {
        reachable: true,
        models,
        queried_model_found,
        detail,
    })
}

async fn probe_ollama(input: &ProbeInput) -> Result<EndpointProbe, ProbeError> {
    let url = format!("{}/api/tags", input.base_url.trim_end_matches('/'));
    guard_provider_probe(&url)?;
    let client = Client::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
        .map_err(|e| ProbeError::Http(e.to_string()))?;

    let resp = client.get(&url).send().await.map_err(|e| {
        if e.is_timeout() {
            ProbeError::Timeout
        } else {
            ProbeError::Http(e.to_string())
        }
    })?;

    let status = resp.status();
    if !status.is_success() {
        return Ok(EndpointProbe {
            reachable: false,
            models: Vec::new(),
            queried_model_found: false,
            detail: format!("HTTP {} from {}", status.as_u16(), url),
        });
    }

    let body: OllamaTagsResponse = resp
        .json()
        .await
        .map_err(|e| ProbeError::Parse(e.to_string()))?;

    let models: Vec<String> = body.models.into_iter().map(|m| m.name).collect();
    let queried_model_found = models.iter().any(|m| m == &input.model);
    let detail = format!("{} ollama models", models.len());

    Ok(EndpointProbe {
        reachable: true,
        models,
        queried_model_found,
        detail,
    })
}

async fn probe_openai_compatible_dry_run(input: &ProbeInput) -> CapabilityDryRunReport {
    let url = format!("{}/chat/completions", input.base_url.trim_end_matches('/'));
    let client = match dry_run_client(&url) {
        Ok(client) => client,
        Err(check) => {
            return CapabilityDryRunReport::from_checks(
                input.protocol.clone(),
                vec![
                    check.clone_for_capability("tool_calls"),
                    check.clone_for_capability("vision"),
                    check.clone_for_capability("json_mode"),
                ],
            )
        }
    };

    let checks = vec![
        run_chat_dry_run_check(
            &client,
            input,
            &url,
            "tool_calls",
            json!({
                "model": input.model,
                "messages": [{ "role": "user", "content": "Say ok." }],
                "max_tokens": 1,
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "atlas_probe_tool",
                        "description": "Capability dry-run probe.",
                        "parameters": { "type": "object", "properties": {} }
                    }
                }]
            }),
        )
        .await,
        run_chat_dry_run_check(
            &client,
            input,
            &url,
            "vision",
            json!({
                "model": input.model,
                "messages": [{
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Describe this image in one word." },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:image/png;base64,{}", probe_png_base64())
                            }
                        }
                    ]
                }],
                "max_tokens": 1
            }),
        )
        .await,
        run_chat_dry_run_check(
            &client,
            input,
            &url,
            "json_mode",
            json!({
                "model": input.model,
                "messages": [
                    { "role": "system", "content": "Return a JSON object." },
                    { "role": "user", "content": "Return {\"ok\":true}." }
                ],
                "max_tokens": 1,
                "response_format": { "type": "json_object" }
            }),
        )
        .await,
    ];
    CapabilityDryRunReport::from_checks(input.protocol.clone(), checks)
}

async fn probe_anthropic_dry_run(input: &ProbeInput) -> CapabilityDryRunReport {
    let url = format!("{}/messages", input.base_url.trim_end_matches('/'));
    let client = match dry_run_client(&url) {
        Ok(client) => client,
        Err(check) => {
            return CapabilityDryRunReport::from_checks(
                input.protocol.clone(),
                vec![
                    check.clone_for_capability("tool_calls"),
                    check.clone_for_capability("vision"),
                    CapabilityDryRunCheck::protocol_false(
                        "json_mode",
                        "anthropic has no OpenAI-style json_mode request flag",
                    ),
                ],
            )
        }
    };
    let checks = vec![
        run_anthropic_dry_run_check(
            &client,
            input,
            &url,
            "tool_calls",
            json!({
                "model": input.model,
                "messages": [{ "role": "user", "content": "Say ok." }],
                "max_tokens": 1,
                "tools": [{
                    "name": "atlas_probe_tool",
                    "description": "Capability dry-run probe.",
                    "input_schema": { "type": "object", "properties": {} }
                }]
            }),
        )
        .await,
        run_anthropic_dry_run_check(
            &client,
            input,
            &url,
            "vision",
            json!({
                "model": input.model,
                "messages": [{
                    "role": "user",
                    "content": [
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/png",
                                "data": probe_png_base64()
                            }
                        },
                        { "type": "text", "text": "Describe this image in one word." }
                    ]
                }],
                "max_tokens": 1
            }),
        )
        .await,
        CapabilityDryRunCheck::protocol_false(
            "json_mode",
            "anthropic has no OpenAI-style json_mode request flag",
        ),
    ];
    CapabilityDryRunReport::from_checks(input.protocol.clone(), checks)
}

/// Ollama native capability dry-run. Ollama exposes a native `/api/chat`
/// endpoint; modern builds reject `tools` for non-tool models with HTTP 400
/// ("does not support tools"), while `format: "json"` is enforced by the
/// engine regardless of model. Vision support is model-specific and Ollama
/// does not reliably reject image input for text-only models, so we do NOT
/// probe vision here — `vision` stays `None` and the caller preserves the
/// baseline value. Scope matches M-9(c): infer tool/json only.
async fn probe_ollama_dry_run(input: &ProbeInput) -> CapabilityDryRunReport {
    let url = format!("{}/api/chat", input.base_url.trim_end_matches('/'));
    let client = match dry_run_client(&url) {
        Ok(client) => client,
        Err(check) => {
            return CapabilityDryRunReport::from_checks(
                input.protocol.clone(),
                vec![
                    check.clone_for_capability("tool_calls"),
                    check.clone_for_capability("json_mode"),
                ],
            )
        }
    };

    let checks = vec![
        run_chat_dry_run_check(
            &client,
            input,
            &url,
            "tool_calls",
            json!({
                "model": input.model,
                "messages": [{ "role": "user", "content": "Say ok." }],
                "stream": false,
                "options": { "num_predict": 1 },
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "atlas_probe_tool",
                        "description": "Capability dry-run probe.",
                        "parameters": { "type": "object", "properties": {} }
                    }
                }]
            }),
        )
        .await,
        run_chat_dry_run_check(
            &client,
            input,
            &url,
            "json_mode",
            json!({
                "model": input.model,
                "messages": [
                    { "role": "system", "content": "Return a JSON object." },
                    { "role": "user", "content": "Return {\"ok\":true}." }
                ],
                "stream": false,
                "format": "json",
                "options": { "num_predict": 1 }
            }),
        )
        .await,
    ];
    CapabilityDryRunReport::from_checks(input.protocol.clone(), checks)
}

fn dry_run_client(url: &str) -> Result<Client, CapabilityDryRunCheck> {
    if let Err(error) = guard_provider_probe(url) {
        return Err(CapabilityDryRunCheck::inconclusive(
            "dry_run",
            None,
            error.to_string(),
        ));
    }
    Client::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
        .map_err(|error| CapabilityDryRunCheck::inconclusive("dry_run", None, error.to_string()))
}

async fn run_chat_dry_run_check(
    client: &Client,
    input: &ProbeInput,
    url: &str,
    capability: &str,
    body: Value,
) -> CapabilityDryRunCheck {
    let response = apply_probe_auth(client.post(url).json(&body), input)
        .send()
        .await;
    classify_dry_run_response(capability, response).await
}

async fn run_anthropic_dry_run_check(
    client: &Client,
    input: &ProbeInput,
    url: &str,
    capability: &str,
    body: Value,
) -> CapabilityDryRunCheck {
    let response = client
        .post(url)
        .header("x-api-key", &input.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await;
    classify_dry_run_response(capability, response).await
}

async fn classify_dry_run_response(
    capability: &str,
    response: Result<reqwest::Response, reqwest::Error>,
) -> CapabilityDryRunCheck {
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            return CapabilityDryRunCheck::inconclusive(
                capability,
                None,
                if error.is_timeout() {
                    "dry-run timed out".to_string()
                } else {
                    error.to_string()
                },
            )
        }
    };
    let status = response.status();
    if status.is_success() {
        return CapabilityDryRunCheck {
            capability: capability.to_string(),
            attempted: true,
            supported: Some(true),
            conclusive: true,
            status: Some(status.as_u16()),
            detail: "dry-run request accepted".to_string(),
        };
    }
    let detail = response
        .text()
        .await
        .unwrap_or_else(|error| error.to_string());
    if is_capability_unsupported_error(capability, status.as_u16(), &detail) {
        return CapabilityDryRunCheck {
            capability: capability.to_string(),
            attempted: true,
            supported: Some(false),
            conclusive: true,
            status: Some(status.as_u16()),
            detail: truncate_detail(&detail),
        };
    }
    CapabilityDryRunCheck::inconclusive(capability, Some(status.as_u16()), truncate_detail(&detail))
}

impl CapabilityDryRunCheck {
    fn inconclusive(capability: &str, status: Option<u16>, detail: String) -> Self {
        Self {
            capability: capability.to_string(),
            attempted: true,
            supported: None,
            conclusive: false,
            status,
            detail,
        }
    }

    fn protocol_false(capability: &str, detail: &str) -> Self {
        Self {
            capability: capability.to_string(),
            attempted: false,
            supported: Some(false),
            conclusive: true,
            status: None,
            detail: detail.to_string(),
        }
    }

    fn clone_for_capability(&self, capability: &str) -> Self {
        Self {
            capability: capability.to_string(),
            attempted: self.attempted,
            supported: self.supported,
            conclusive: self.conclusive,
            status: self.status,
            detail: self.detail.clone(),
        }
    }
}

fn is_capability_unsupported_error(capability: &str, status: u16, detail: &str) -> bool {
    if !matches!(status, 400 | 404 | 422) {
        return false;
    }
    let lower = detail.to_lowercase();
    let unsupported = [
        "unsupported",
        "not support",
        "not_supported",
        "does not support",
        "is not supported",
        "unknown parameter",
        "unrecognized request argument",
        "invalid type",
        "invalid parameter",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    if !unsupported {
        return false;
    }
    match capability {
        "tool_calls" => ["tool", "function", "tools"]
            .iter()
            .any(|marker| lower.contains(marker)),
        "vision" => ["image", "vision", "multimodal", "content"]
            .iter()
            .any(|marker| lower.contains(marker)),
        "json_mode" => ["response_format", "json", "format"]
            .iter()
            .any(|marker| lower.contains(marker)),
        _ => true,
    }
}

fn truncate_detail(detail: &str) -> String {
    detail.chars().take(400).collect()
}

fn probe_png_base64() -> &'static str {
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII="
}

#[derive(Debug, Deserialize)]
struct OpenAIModelsResponse {
    #[serde(default)]
    data: Vec<OpenAIModelEntry>,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelEntry {
    id: String,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaModelEntry>,
}

#[derive(Debug, Deserialize)]
struct OllamaModelEntry {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::capabilities::{CapabilitySource, ProviderCapabilities};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration as StdDuration;

    #[tokio::test]
    async fn anthropic_protocol_returns_unreachable_without_error() {
        let input = ProbeInput {
            protocol: "anthropic".into(),
            base_url: "https://example.invalid".into(),
            api_key: "ignored".into(),
            model: "claude".into(),
            auth_header: None,
        };
        let res = probe_endpoint(&input).await.unwrap();
        assert!(!res.reachable);
        assert!(!res.queried_model_found);
        assert!(res.detail.contains("anthropic"));
    }

    #[tokio::test]
    async fn unknown_protocol_returns_error() {
        let input = ProbeInput {
            protocol: "made-up".into(),
            base_url: "https://example.invalid".into(),
            api_key: String::new(),
            model: String::new(),
            auth_header: None,
        };
        let err = probe_endpoint(&input).await.unwrap_err();
        match err {
            ProbeError::UnsupportedProtocol(p) => assert_eq!(p, "made-up"),
            other => panic!("expected UnsupportedProtocol, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unreachable_host_yields_unreachable_or_error() {
        // 127.0.0.1:1 is conventionally refused, but corporate proxies may
        // turn the connection into a 5xx response. Accept either shape:
        // an Err (Http/Timeout) or an Ok with reachable=false.
        let input = ProbeInput {
            protocol: "openai-compatible".into(),
            base_url: "http://127.0.0.1:1".into(),
            api_key: "sk-test".into(),
            model: "gpt-4o".into(),
            auth_header: None,
        };
        match probe_endpoint(&input).await {
            Ok(probe) => {
                assert!(!probe.reachable, "unexpected reachable result: {probe:?}");
                assert!(!probe.queried_model_found);
            }
            Err(ProbeError::Http(_)) | Err(ProbeError::Timeout) => {}
            Err(other) => panic!("expected Http/Timeout/unreachable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dry_run_detects_openai_vision_unsupported_without_marking_inconclusive() {
        let base_url = start_openai_dry_run_fixture();
        let input = ProbeInput {
            protocol: "openai-compatible".into(),
            base_url,
            api_key: "sk-test".into(),
            model: "text-only-model".into(),
            auth_header: None,
        };
        let baseline = ProviderCapabilities {
            provider_id: "openai".into(),
            model: "text-only-model".into(),
            vision: true,
            tool_calls: true,
            json_mode: true,
            max_context: 128_000,
            source: CapabilitySource::Builtin,
        };

        let report = probe_capability_dry_run(&input, &baseline).await;

        assert!(report.attempted);
        assert!(report.verified, "unsupported feature errors are conclusive");
        assert_eq!(report.tool_calls, Some(true));
        assert_eq!(report.vision, Some(false));
        assert_eq!(report.json_mode, Some(true));
        let vision = report
            .checks
            .iter()
            .find(|check| check.capability == "vision")
            .unwrap();
        assert_eq!(vision.status, Some(400));
        assert_eq!(vision.supported, Some(false));
        assert!(vision.detail.contains("image"));
    }

    fn start_openai_dry_run_fixture() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let body = read_request_body(&mut stream);
                let (status, response_body) = if body.contains("\"image_url\"") {
                    (
                        "400 Bad Request",
                        r#"{"error":{"message":"This model does not support image input."}}"#,
                    )
                } else {
                    (
                        "200 OK",
                        r#"{"id":"cmpl_probe","choices":[{"message":{"role":"assistant","content":"ok"}}]}"#,
                    )
                };
                write_response(&mut stream, status, response_body);
            }
        });
        format!("http://{}", addr)
    }

    fn read_request_body(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(StdDuration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buf = [0_u8; 1024];
        let mut header_end = None;
        let mut content_length = None;
        loop {
            let read = stream.read(&mut buf).unwrap();
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..read]);
            if header_end.is_none() {
                header_end = find_header_end(&bytes);
                if let Some(end) = header_end {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    content_length = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    });
                }
            }
            if let (Some(end), Some(length)) = (header_end, content_length) {
                if bytes.len().saturating_sub(end + 4) >= length {
                    return String::from_utf8_lossy(&bytes[end + 4..end + 4 + length]).to_string();
                }
            }
        }
        String::new()
    }

    fn find_header_end(bytes: &[u8]) -> Option<usize> {
        bytes.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn write_response(stream: &mut TcpStream, status: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    #[tokio::test]
    async fn ollama_dry_run_detects_tool_unsupported_and_json_supported() {
        let base_url = start_ollama_dry_run_fixture();
        let input = ProbeInput {
            protocol: "ollama".into(),
            base_url,
            api_key: String::new(),
            model: "text-only-model".into(),
            auth_header: None,
        };
        let baseline = ProviderCapabilities {
            provider_id: "ollama".into(),
            model: "text-only-model".into(),
            vision: true,
            tool_calls: true,
            json_mode: false,
            max_context: 8_192,
            source: CapabilitySource::Builtin,
        };

        let report = probe_capability_dry_run(&input, &baseline).await;

        assert!(report.attempted);
        assert!(report.verified, "tool+json conclusive ⇒ verified");
        assert_eq!(report.tool_calls, Some(false));
        assert_eq!(report.json_mode, Some(true));
        assert_eq!(
            report.vision, None,
            "ollama vision is intentionally not probed"
        );
        let tool = report
            .checks
            .iter()
            .find(|check| check.capability == "tool_calls")
            .unwrap();
        assert_eq!(tool.status, Some(400));
        assert_eq!(tool.supported, Some(false));
        assert!(tool.detail.contains("does not support"));
    }

    #[tokio::test]
    async fn ollama_dry_run_unreachable_stays_unverified() {
        let input = ProbeInput {
            protocol: "ollama".into(),
            base_url: "http://127.0.0.1:1".into(),
            api_key: String::new(),
            model: "any".into(),
            auth_header: None,
        };
        let baseline = ProviderCapabilities {
            provider_id: "ollama".into(),
            model: "any".into(),
            vision: false,
            tool_calls: false,
            json_mode: false,
            max_context: 8_192,
            source: CapabilitySource::Builtin,
        };
        let report = probe_capability_dry_run(&input, &baseline).await;
        // Transport failure must not pretend the probe proved anything.
        assert!(!report.verified);
    }

    fn start_ollama_dry_run_fixture() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let body = read_request_body(&mut stream);
                let (status, response_body) = if body.contains("\"tools\"") {
                    (
                        "400 Bad Request",
                        r#"{"error":"\"text-only-model\" does not support tools"}"#,
                    )
                } else {
                    (
                        "200 OK",
                        r#"{"model":"text-only-model","message":{"role":"assistant","content":"{\"ok\":true}"},"done":true}"#,
                    )
                };
                write_response(&mut stream, status, response_body);
            }
        });
        format!("http://{}", addr)
    }
}
