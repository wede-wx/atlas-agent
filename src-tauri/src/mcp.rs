use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::time::timeout;
use uuid::Uuid;

use crate::storage::LocalDb;

const MCP_SERVERS_KEY: &str = "mcp_servers";
const MCP_AUDIT_KEY: &str = "mcp_audit_events";
const MCP_AUDIT_LIMIT: usize = 300;
const MCP_PROTOCOL_VERSION: &str = "2025-11-25";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    pub url: Option<String>,
    #[serde(default)]
    pub env: Vec<McpKeyValue>,
    #[serde(default)]
    pub pass_env: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub headers: Vec<McpKeyValue>,
    pub auth_type: Option<String>,
    pub auth_token: Option<String>,
    pub enabled: bool,
    pub risk: String,
    // P1-8: server trust lifecycle. A server is an external black box; it must be
    // explicitly trusted once before any tool call is allowed. Defaults to false so
    // legacy persisted servers (written before this field existed) are treated as
    // untrusted until the user confirms — fail-closed, per the security baseline.
    #[serde(default)]
    pub trusted: bool,
    #[serde(default)]
    pub trusted_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInput {
    pub id: Option<String>,
    pub name: String,
    pub transport: String,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    pub url: Option<String>,
    #[serde(default)]
    pub env: Vec<McpKeyValue>,
    #[serde(default)]
    pub pass_env: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub headers: Vec<McpKeyValue>,
    pub auth_type: Option<String>,
    pub auth_token: Option<String>,
    pub enabled: bool,
    pub risk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpKeyValue {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub name: String,
    pub title: String,
    pub description: String,
    pub read_only: bool,
    pub risk: String,
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatus {
    pub server: McpServerConfig,
    pub status: String,
    pub message: String,
    pub tools: Vec<McpToolInfo>,
    pub resources: Vec<String>,
    pub prompts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpInvokeResult {
    pub server_id: String,
    pub tool_name: String,
    pub status: String,
    pub output: serde_json::Value,
    pub untrusted_external: bool,
    pub requires_confirmation: bool,
    pub server_trusted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAuditEvent {
    pub id: String,
    pub server_id: String,
    pub server_name: String,
    pub tool_name: String,
    pub action: String,
    pub risk: String,
    pub confirmed: bool,
    pub status: String,
    pub reason: String,
    // P1-8: privacy-safe digest of the tool arguments (top-level field names +
    // payload size + secret-hit count). Never stores raw argument values, so the
    // audit trail stays auditable without leaking credentials.
    #[serde(default)]
    pub input_summary: String,
    pub created_at: i64,
}

pub fn load_mcp_servers(db: &LocalDb) -> Result<Vec<McpServerConfig>, String> {
    let stored = db
        .get_app_state(MCP_SERVERS_KEY)
        .map_err(|error| error.to_string())?;
    match stored {
        Some(value) => {
            let mut servers: Vec<McpServerConfig> =
                serde_json::from_value(value).map_err(|error| error.to_string())?;
            servers.retain(|server| !is_builtin_mock_server(server));
            Ok(servers)
        }
        None => Ok(Vec::new()),
    }
}

pub fn save_mcp_server(db: &LocalDb, input: McpServerInput) -> Result<McpServerConfig, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("MCP 服务名称不能为空。".to_string());
    }
    let transport = normalize_transport(&input.transport)?;
    validate_transport_config(&transport, input.command.as_deref(), input.url.as_deref())?;
    let cwd = clean_opt(input.cwd);
    validate_working_directory(cwd.as_deref())?;
    let mut servers = load_mcp_servers(db)?;
    let now = now_millis();
    let id = input
        .id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("mcp_{}", Uuid::new_v4()));
    let existing = servers.iter().find(|server| server.id == id);
    let existing_created_at = existing.map(|server| server.created_at).unwrap_or(now);
    // Trust is a deliberate user decision: preserve it across edits, but never
    // grant it implicitly when a server is first saved.
    let (trusted, trusted_at) = existing
        .map(|server| (server.trusted, server.trusted_at))
        .unwrap_or((false, None));
    let server = McpServerConfig {
        id: id.clone(),
        name: name.to_string(),
        transport,
        command: clean_opt(input.command),
        args: input
            .args
            .into_iter()
            .filter(|arg| !arg.trim().is_empty())
            .map(|arg| arg.trim().to_string())
            .collect(),
        url: clean_opt(input.url),
        env: clean_key_values(input.env),
        pass_env: input
            .pass_env
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        cwd,
        headers: clean_key_values(input.headers),
        auth_type: normalize_auth_type(input.auth_type),
        auth_token: clean_opt(input.auth_token),
        enabled: input.enabled,
        risk: normalize_risk(&input.risk),
        trusted,
        trusted_at,
        created_at: existing_created_at,
        updated_at: now,
        last_status: None,
        last_error: None,
    };
    servers.retain(|item| item.id != id);
    servers.push(server.clone());
    servers.sort_by_key(|a| a.name.to_lowercase());
    store_mcp_servers(db, &servers)?;
    Ok(server)
}

pub fn delete_mcp_server(db: &LocalDb, id: &str) -> Result<(), String> {
    let mut servers = load_mcp_servers(db)?;
    let before = servers.len();
    servers.retain(|server| server.id != id);
    if servers.len() == before {
        return Err("没有找到可删除的 MCP 服务。".to_string());
    }
    store_mcp_servers(db, &servers)
}

pub fn set_mcp_server_status(
    db: &LocalDb,
    id: &str,
    status: Option<String>,
    error: Option<String>,
) -> Result<McpServerConfig, String> {
    let mut servers = load_mcp_servers(db)?;
    let mut updated = None;
    for server in &mut servers {
        if server.id == id {
            server.last_status = status.clone();
            server.last_error = error.clone();
            server.updated_at = now_millis();
            updated = Some(server.clone());
            break;
        }
    }
    store_mcp_servers(db, &servers)?;
    updated.ok_or_else(|| "没有找到 MCP 服务。".to_string())
}

/// P1-8: grant or revoke trust for an MCP server, persisting the decision and
/// writing an audit line. Trust is the gate that `invoke_mcp_tool` checks before
/// any tool call reaches the external server.
pub fn set_mcp_server_trust(
    db: &LocalDb,
    id: &str,
    trusted: bool,
) -> Result<McpServerConfig, String> {
    let mut servers = load_mcp_servers(db)?;
    let mut updated = None;
    for server in &mut servers {
        if server.id == id {
            server.trusted = trusted;
            server.trusted_at = if trusted { Some(now_millis()) } else { None };
            server.updated_at = now_millis();
            updated = Some(server.clone());
            break;
        }
    }
    let server = updated.ok_or_else(|| "没有找到 MCP 服务。".to_string())?;
    store_mcp_servers(db, &servers)?;
    log_mcp_audit(
        db,
        &server,
        "__server__",
        if trusted { "trust" } else { "untrust" },
        &server.risk,
        trusted,
        "ok",
        if trusted {
            "用户已信任该 MCP 服务。"
        } else {
            "用户撤销了该 MCP 服务的信任。"
        },
        "",
    )?;
    Ok(server)
}

pub async fn list_real_mcp_tools(server: &McpServerConfig) -> Result<Vec<McpToolInfo>, String> {
    let response = request_real_mcp(server, "tools/list", json!({}), None).await?;
    Ok(tools_from_mcp_response(&response, server))
}

pub async fn invoke_mcp_tool(
    db: &LocalDb,
    server_id: &str,
    tool_name: &str,
    arguments: serde_json::Value,
    confirmed: bool,
) -> Result<McpInvokeResult, String> {
    let servers = load_mcp_servers(db)?;
    let server = servers
        .iter()
        .find(|server| server.id == server_id)
        .ok_or_else(|| "没有找到 MCP 服务。".to_string())?;
    if !server.enabled {
        return Err("MCP 服务未启用。".to_string());
    }
    let input_summary = summarize_mcp_input(&arguments);
    // P1-8 gate 1: server-level trust. An untrusted external server must never be
    // contacted, so this check runs before tool discovery and before any network
    // call. Fail-closed with a recoverable error that points the caller at trust.
    if !server.trusted {
        log_mcp_audit(
            db,
            server,
            tool_name,
            "invoke",
            &server.risk,
            confirmed,
            "blocked",
            "MCP 服务尚未被信任，请先确认信任该服务。",
            &input_summary,
        )?;
        return Err("MCP 服务尚未被信任，请先确认信任该服务。".to_string());
    }
    let tools = list_real_mcp_tools(server).await?;
    let tool = tools
        .iter()
        .find(|tool| tool.name == tool_name)
        .ok_or_else(|| "这个 MCP 服务尚未暴露该工具。".to_string())?;
    let tool_risk = tool.risk.clone();
    // P1-8 gate 2: per-call confirmation for side-effecting / non-safe tools.
    let requires_confirmation = tool.requires_confirmation || server.risk != "safe";
    if requires_confirmation && !confirmed {
        log_mcp_audit(
            db,
            server,
            tool_name,
            "invoke",
            &tool_risk,
            false,
            "blocked",
            "MCP 工具风险级别要求用户确认。",
            &input_summary,
        )?;
        return Err("MCP 工具风险级别要求用户确认。".to_string());
    }

    let result = request_real_mcp(
        server,
        "tools/call",
        json!({ "name": tool_name, "arguments": arguments }),
        Some(tool_name),
    )
    .await;
    match result {
        Ok(output) => {
            log_mcp_audit(
                db,
                server,
                tool_name,
                "invoke",
                &tool_risk,
                confirmed,
                "ok",
                "真实 MCP 工具调用完成。",
                &input_summary,
            )?;
            Ok(McpInvokeResult {
                server_id: server.id.clone(),
                tool_name: tool_name.to_string(),
                status: "ok".to_string(),
                output,
                untrusted_external: true,
                requires_confirmation,
                server_trusted: server.trusted,
            })
        }
        Err(error) => {
            log_mcp_audit(
                db,
                server,
                tool_name,
                "invoke",
                &tool_risk,
                confirmed,
                "failed",
                &error,
                &input_summary,
            )?;
            Err(error)
        }
    }
}

pub fn list_mcp_audit_events(db: &LocalDb, limit: usize) -> Result<Vec<McpAuditEvent>, String> {
    let stored = db
        .get_app_state(MCP_AUDIT_KEY)
        .map_err(|error| error.to_string())?;
    let mut events: Vec<McpAuditEvent> = match stored {
        Some(value) => serde_json::from_value(value).map_err(|error| error.to_string())?,
        None => Vec::new(),
    };
    events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
    events.truncate(limit.min(MCP_AUDIT_LIMIT));
    Ok(events)
}

pub fn log_mcp_discovery_audit(
    db: &LocalDb,
    server: &McpServerConfig,
    status: &str,
    reason: &str,
) -> Result<(), String> {
    log_mcp_audit(
        db,
        server,
        "__tools_list__",
        "discover",
        &server.risk,
        false,
        status,
        reason,
        "",
    )
}

/// P1-8: build a privacy-safe digest of MCP tool arguments for the audit log.
/// Records top-level field names, payload size, and secret-hit count — never the
/// raw values — so credentials are not persisted into the audit store.
fn summarize_mcp_input(arguments: &serde_json::Value) -> String {
    let serialized = arguments.to_string();
    let secret_count = crate::tools::outbound::screen_egress(&serialized).secret_count;
    let field_part = match arguments.as_object() {
        Some(map) if !map.is_empty() => {
            let mut names: Vec<&str> = map.keys().map(String::as_str).collect();
            names.sort_unstable();
            format!("字段[{}]", names.join(", "))
        }
        _ => "无字段".to_string(),
    };
    format!(
        "{field_part} · {}字符 · {secret_count}处敏感",
        serialized.chars().count()
    )
}

#[allow(clippy::too_many_arguments)]
fn log_mcp_audit(
    db: &LocalDb,
    server: &McpServerConfig,
    tool_name: &str,
    action: &str,
    risk: &str,
    confirmed: bool,
    status: &str,
    reason: &str,
    input_summary: &str,
) -> Result<(), String> {
    let mut events = list_mcp_audit_events(db, MCP_AUDIT_LIMIT)?;
    events.push(McpAuditEvent {
        id: format!("mcp_audit_{}", Uuid::new_v4()),
        server_id: server.id.clone(),
        server_name: server.name.clone(),
        tool_name: tool_name.to_string(),
        action: action.to_string(),
        risk: risk.to_string(),
        confirmed,
        status: status.to_string(),
        reason: reason.to_string(),
        input_summary: input_summary.to_string(),
        created_at: now_millis(),
    });
    events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
    events.truncate(MCP_AUDIT_LIMIT);
    db.set_app_state(MCP_AUDIT_KEY, json!(events))
        .map_err(|error| error.to_string())
}

fn store_mcp_servers(db: &LocalDb, servers: &[McpServerConfig]) -> Result<(), String> {
    db.set_app_state(MCP_SERVERS_KEY, json!(servers))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
fn mock_server_config() -> McpServerConfig {
    let now = now_millis();
    McpServerConfig {
        id: "aura_mock_mcp".to_string(),
        name: "Aura 内置 MCP".to_string(),
        transport: "mock".to_string(),
        command: None,
        args: Vec::new(),
        url: None,
        env: Vec::new(),
        pass_env: Vec::new(),
        cwd: None,
        headers: Vec::new(),
        auth_type: None,
        auth_token: None,
        enabled: true,
        risk: "safe".to_string(),
        trusted: false,
        trusted_at: None,
        created_at: now,
        updated_at: now,
        last_status: Some("ready".to_string()),
        last_error: None,
    }
}

fn is_builtin_mock_server(server: &McpServerConfig) -> bool {
    server.id == "aura_mock_mcp" || server.transport == "mock"
}

fn normalize_transport(value: &str) -> Result<String, String> {
    match value.trim().to_lowercase().as_str() {
        "stdio" => Ok("stdio".to_string()),
        "http" | "sse" | "http_sse" | "http/sse" => Ok("http_sse".to_string()),
        _ => Err("MCP 连接方式只支持 stdio 或 http/sse。".to_string()),
    }
}

fn normalize_risk(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "safe" | "sensitive" | "destructive" => value.trim().to_lowercase(),
        _ => "sensitive".to_string(),
    }
}

fn validate_transport_config(
    transport: &str,
    command: Option<&str>,
    url: Option<&str>,
) -> Result<(), String> {
    if transport == "stdio" && command.map(str::trim).unwrap_or_default().is_empty() {
        return Err("stdio MCP 服务必须填写启动命令。".to_string());
    }
    if transport == "http_sse" && url.map(str::trim).unwrap_or_default().is_empty() {
        return Err("http/sse MCP 服务必须填写 URL。".to_string());
    }
    Ok(())
}

fn validate_working_directory(cwd: Option<&str>) -> Result<(), String> {
    let Some(cwd) = cwd else {
        return Ok(());
    };
    if Path::new(cwd).is_dir() {
        return Ok(());
    }
    Err("MCP 工作目录不存在。".to_string())
}

fn clean_key_values(values: Vec<McpKeyValue>) -> Vec<McpKeyValue> {
    values
        .into_iter()
        .map(|item| McpKeyValue {
            key: item.key.trim().to_string(),
            value: item.value.trim().to_string(),
        })
        .filter(|item| !item.key.is_empty())
        .collect()
}

fn normalize_auth_type(value: Option<String>) -> Option<String> {
    match value
        .as_deref()
        .map(str::trim)
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("bearer") => Some("bearer".to_string()),
        Some("none") | None | Some("") => None,
        _ => None,
    }
}

fn clean_opt(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

async fn request_real_mcp(
    server: &McpServerConfig,
    method: &str,
    params: serde_json::Value,
    name_header: Option<&str>,
) -> Result<serde_json::Value, String> {
    match server.transport.as_str() {
        "stdio" => request_stdio_mcp(server, method, params).await,
        "http_sse" => request_http_mcp(server, method, params, name_header).await,
        other => Err(format!("这个 MCP transport 不支持真实请求：{other}")),
    }
}

async fn request_stdio_mcp(
    server: &McpServerConfig,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let command = server
        .command
        .as_deref()
        .ok_or_else(|| "stdio MCP 缺少启动命令。".to_string())?;
    let mut command_builder = tokio::process::Command::new(command);
    command_builder
        .args(&server.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = server
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        command_builder.current_dir(cwd);
    }
    for item in &server.env {
        command_builder.env(&item.key, &item.value);
    }
    for key in &server.pass_env {
        if let Ok(value) = std::env::var(key) {
            command_builder.env(key, value);
        }
    }
    let mut child = command_builder
        .spawn()
        .map_err(|error| format!("启动 stdio MCP 失败：{error}"))?;

    let initialize_id = 1_i64;
    let request_id = 2_i64;
    let initialize = mcp_initialize_request(initialize_id);
    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    });
    let request = mcp_json_rpc_request(request_id, method, params);
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "stdio MCP stdin 不可用。".to_string())?;
        stdin
            .write_all(format!("{initialize}\n{initialized}\n").as_bytes())
            .await
            .map_err(|error| format!("写入 stdio MCP 初始化请求失败：{error}"))?;
        stdin
            .write_all(format!("{request}\n").as_bytes())
            .await
            .map_err(|error| format!("写入 stdio MCP 请求失败：{error}"))?;
    }
    child.stdin.take();

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdio MCP stdout 不可用。".to_string())?;
    let mut lines = BufReader::new(stdout).lines();
    let read = async {
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|error| format!("读取 stdio MCP 响应失败：{error}"))?
        {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value: serde_json::Value = serde_json::from_str(trimmed)
                .map_err(|error| format!("stdio MCP 返回了非 JSON-RPC 消息：{error}"))?;
            if value.get("id").and_then(|item| item.as_i64()) == Some(initialize_id) {
                mcp_result_or_error(value)?;
                continue;
            }
            if value.get("id").and_then(|item| item.as_i64()) == Some(request_id) {
                return mcp_result_or_error(value);
            }
        }
        Err("stdio MCP 进程没有返回请求响应。".to_string())
    };
    let result = timeout(Duration::from_secs(10), read)
        .await
        .map_err(|_| "stdio MCP 请求超时。".to_string())?;
    child.kill().await.ok();
    result
}

fn mcp_initialize_request(id: i64) -> serde_json::Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "Aura",
                "version": "0.1.0"
            }
        }
    })
}

async fn request_http_mcp(
    server: &McpServerConfig,
    method: &str,
    params: serde_json::Value,
    name_header: Option<&str>,
) -> Result<serde_json::Value, String> {
    let url = server
        .url
        .as_deref()
        .ok_or_else(|| "HTTP/SSE MCP 缺少 URL。".to_string())?;

    // P0-3: MCP-server outbound sub-boundary. Local MCP (loopback) is allowed;
    // malformed / non-http(s) endpoints are refused. Screen the outbound payload
    // for secrets and emit a host-only audit line. We record the secret count
    // rather than masking the arguments, because masking could corrupt a
    // legitimate call (e.g. handing a token to a secrets-manager MCP); the
    // confirmation gate + P0-2 untrusted handling cover injection-driven exfil.
    {
        let policy = crate::tools::outbound::active_policy();
        let decision =
            policy.evaluate_url(crate::tools::outbound::OutboundChannel::McpServer, url, &[]);
        let secret_hits = crate::tools::outbound::screen_egress(&params.to_string()).secret_count;
        crate::tools::outbound::OutboundAudit {
            channel: crate::tools::outbound::OutboundChannel::McpServer,
            target: format!(
                "{} ({})",
                crate::tools::outbound::audit_target_host(url),
                server.id
            ),
            allowed: decision.is_allowed(),
            secret_hits,
            summary: format!("method={method}"),
        }
        .emit();
        if let crate::tools::outbound::OutboundDecision::Deny { reason } = decision {
            return Err(reason);
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|error| error.to_string())?;

    let initialize = mcp_initialize_request(1);
    let init_response = send_http_mcp_message(
        &client,
        url,
        server,
        initialize,
        MCP_PROTOCOL_VERSION,
        None,
        Some("initialize"),
        Some(1),
    )
    .await?;
    let protocol_version = init_response
        .result
        .as_ref()
        .and_then(|value| value.get("protocolVersion"))
        .and_then(|value| value.as_str())
        .unwrap_or(MCP_PROTOCOL_VERSION)
        .to_string();
    let session_id = init_response.session_id;

    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    });
    send_http_mcp_message(
        &client,
        url,
        server,
        initialized,
        &protocol_version,
        session_id.as_deref(),
        Some("notifications/initialized"),
        None,
    )
    .await?;

    let request_id = 2_i64;
    let request = mcp_json_rpc_request(request_id, method, params);
    let response = send_http_mcp_message(
        &client,
        url,
        server,
        request,
        &protocol_version,
        session_id.as_deref(),
        name_header.or(Some(method)),
        Some(request_id),
    )
    .await?;
    response
        .result
        .ok_or_else(|| "HTTP/SSE MCP 响应缺少 JSON-RPC 结果。".to_string())
}

struct HttpMcpResponse {
    result: Option<serde_json::Value>,
    session_id: Option<String>,
}

#[allow(clippy::too_many_arguments)]
async fn send_http_mcp_message(
    client: &reqwest::Client,
    url: &str,
    server: &McpServerConfig,
    request: serde_json::Value,
    protocol_version: &str,
    session_id: Option<&str>,
    name_header: Option<&str>,
    request_id: Option<i64>,
) -> Result<HttpMcpResponse, String> {
    let mut builder = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", protocol_version)
        .json(&request);
    for item in &server.headers {
        if !item.key.trim().is_empty() {
            builder = builder.header(item.key.trim(), item.value.trim());
        }
    }
    if server.auth_type.as_deref() == Some("bearer") {
        if let Some(token) = server
            .auth_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            builder = builder.bearer_auth(token);
        }
    }
    if let Some(name) = name_header {
        builder = builder.header("Mcp-Name", name);
    }
    if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
        builder = builder.header("Mcp-Session-Id", session_id);
    }
    let response = builder
        .send()
        .await
        .map_err(|error| format!("HTTP/SSE MCP 请求失败：{error}"))?;
    let status = response.status();
    let response_session_id = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取 HTTP/SSE MCP 响应失败：{error}"))?;
    if !status.is_success() {
        return Err(format!("HTTP/SSE MCP 返回 {status}: {}", text.trim()));
    }
    if request_id.is_none() && text.trim().is_empty() {
        return Ok(HttpMcpResponse {
            result: None,
            session_id: response_session_id,
        });
    }
    if content_type.contains("text/event-stream") {
        let request_id =
            request_id.ok_or_else(|| "HTTP/SSE MCP 通知不应返回 SSE 结果。".to_string())?;
        return Ok(HttpMcpResponse {
            result: Some(parse_sse_json_rpc_response(&text, request_id)?),
            session_id: response_session_id,
        });
    }
    if request_id.is_none() && text.trim().is_empty() {
        return Ok(HttpMcpResponse {
            result: None,
            session_id: response_session_id,
        });
    }
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("HTTP/SSE MCP 返回非 JSON：{error}"))?;
    let result = if request_id.is_some() {
        Some(mcp_result_or_error(value)?)
    } else {
        None
    };
    Ok(HttpMcpResponse {
        result,
        session_id: response_session_id,
    })
}

fn mcp_json_rpc_request(id: i64, method: &str, mut params: serde_json::Value) -> serde_json::Value {
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": MCP_PROTOCOL_VERSION,
        "io.modelcontextprotocol/clientInfo": {
            "name": "Aura",
            "version": "0.1.0"
        },
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    if let Some(object) = params.as_object_mut() {
        object.insert("_meta".to_string(), meta);
    } else {
        params = json!({ "_meta": meta });
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    })
}

fn parse_sse_json_rpc_response(text: &str, request_id: i64) -> Result<serde_json::Value, String> {
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("data:") {
            continue;
        }
        let payload = line.trim_start_matches("data:").trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(payload)
            .map_err(|error| format!("SSE MCP data 不是 JSON：{error}"))?;
        if value.get("id").and_then(|item| item.as_i64()) == Some(request_id) {
            return mcp_result_or_error(value);
        }
    }
    Err("SSE MCP 响应中没有找到对应 JSON-RPC 结果。".to_string())
}

fn mcp_result_or_error(value: serde_json::Value) -> Result<serde_json::Value, String> {
    if let Some(error) = value.get("error") {
        return Err(format!("MCP JSON-RPC error: {error}"));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| "MCP JSON-RPC 响应缺少 result。".to_string())
}

fn tools_from_mcp_response(
    response: &serde_json::Value,
    server: &McpServerConfig,
) -> Vec<McpToolInfo> {
    response
        .get("tools")
        .and_then(|tools| tools.as_array())
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| {
                    let name = tool.get("name").and_then(|value| value.as_str())?;
                    let description = tool
                        .get("description")
                        .and_then(|value| value.as_str())
                        .unwrap_or("MCP tool");
                    let read_only = tool
                        .get("annotations")
                        .and_then(|value| value.get("readOnlyHint"))
                        .and_then(|value| value.as_bool())
                        .unwrap_or(server.risk == "safe");
                    Some(McpToolInfo {
                        name: name.to_string(),
                        title: name.to_string(),
                        description: description.to_string(),
                        read_only,
                        risk: if read_only {
                            "safe".to_string()
                        } else {
                            server.risk.clone()
                        },
                        requires_confirmation: !read_only || server.risk != "safe",
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db() -> LocalDb {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        LocalDb::open(std::env::temp_dir().join(format!("aura_mcp_{unique}.db"))).unwrap()
    }

    #[test]
    fn default_mcp_list_is_empty_and_filters_legacy_mock_server() {
        let db = temp_db();
        let servers = load_mcp_servers(&db).unwrap();
        assert!(servers.is_empty());

        store_mcp_servers(&db, &[mock_server_config()]).unwrap();
        let servers = load_mcp_servers(&db).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn save_mcp_server_persists_and_reloads_real_config() {
        let db = temp_db();
        store_mcp_servers(&db, &[mock_server_config()]).unwrap();

        let saved = save_mcp_server(
            &db,
            McpServerInput {
                id: Some("persisted_http_mcp".to_string()),
                name: "  Persisted HTTP MCP  ".to_string(),
                transport: "http/sse".to_string(),
                command: None,
                args: Vec::new(),
                url: Some("  http://127.0.0.1:8765/mcp  ".to_string()),
                env: vec![
                    McpKeyValue {
                        key: "  ".to_string(),
                        value: "ignored".to_string(),
                    },
                    McpKeyValue {
                        key: "AURA_TOKEN".to_string(),
                        value: " local-token ".to_string(),
                    },
                ],
                pass_env: vec![" PATH ".to_string(), " ".to_string()],
                cwd: Some(std::env::temp_dir().to_string_lossy().to_string()),
                headers: vec![McpKeyValue {
                    key: "X-Smoke".to_string(),
                    value: " enabled ".to_string(),
                }],
                auth_type: Some("bearer".to_string()),
                auth_token: Some(" smoke-secret ".to_string()),
                enabled: true,
                risk: "safe".to_string(),
            },
        )
        .unwrap();

        assert_eq!(saved.id, "persisted_http_mcp");
        assert_eq!(saved.name, "Persisted HTTP MCP");
        assert_eq!(saved.transport, "http_sse");

        let loaded = load_mcp_servers(&db).unwrap();
        assert_eq!(loaded.len(), 1);
        let server = &loaded[0];
        assert_eq!(server.id, "persisted_http_mcp");
        assert_eq!(server.url.as_deref(), Some("http://127.0.0.1:8765/mcp"));
        assert_eq!(server.auth_type.as_deref(), Some("bearer"));
        assert_eq!(server.auth_token.as_deref(), Some("smoke-secret"));
        assert_eq!(server.env.len(), 1);
        assert_eq!(server.env[0].key, "AURA_TOKEN");
        assert_eq!(server.env[0].value, "local-token");
        assert_eq!(server.pass_env, vec!["PATH".to_string()]);
        assert_eq!(server.headers.len(), 1);
        assert_eq!(server.headers[0].key, "X-Smoke");
        assert_eq!(server.headers[0].value, "enabled");
        assert!(loaded.iter().all(|server| server.transport != "mock"));
    }

    #[tokio::test]
    async fn http_mcp_tools_list_and_call_round_trip() {
        let db = temp_db();
        let url = start_http_mcp_fixture();
        let server = save_mcp_server(
            &db,
            McpServerInput {
                id: Some("test_http_mcp".to_string()),
                name: "Test HTTP MCP".to_string(),
                transport: "http_sse".to_string(),
                command: None,
                args: Vec::new(),
                url: Some(url),
                env: Vec::new(),
                pass_env: Vec::new(),
                cwd: None,
                headers: Vec::new(),
                auth_type: None,
                auth_token: None,
                enabled: true,
                risk: "safe".to_string(),
            },
        )
        .unwrap();

        let tools = list_real_mcp_tools(&server).await.unwrap();
        assert_eq!(tools.len(), 2);
        assert!(tools
            .iter()
            .any(|tool| tool.name == "echo" && tool.read_only));
        assert!(tools
            .iter()
            .any(|tool| tool.name == "mutate" && tool.requires_confirmation));

        set_mcp_server_trust(&db, &server.id, true).unwrap();
        let result = invoke_mcp_tool(
            &db,
            &server.id,
            "echo",
            json!({ "message": "hello" }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(result.status, "ok");
        assert!(result.server_trusted);
        assert!(result.output.get("content").is_some());
        assert!(list_mcp_audit_events(&db, 20)
            .unwrap()
            .iter()
            .any(|event| event.server_id == "test_http_mcp" && event.status == "ok"));
    }

    #[tokio::test]
    async fn stdio_mcp_tools_list_and_call_round_trip() {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let db = temp_db();
        let script_path = std::env::temp_dir().join(format!(
            "aura_stdio_mcp_{}.mjs",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(
            &script_path,
            r#"
let input = '';
process.stdin.on('data', chunk => input += chunk.toString());
process.stdin.on('end', () => {
  for (const line of input.trim().split(/\r?\n/)) {
    const request = JSON.parse(line);
    if (!request.id) continue;
    const result = request.method === 'initialize'
      ? { protocolVersion: '2024-11-05', capabilities: {}, serverInfo: { name: 'fixture', version: '1' } }
      : request.method === 'tools/list'
        ? {
            tools: [
              { name: 'echo', description: 'Echo arguments', annotations: { readOnlyHint: true } },
              { name: 'mutate', description: 'Mutation probe', annotations: { readOnlyHint: false } }
            ]
          }
        : {
            content: [{ type: 'text', text: 'stdio called' }],
            isError: false
          };
    process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result }) + '\n');
  }
});
"#,
        )
        .unwrap();

        let server = save_mcp_server(
            &db,
            McpServerInput {
                id: Some("test_stdio_mcp".to_string()),
                name: "Test Stdio MCP".to_string(),
                transport: "stdio".to_string(),
                command: Some("node".to_string()),
                args: vec![script_path.to_string_lossy().to_string()],
                url: None,
                env: vec![McpKeyValue {
                    key: "AURA_MCP_FIXTURE".to_string(),
                    value: "1".to_string(),
                }],
                pass_env: Vec::new(),
                cwd: None,
                headers: Vec::new(),
                auth_type: None,
                auth_token: None,
                enabled: true,
                risk: "safe".to_string(),
            },
        )
        .unwrap();

        let tools = list_real_mcp_tools(&server).await.unwrap();
        assert_eq!(tools.len(), 2);
        assert!(tools
            .iter()
            .any(|tool| tool.name == "echo" && tool.read_only));
        set_mcp_server_trust(&db, &server.id, true).unwrap();
        let result = invoke_mcp_tool(
            &db,
            &server.id,
            "echo",
            json!({ "message": "hello" }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(result.status, "ok");
        assert!(result.output.get("content").is_some());
        assert!(list_mcp_audit_events(&db, 20)
            .unwrap()
            .iter()
            .any(|event| event.server_id == "test_stdio_mcp" && event.status == "ok"));

        fs::remove_file(script_path).ok();
    }

    fn trust_test_server(db: &LocalDb, id: &str) -> McpServerConfig {
        save_mcp_server(
            db,
            McpServerInput {
                id: Some(id.to_string()),
                name: "Trust Probe".to_string(),
                transport: "http/sse".to_string(),
                command: None,
                args: Vec::new(),
                url: Some("http://127.0.0.1:9/mcp".to_string()),
                env: Vec::new(),
                pass_env: Vec::new(),
                cwd: None,
                headers: Vec::new(),
                auth_type: None,
                auth_token: None,
                enabled: true,
                risk: "safe".to_string(),
            },
        )
        .unwrap()
    }

    #[test]
    fn new_server_defaults_untrusted_and_trust_persists() {
        let db = temp_db();
        let saved = trust_test_server(&db, "trust_persist_mcp");
        assert!(!saved.trusted, "新建 server 必须默认未信任（fail-closed）");
        assert!(saved.trusted_at.is_none());

        let trusted = set_mcp_server_trust(&db, "trust_persist_mcp", true).unwrap();
        assert!(trusted.trusted);
        assert!(trusted.trusted_at.is_some());

        // Editing the server must preserve the trust decision, not reset it.
        let reloaded = trust_test_server(&db, "trust_persist_mcp");
        assert!(reloaded.trusted, "编辑 server 不应清掉已有信任");

        let revoked = set_mcp_server_trust(&db, "trust_persist_mcp", false).unwrap();
        assert!(!revoked.trusted);
        assert!(revoked.trusted_at.is_none());

        assert!(list_mcp_audit_events(&db, 20)
            .unwrap()
            .iter()
            .any(|event| event.action == "trust" && event.server_id == "trust_persist_mcp"));
    }

    #[tokio::test]
    async fn untrusted_server_invoke_is_blocked_and_audited() {
        let db = temp_db();
        trust_test_server(&db, "untrusted_mcp");
        // No network fixture: an untrusted server must be refused before any call.
        let error = invoke_mcp_tool(&db, "untrusted_mcp", "echo", json!({ "k": "v" }), true)
            .await
            .unwrap_err();
        assert!(error.contains("信任"), "未信任 server 必须被拦截");

        let blocked = list_mcp_audit_events(&db, 20)
            .unwrap()
            .into_iter()
            .find(|event| event.server_id == "untrusted_mcp" && event.status == "blocked")
            .expect("拦截必须落审计");
        assert_eq!(blocked.action, "invoke");
        assert!(blocked.input_summary.contains("字段[k]"));
    }

    #[test]
    fn input_summary_lists_keys_and_counts_secrets_without_raw_values() {
        let summary = summarize_mcp_input(&json!({
            "token": "sk-ant-1234567890ABCDEFGHIJ",
            "path": "/tmp/file"
        }));
        assert!(summary.contains("字段[path, token]"));
        assert!(summary.contains("字符"));
        assert!(summary.contains("敏感"));
        assert!(
            !summary.contains("sk-ant-1234567890ABCDEFGHIJ"),
            "审计摘要绝不能包含原始密钥值"
        );

        let empty = summarize_mcp_input(&json!({}));
        assert!(empty.contains("无字段"));
    }

    fn start_http_mcp_fixture() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        // De-flake: serve an unbounded number of connections on a detached thread.
        // The client's connection count depends on keep-alive / pool reuse, so a
        // fixed `take(N)` + join was brittle — too few connections hung the join,
        // too many got refused. The thread parks on accept after the test and is
        // reaped at process exit.
        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => handle_http_mcp_request(stream),
                    Err(_) => break,
                }
            }
        });
        url
    }

    fn handle_http_mcp_request(mut stream: TcpStream) {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if let Some((headers, body_start)) = split_http_request(&buffer) {
                let content_length = http_content_length(headers).unwrap_or(0);
                if buffer.len() >= body_start + content_length {
                    break;
                }
            }
        }
        let (_, body_start) = split_http_request(&buffer).unwrap();
        let request: serde_json::Value = serde_json::from_slice(&buffer[body_start..]).unwrap();
        let id = request.get("id").cloned().unwrap_or_else(|| json!(1));
        let method = request
            .get("method")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let response = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "serverInfo": { "name": "fixture", "version": "1" }
                }
            }),
            "notifications/initialized" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {}
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": [
                        {
                            "name": "echo",
                            "description": "Echo arguments",
                            "annotations": { "readOnlyHint": true }
                        },
                        {
                            "name": "mutate",
                            "description": "Mutation probe",
                            "annotations": { "readOnlyHint": false }
                        }
                    ]
                }
            }),
            "tools/call" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [
                        { "type": "text", "text": "called" }
                    ],
                    "isError": false
                }
            }),
            other => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("unknown method {other}") }
            }),
        };
        let body = response.to_string();
        let http_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: fixture-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(http_response.as_bytes()).unwrap();
    }

    fn split_http_request(buffer: &[u8]) -> Option<(&[u8], usize)> {
        buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|position| (&buffer[..position], position + 4))
    }

    fn http_content_length(headers: &[u8]) -> Option<usize> {
        let headers = String::from_utf8_lossy(headers);
        headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse().ok()
            } else {
                None
            }
        })
    }
}
