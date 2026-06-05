use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::time::{timeout, Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::storage::LocalDb;

const FEISHU_EVENTS_KEY: &str = "feishu_received_events_v1";
const FEISHU_STATUS_KEY: &str = "feishu_callback_status_v1";
const FEISHU_TUNNEL_STATUS_KEY: &str = "feishu_tunnel_status_v1";
const FEISHU_EVENT_LIMIT: usize = 300;
const FEISHU_TUNNEL_TIMEOUT_SECONDS: u64 = 35;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuReceiveRequirement {
    pub id: String,
    pub label: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuReceivedEvent {
    pub id: String,
    pub event_id: String,
    pub event_type: String,
    #[serde(default = "default_event_source")]
    pub source: String,
    pub chat_id: Option<String>,
    pub sender_id: Option<String>,
    pub message_type: Option<String>,
    pub text: Option<String>,
    pub raw: Value,
    pub received_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuCallbackStatus {
    pub running: bool,
    pub local_url: Option<String>,
    #[serde(default)]
    pub public_url: Option<String>,
    pub received_count: usize,
    pub last_event_at: Option<i64>,
    pub message: String,
    #[serde(default)]
    pub receive_ready: bool,
    #[serde(default)]
    pub requirements: Vec<FeishuReceiveRequirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuCallbackResult {
    pub status_code: u16,
    pub body: Value,
    pub event: Option<FeishuReceivedEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuTunnelStatus {
    pub running: bool,
    pub provider: String,
    pub public_url: Option<String>,
    pub callback_url: Option<String>,
    pub local_url: String,
    pub started_at: Option<i64>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuSetupLinks {
    pub app_id: String,
    pub app_home_url: String,
    pub permission_url: String,
    pub event_url: String,
    pub required_chat_scopes: Vec<String>,
    pub required_message_scopes: Vec<String>,
    pub required_event_type: String,
}

pub struct FeishuCallbackServer {
    pub port: u16,
    shutdown: CancellationToken,
}

pub struct FeishuTunnelProcess {
    pub status: FeishuTunnelStatus,
    child: Child,
}

impl FeishuTunnelProcess {
    pub async fn stop(mut self, db: &LocalDb) -> Result<FeishuTunnelStatus, String> {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
        let mut status = self.status.clone();
        status.running = false;
        status.message = "飞书公网穿透已停止；飞书开放平台中的临时回调地址不再可用。".to_string();
        store_feishu_tunnel_status(db, &status)?;
        Ok(status)
    }
}

impl Drop for FeishuTunnelProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

impl FeishuCallbackServer {
    pub fn stop(&self) {
        self.shutdown.cancel();
    }
}

pub async fn start_feishu_callback_server(
    db: LocalDb,
    port: Option<u16>,
) -> Result<FeishuCallbackServer, String> {
    let bind_port = port.unwrap_or(18080);
    let listener = TcpListener::bind(("127.0.0.1", bind_port))
        .await
        .map_err(|error| format!("飞书回调服务启动失败：{error}"))?;
    let actual_port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let shutdown = CancellationToken::new();
    let shutdown_task = shutdown.clone();
    let status_db = db.clone();
    let db = Arc::new(db);
    let server_db = db.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_task.cancelled() => break,
                accepted = listener.accept() => {
                    let Ok((mut socket, _addr)) = accepted else { continue };
                    let db = server_db.clone();
                    tokio::spawn(async move {
                        let mut buffer = Vec::new();
                        let mut chunk = [0_u8; 4096];
                        let mut header_end = None;
                        let mut content_length = 0_usize;
                        loop {
                            let Ok(read) = socket.read(&mut chunk).await else { return };
                            if read == 0 { break; }
                            buffer.extend_from_slice(&chunk[..read]);
                            if header_end.is_none() {
                                if let Some(pos) = find_header_end(&buffer) {
                                    header_end = Some(pos);
                                    content_length = parse_content_length(&buffer[..pos]).unwrap_or(0);
                                }
                            }
                            if let Some(pos) = header_end {
                                if buffer.len() >= pos + 4 + content_length {
                                    break;
                                }
                            }
                            if buffer.len() > 2 * 1024 * 1024 {
                                break;
                            }
                        }
                        let response = handle_raw_http_request(&db, &buffer);
                        let body = response.body.to_string();
                        let status_text = if response.status_code == 200 { "OK" } else { "BAD REQUEST" };
                        let raw = format!(
                            "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            response.status_code,
                            status_text,
                            body.len(),
                            body
                        );
                        let _ = socket.write_all(raw.as_bytes()).await;
                    });
                }
            }
        }
    });

    let mut status = FeishuCallbackStatus {
        running: true,
        local_url: Some(format!("http://127.0.0.1:{actual_port}/feishu/events")),
        public_url: None,
        received_count: list_feishu_events(&status_db, FEISHU_EVENT_LIMIT)?.len(),
        last_event_at: latest_feishu_event_at(&status_db)?,
        message: "本地飞书回调服务已启动；飞书平台需要配置可公网访问的回调地址。".to_string(),
        receive_ready: false,
        requirements: Vec::new(),
    };
    refresh_feishu_status(&status_db, true, &mut status)?;
    store_feishu_status(&status_db, &status)?;
    Ok(FeishuCallbackServer {
        port: actual_port,
        shutdown,
    })
}

pub fn stop_feishu_callback_server(db: &LocalDb) -> Result<FeishuCallbackStatus, String> {
    let mut status = load_feishu_status(db)?;
    status.running = false;
    status.message = "飞书本地回调服务已停止。".to_string();
    refresh_feishu_status(db, false, &mut status)?;
    store_feishu_status(db, &status)?;
    Ok(status)
}

pub fn get_feishu_callback_status(
    db: &LocalDb,
    running: bool,
) -> Result<FeishuCallbackStatus, String> {
    let mut status = load_feishu_status(db)?;
    refresh_feishu_status(db, running, &mut status)?;
    Ok(status)
}

pub fn set_feishu_public_url(
    db: &LocalDb,
    public_url: Option<String>,
    running: bool,
) -> Result<FeishuCallbackStatus, String> {
    let mut status = load_feishu_status(db)?;
    status.public_url = public_url
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    status.message = if status.public_url.is_some() {
        "飞书公网回调地址已记录；仍需在飞书开放平台完成 URL 校验并收到真实事件。".to_string()
    } else {
        "飞书公网回调地址已清空。".to_string()
    };
    refresh_feishu_status(db, running, &mut status)?;
    store_feishu_status(db, &status)?;
    Ok(status)
}

pub fn get_feishu_setup_links() -> Result<FeishuSetupLinks, String> {
    let app_id = crate::env::secret_value("FEISHU_APP_ID")
        .or_else(|| crate::env::secret_value("FEISHU_APPID"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "缺少 FEISHU_APP_ID，不能打开飞书应用后台。".to_string())?;
    let chat_scopes = feishu_chat_scopes();
    let message_scopes = feishu_message_scopes();
    let all_scopes = chat_scopes
        .iter()
        .chain(message_scopes.iter())
        .cloned()
        .collect::<Vec<_>>();
    let app_home_url = format!("https://open.feishu.cn/app/{app_id}");
    Ok(FeishuSetupLinks {
        permission_url: format!(
            "{app_home_url}/auth?q={}&op_from=openapi&token_type=tenant",
            all_scopes.join(",")
        ),
        event_url: format!("{app_home_url}/event"),
        app_home_url,
        app_id,
        required_chat_scopes: chat_scopes,
        required_message_scopes: message_scopes,
        required_event_type: "im.message.receive_v1".to_string(),
    })
}

pub async fn start_feishu_public_tunnel(
    db: LocalDb,
    port: u16,
) -> Result<FeishuTunnelProcess, String> {
    let mut command = Command::new(npx_command());
    let port_arg = port.to_string();
    command
        .args([
            "-y",
            "localtunnel",
            "--port",
            &port_arg,
            "--local-host",
            "127.0.0.1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("localtunnel 启动失败：{error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "localtunnel 没有可读取的输出。".to_string())?;
    let root_url = match timeout(
        Duration::from_secs(FEISHU_TUNNEL_TIMEOUT_SECONDS),
        read_tunnel_public_url(stdout),
    )
    .await
    {
        Ok(Ok(url)) => url,
        Ok(Err(error)) => {
            kill_child(&mut child).await;
            return Err(error);
        }
        Err(_) => {
            kill_child(&mut child).await;
            return Err("localtunnel 启动超时；没有获得公网地址。".to_string());
        }
    };
    let callback_url = format!("{}/feishu/events", root_url.trim_end_matches('/'));
    let mut callback_status = match set_feishu_public_url(&db, Some(callback_url.clone()), true) {
        Ok(status) => status,
        Err(error) => {
            kill_child(&mut child).await;
            return Err(error);
        }
    };
    callback_status.message =
        "已生成临时公网回调地址；请在飞书开放平台事件订阅中完成 URL 校验。".to_string();
    if let Err(error) = store_feishu_status(&db, &callback_status) {
        kill_child(&mut child).await;
        return Err(error);
    }
    let status = FeishuTunnelStatus {
        running: true,
        provider: "localtunnel".to_string(),
        public_url: Some(root_url),
        callback_url: Some(callback_url),
        local_url: format!("http://127.0.0.1:{port}/feishu/events"),
        started_at: Some(Utc::now().timestamp_millis()),
        message: "localtunnel 已启动；等待飞书开放平台校验和真实事件推送。".to_string(),
    };
    if let Err(error) = store_feishu_tunnel_status(&db, &status) {
        kill_child(&mut child).await;
        return Err(error);
    }
    Ok(FeishuTunnelProcess { status, child })
}

async fn kill_child(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

pub fn get_feishu_tunnel_status(db: &LocalDb, running: bool) -> Result<FeishuTunnelStatus, String> {
    let mut status = load_feishu_tunnel_status(db)?;
    status.running = running;
    if !running && status.callback_url.is_some() {
        status.message =
            "飞书公网穿透未运行；显示的是上次生成的临时地址，当前不能作为可用回调。".to_string();
    }
    Ok(status)
}

pub fn list_feishu_events(db: &LocalDb, limit: usize) -> Result<Vec<FeishuReceivedEvent>, String> {
    let stored = db
        .get_app_state(FEISHU_EVENTS_KEY)
        .map_err(|error| error.to_string())?;
    let mut events: Vec<FeishuReceivedEvent> = match stored {
        Some(value) => serde_json::from_value(value).map_err(|error| error.to_string())?,
        None => Vec::new(),
    };
    events.sort_by_key(|e| std::cmp::Reverse(e.received_at));
    events.truncate(limit.min(FEISHU_EVENT_LIMIT));
    Ok(events)
}

pub fn handle_feishu_event_payload(
    db: &LocalDb,
    payload: Value,
) -> Result<FeishuCallbackResult, String> {
    handle_feishu_event_payload_with_source(db, payload, "manual_ingest")
}

fn handle_feishu_event_payload_with_source(
    db: &LocalDb,
    payload: Value,
    source: &str,
) -> Result<FeishuCallbackResult, String> {
    if let Some(challenge) = payload.get("challenge").and_then(Value::as_str) {
        verify_feishu_token(&payload)?;
        return Ok(FeishuCallbackResult {
            status_code: 200,
            body: json!({ "challenge": challenge }),
            event: None,
        });
    }

    verify_feishu_token(&payload)?;
    let event_id = extract_event_id(&payload);
    let event_type = extract_event_type(&payload);
    let mut events = list_feishu_events(db, FEISHU_EVENT_LIMIT)?;
    if let Some(existing_index) = events.iter().position(|event| event.event_id == event_id) {
        if is_verified_feishu_source(source)
            && !is_verified_feishu_source(&events[existing_index].source)
        {
            let event = FeishuReceivedEvent {
                id: events[existing_index].id.clone(),
                event_id,
                event_type,
                source: source.to_string(),
                chat_id: payload
                    .pointer("/event/message/chat_id")
                    .or_else(|| payload.pointer("/event/open_chat_id"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                sender_id: payload
                    .pointer("/event/sender/sender_id/open_id")
                    .or_else(|| payload.pointer("/event/sender/sender_id/user_id"))
                    .or_else(|| payload.pointer("/event/open_id"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                message_type: payload
                    .pointer("/event/message/message_type")
                    .or_else(|| payload.pointer("/event/message_type"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                text: extract_feishu_text(&payload),
                raw: payload,
                received_at: Utc::now().timestamp_millis(),
            };
            events[existing_index] = event.clone();
            events.sort_by_key(|e| std::cmp::Reverse(e.received_at));
            db.set_app_state(FEISHU_EVENTS_KEY, json!(events))
                .map_err(|error| error.to_string())?;
            let mut status = load_feishu_status(db)?;
            status.received_count = list_feishu_events(db, FEISHU_EVENT_LIMIT)?.len();
            status.last_event_at = Some(event.received_at);
            status.message = "已用真实飞书事件更新本地审计。".to_string();
            let running = status.running;
            refresh_feishu_status(db, running, &mut status)?;
            store_feishu_status(db, &status)?;
            return Ok(FeishuCallbackResult {
                status_code: 200,
                body: json!({ "code": 0, "msg": "duplicate upgraded" }),
                event: Some(event),
            });
        }
        return Ok(FeishuCallbackResult {
            status_code: 200,
            body: json!({ "code": 0, "msg": "duplicate ignored" }),
            event: Some(events.remove(existing_index)),
        });
    }
    let event = FeishuReceivedEvent {
        id: format!("feishu_evt_{}", Uuid::new_v4()),
        event_id,
        event_type,
        source: source.to_string(),
        chat_id: payload
            .pointer("/event/message/chat_id")
            .or_else(|| payload.pointer("/event/open_chat_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        sender_id: payload
            .pointer("/event/sender/sender_id/open_id")
            .or_else(|| payload.pointer("/event/sender/sender_id/user_id"))
            .or_else(|| payload.pointer("/event/open_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        message_type: payload
            .pointer("/event/message/message_type")
            .or_else(|| payload.pointer("/event/message_type"))
            .and_then(Value::as_str)
            .map(str::to_string),
        text: extract_feishu_text(&payload),
        raw: payload,
        received_at: Utc::now().timestamp_millis(),
    };
    events.push(event.clone());
    events.sort_by_key(|e| std::cmp::Reverse(e.received_at));
    events.truncate(FEISHU_EVENT_LIMIT);
    db.set_app_state(FEISHU_EVENTS_KEY, json!(events))
        .map_err(|error| error.to_string())?;
    let mut status = load_feishu_status(db)?;
    status.received_count = list_feishu_events(db, FEISHU_EVENT_LIMIT)?.len();
    status.last_event_at = Some(event.received_at);
    status.message = "已接收飞书事件并写入本地审计。".to_string();
    let running = status.running;
    refresh_feishu_status(db, running, &mut status)?;
    store_feishu_status(db, &status)?;
    Ok(FeishuCallbackResult {
        status_code: 200,
        body: json!({ "code": 0, "msg": "ok" }),
        event: Some(event),
    })
}

fn handle_raw_http_request(db: &LocalDb, request: &[u8]) -> FeishuCallbackResult {
    let Some(header_end) = find_header_end(request) else {
        return bad_request("HTTP 请求不完整。");
    };
    let first_line = String::from_utf8_lossy(&request[..header_end])
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    if !first_line.starts_with("POST ") {
        return FeishuCallbackResult {
            status_code: 200,
            body: json!({ "ok": true, "service": "atlas-feishu-callback" }),
            event: None,
        };
    }
    let body = &request[header_end + 4..];
    let source = if request_path(&first_line) == Some("/feishu/sdk-events") {
        match verify_sdk_bridge_token(&request[..header_end]) {
            Ok(()) => "sdk_websocket",
            Err(error) => return bad_request(&error),
        }
    } else if is_public_callback_request(db, &request[..header_end]) {
        "public_callback"
    } else {
        "local_callback"
    };
    match serde_json::from_slice::<Value>(body) {
        Ok(payload) => handle_feishu_event_payload_with_source(db, payload, source)
            .unwrap_or_else(|error| bad_request(&error)),
        Err(error) => bad_request(&format!("飞书回调 JSON 解析失败：{error}")),
    }
}

fn verify_feishu_token(payload: &Value) -> Result<(), String> {
    let expected = crate::env::secret_value("FEISHU_VERIFICATION_TOKEN")
        .or_else(|| crate::env::secret_value("FEISHU_APP_VERIFICATION_TOKEN"));
    let Some(expected) = expected.filter(|value| !value.trim().is_empty()) else {
        return Ok(());
    };
    let actual = payload
        .get("token")
        .or_else(|| payload.pointer("/header/token"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if actual == expected {
        Ok(())
    } else {
        Err("飞书回调 verification token 不匹配。".to_string())
    }
}

fn extract_event_id(payload: &Value) -> String {
    payload
        .pointer("/header/event_id")
        .or_else(|| payload.get("uuid"))
        .or_else(|| payload.pointer("/event/message/message_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("event_{}", Uuid::new_v4()))
}

fn extract_event_type(payload: &Value) -> String {
    payload
        .pointer("/header/event_type")
        .or_else(|| payload.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

fn extract_feishu_text(payload: &Value) -> Option<String> {
    let raw = payload
        .pointer("/event/message/content")
        .or_else(|| payload.pointer("/event/text"))
        .and_then(Value::as_str)?;
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| Some(raw.to_string()))
}

fn load_feishu_status(db: &LocalDb) -> Result<FeishuCallbackStatus, String> {
    let stored = db
        .get_app_state(FEISHU_STATUS_KEY)
        .map_err(|error| error.to_string())?;
    match stored {
        Some(value) => serde_json::from_value(value).map_err(|error| error.to_string()),
        None => Ok(FeishuCallbackStatus {
            running: false,
            local_url: Some("http://127.0.0.1:18080/feishu/events".to_string()),
            public_url: public_url_from_env(),
            received_count: 0,
            last_event_at: None,
            message: "飞书回调服务未启动。".to_string(),
            receive_ready: false,
            requirements: Vec::new(),
        }),
    }
}

fn store_feishu_status(db: &LocalDb, status: &FeishuCallbackStatus) -> Result<(), String> {
    db.set_app_state(FEISHU_STATUS_KEY, json!(status))
        .map_err(|error| error.to_string())
}

fn load_feishu_tunnel_status(db: &LocalDb) -> Result<FeishuTunnelStatus, String> {
    let stored = db
        .get_app_state(FEISHU_TUNNEL_STATUS_KEY)
        .map_err(|error| error.to_string())?;
    match stored {
        Some(value) => serde_json::from_value(value).map_err(|error| error.to_string()),
        None => Ok(FeishuTunnelStatus {
            running: false,
            provider: "localtunnel".to_string(),
            public_url: None,
            callback_url: None,
            local_url: "http://127.0.0.1:18080/feishu/events".to_string(),
            started_at: None,
            message: "飞书公网穿透未启动。".to_string(),
        }),
    }
}

fn store_feishu_tunnel_status(db: &LocalDb, status: &FeishuTunnelStatus) -> Result<(), String> {
    db.set_app_state(FEISHU_TUNNEL_STATUS_KEY, json!(status))
        .map_err(|error| error.to_string())
}

fn latest_feishu_event_at(db: &LocalDb) -> Result<Option<i64>, String> {
    Ok(list_feishu_events(db, 1)?
        .into_iter()
        .next()
        .map(|event| event.received_at))
}

pub fn count_public_feishu_events(db: &LocalDb) -> Result<usize, String> {
    Ok(list_feishu_events(db, FEISHU_EVENT_LIMIT)?
        .into_iter()
        .filter(|event| event.source == "public_callback")
        .count())
}

pub fn count_websocket_feishu_events(db: &LocalDb) -> Result<usize, String> {
    Ok(list_feishu_events(db, FEISHU_EVENT_LIMIT)?
        .into_iter()
        .filter(|event| event.source == "sdk_websocket")
        .count())
}

pub fn count_verified_feishu_events(db: &LocalDb) -> Result<usize, String> {
    Ok(list_feishu_events(db, FEISHU_EVENT_LIMIT)?
        .into_iter()
        .filter(|event| event.source == "public_callback" || event.source == "sdk_websocket")
        .count())
}

fn refresh_feishu_status(
    db: &LocalDb,
    running: bool,
    status: &mut FeishuCallbackStatus,
) -> Result<(), String> {
    status.running = running;
    status.received_count = list_feishu_events(db, FEISHU_EVENT_LIMIT)?.len();
    status.last_event_at = latest_feishu_event_at(db)?;
    if status.public_url.is_none() {
        status.public_url = public_url_from_env();
    }
    let public_event_count = count_public_feishu_events(db)?;
    let websocket_event_count = count_websocket_feishu_events(db)?;
    status.requirements =
        build_feishu_requirements(status, public_event_count, websocket_event_count);
    status.receive_ready = status
        .requirements
        .iter()
        .all(|requirement| requirement.status == "ready");
    if status.receive_ready {
        status.message = "飞书发送和真实事件接收闭环已通过记录验证。".to_string();
    }
    Ok(())
}

fn build_feishu_requirements(
    status: &FeishuCallbackStatus,
    public_event_count: usize,
    websocket_event_count: usize,
) -> Vec<FeishuReceiveRequirement> {
    let verification_token = crate::env::secret_value("FEISHU_VERIFICATION_TOKEN")
        .or_else(|| crate::env::secret_value("FEISHU_APP_VERIFICATION_TOKEN"))
        .is_some();
    let public_url = status.public_url.as_deref().unwrap_or("").trim();
    let public_url_ready = public_url.starts_with("https://");
    let verified_event_count = public_event_count + websocket_event_count;
    let verified_receive_ready = verified_event_count > 0;
    let receiver_ready = public_url_ready || verified_receive_ready;
    vec![
        requirement(
            "local_service",
            "本地回调服务",
            status.running || verified_receive_ready,
            if verified_receive_ready {
                "已完成至少一次真实飞书事件接收验证；接收器可按需再次启动。"
            } else if status.running {
                "已启动本地 /feishu/events 接收服务。"
            } else {
                "需要先启动本地回调服务。"
            },
        ),
        requirement(
            "receive_channel",
            "公网回调或长连接",
            receiver_ready,
            if websocket_event_count > 0 {
                "已通过飞书官方 SDK 长连接收到真实事件。"
            } else if public_event_count > 0 {
                "已通过 Host 匹配公网回调地址收到真实事件。"
            } else if public_url_ready {
                "已记录 https 公网地址，可填入飞书开放平台。"
            } else if public_url.is_empty() {
                "缺少公网回调地址；或启动飞书长连接接收。"
            } else {
                "公网回调地址必须使用 https。"
            },
        ),
        requirement(
            "verification_token",
            "Verification Token",
            verification_token,
            if verification_token {
                "已配置 FEISHU_VERIFICATION_TOKEN 或 FEISHU_APP_VERIFICATION_TOKEN。"
            } else {
                "缺少飞书开放平台事件订阅的 verification token。"
            },
        ),
        requirement(
            "real_event",
            "真实事件接收",
            verified_event_count > 0,
            if websocket_event_count > 0 {
                "已经收到飞书官方 SDK 长连接事件。"
            } else if public_event_count > 0 {
                "已经收到 Host 匹配公网回调地址的飞书事件。"
            } else if status.received_count > 0 {
                "已有本地或手动测试事件，但还没有公网回调或长连接事件。"
            } else {
                "还没有收到飞书平台推送的真实事件。"
            },
        ),
    ]
}

fn requirement(id: &str, label: &str, ready: bool, detail: &str) -> FeishuReceiveRequirement {
    FeishuReceiveRequirement {
        id: id.to_string(),
        label: label.to_string(),
        status: if ready { "ready" } else { "missing" }.to_string(),
        detail: detail.to_string(),
    }
}

fn public_url_from_env() -> Option<String> {
    crate::env::secret_value("ATLAS_FEISHU_PUBLIC_URL")
        .or_else(|| crate::env::secret_value("FEISHU_PUBLIC_CALLBACK_URL"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn feishu_chat_scopes() -> Vec<String> {
    [
        "im:chat:readonly",
        "im:chat",
        "im:chat.group_info:readonly",
        "im:chat:read",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn feishu_message_scopes() -> Vec<String> {
    [
        "im:message.group_at_msg:readonly",
        "im:message.p2p_msg:readonly",
        "im:message.group_msg",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

async fn read_tunnel_public_url(stdout: tokio::process::ChildStdout) -> Result<String, String> {
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|error| format!("localtunnel 输出读取失败：{error}"))?
    {
        if let Some(url) = extract_tunnel_public_url(&line) {
            return Ok(url);
        }
    }
    Err("localtunnel 已退出，但没有输出公网地址。".to_string())
}

fn extract_tunnel_public_url(line: &str) -> Option<String> {
    line.split_whitespace()
        .find(|part| part.starts_with("https://"))
        .map(|part| part.trim_matches(|ch: char| ch == '"' || ch == '\'' || ch == ','))
        .map(str::to_string)
}

#[cfg(windows)]
fn npx_command() -> &'static str {
    "npx.cmd"
}

#[cfg(not(windows))]
fn npx_command() -> &'static str {
    "npx"
}

fn is_public_callback_request(db: &LocalDb, headers: &[u8]) -> bool {
    let Ok(status) = load_feishu_status(db) else {
        return false;
    };
    let Some(public_host) = status.public_url.as_deref().and_then(public_https_host) else {
        return false;
    };
    header_value(headers, "host")
        .as_deref()
        .and_then(normalize_host)
        .is_some_and(|host| host == public_host)
}

fn public_https_host(url: &str) -> Option<String> {
    let rest = url.trim().strip_prefix("https://")?;
    let host_port = rest.split('/').next()?.trim();
    let host = normalize_host(host_port)?;
    let local_hosts = ["localhost", "127.0.0.1", "0.0.0.0", "::1"];
    (!local_hosts.contains(&host.as_str())).then_some(host)
}

fn normalize_host(value: &str) -> Option<String> {
    let value = value.trim().trim_start_matches('[');
    let host = value
        .trim_end_matches(']')
        .split(':')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    (!host.is_empty() && !host.chars().any(char::is_whitespace)).then_some(host)
}

fn header_value(headers: &[u8], name: &str) -> Option<String> {
    String::from_utf8_lossy(headers).lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

fn verify_sdk_bridge_token(headers: &[u8]) -> Result<(), String> {
    let expected = crate::env::secret_value("ATLAS_FEISHU_WS_BRIDGE_TOKEN");
    let Some(expected) = expected.filter(|value| !value.trim().is_empty()) else {
        return Err("飞书 SDK bridge token 未配置。".to_string());
    };
    let actual = header_value(headers, "x-atlas-feishu-bridge-token")
        .or_else(|| bearer_token(headers))
        .unwrap_or_default();
    if actual == expected {
        Ok(())
    } else {
        Err("飞书 SDK bridge token 不匹配。".to_string())
    }
}

fn bearer_token(headers: &[u8]) -> Option<String> {
    let value = header_value(headers, "authorization")?;
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn is_verified_feishu_source(source: &str) -> bool {
    source == "public_callback" || source == "sdk_websocket"
}

fn request_path(first_line: &str) -> Option<&str> {
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?;
    if method != "POST" {
        return None;
    }
    parts.next()
}

fn default_event_source() -> String {
    "unknown".to_string()
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    String::from_utf8_lossy(headers).lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    })
}

fn bad_request(message: &str) -> FeishuCallbackResult {
    FeishuCallbackResult {
        status_code: 400,
        body: json!({ "code": 400, "msg": message }),
        event: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db() -> LocalDb {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        LocalDb::open(std::env::temp_dir().join(format!("atlas_feishu_{unique}.db"))).unwrap()
    }

    #[test]
    fn feishu_url_verification_returns_challenge() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        let db = temp_db();
        std::env::set_var("FEISHU_VERIFICATION_TOKEN", "verify-token");
        let result = handle_feishu_event_payload(
            &db,
            json!({
                "token": "verify-token",
                "challenge": "abc"
            }),
        )
        .unwrap();
        assert_eq!(result.status_code, 200);
        assert_eq!(
            result.body.get("challenge").and_then(Value::as_str),
            Some("abc")
        );
        std::env::remove_var("FEISHU_VERIFICATION_TOKEN");
    }

    #[test]
    fn feishu_rejects_bad_verification_token() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        let db = temp_db();
        std::env::set_var("FEISHU_VERIFICATION_TOKEN", "verify-token");
        let error = handle_feishu_event_payload(
            &db,
            json!({
                "token": "wrong",
                "challenge": "abc"
            }),
        )
        .unwrap_err();
        assert!(error.contains("verification token"));
        std::env::remove_var("FEISHU_VERIFICATION_TOKEN");
    }

    #[test]
    fn feishu_message_event_is_stored_and_deduped() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        let db = temp_db();
        std::env::set_var("FEISHU_VERIFICATION_TOKEN", "verify-token");
        let payload = json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-1",
                "event_type": "im.message.receive_v1",
                "token": "verify-token"
            },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_test" } },
                "message": {
                    "message_id": "om_test",
                    "chat_id": "oc_test",
                    "message_type": "text",
                    "content": "{\"text\":\"你好 Atlas\"}"
                }
            }
        });
        let first = handle_feishu_event_payload(&db, payload.clone()).unwrap();
        let second = handle_feishu_event_payload(&db, payload).unwrap();
        assert_eq!(
            first.event.as_ref().unwrap().text.as_deref(),
            Some("你好 Atlas")
        );
        assert_eq!(
            second.body.get("msg").and_then(Value::as_str),
            Some("duplicate ignored")
        );
        assert_eq!(list_feishu_events(&db, 10).unwrap().len(), 1);
        std::env::remove_var("FEISHU_VERIFICATION_TOKEN");
    }

    #[test]
    fn feishu_status_reports_missing_receive_requirements() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        let db = temp_db();
        std::env::remove_var("FEISHU_VERIFICATION_TOKEN");
        std::env::remove_var("FEISHU_APP_VERIFICATION_TOKEN");
        std::env::remove_var("ATLAS_FEISHU_PUBLIC_URL");
        std::env::remove_var("FEISHU_PUBLIC_CALLBACK_URL");
        std::env::set_var("ATLAS_TEST_DISABLE_PROJECT_ENV", "1");
        let status = get_feishu_callback_status(&db, false).unwrap();
        assert!(!status.receive_ready);
        assert!(status
            .requirements
            .iter()
            .any(|item| item.id == "receive_channel" && item.status == "missing"));
        assert!(status
            .requirements
            .iter()
            .any(|item| item.id == "verification_token" && item.status == "missing"));
        std::env::remove_var("ATLAS_TEST_DISABLE_PROJECT_ENV");
    }

    #[test]
    fn feishu_setup_links_use_app_id_without_secrets() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        std::env::set_var("FEISHU_APP_ID", "cli_test_app");
        let links = get_feishu_setup_links().unwrap();
        assert_eq!(links.app_id, "cli_test_app");
        assert!(links
            .permission_url
            .starts_with("https://open.feishu.cn/app/cli_test_app/auth?"));
        assert!(links.permission_url.contains("im:chat:readonly"));
        assert!(links
            .permission_url
            .contains("im:message.group_at_msg:readonly"));
        assert_eq!(links.required_event_type, "im.message.receive_v1");
        assert_eq!(
            links.event_url,
            "https://open.feishu.cn/app/cli_test_app/event"
        );
        std::env::remove_var("FEISHU_APP_ID");
    }

    #[test]
    fn feishu_public_url_is_recorded_but_does_not_fake_receive_ready() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        let db = temp_db();
        std::env::set_var("FEISHU_VERIFICATION_TOKEN", "verify-token");
        let status = set_feishu_public_url(
            &db,
            Some("https://callback.example/feishu/events".into()),
            true,
        )
        .unwrap();
        assert_eq!(
            status.public_url.as_deref(),
            Some("https://callback.example/feishu/events")
        );
        assert!(!status.receive_ready);
        assert!(status
            .requirements
            .iter()
            .any(|item| item.id == "real_event" && item.status == "missing"));
        std::env::remove_var("FEISHU_VERIFICATION_TOKEN");
    }

    #[test]
    fn feishu_local_test_event_does_not_mark_public_receive_ready() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        let db = temp_db();
        std::env::set_var("FEISHU_VERIFICATION_TOKEN", "verify-token");
        set_feishu_public_url(
            &db,
            Some("https://callback.example/feishu/events".into()),
            true,
        )
        .unwrap();
        let payload = json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-local-ready-guard",
                "event_type": "im.message.receive_v1",
                "token": "verify-token"
            },
            "event": {
                "message": {
                    "message_id": "om_local_ready_guard",
                    "chat_id": "oc_local",
                    "message_type": "text",
                    "content": "{\"text\":\"本地测试事件\"}"
                }
            }
        });
        let body = payload.to_string();
        let request = format!(
            "POST /feishu/events HTTP/1.1\r\nHost: 127.0.0.1:18080\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let result = handle_raw_http_request(&db, request.as_bytes());
        assert_eq!(result.status_code, 200);
        let status = get_feishu_callback_status(&db, true).unwrap();
        assert_eq!(status.received_count, 1);
        assert!(!status.receive_ready);
        assert_eq!(count_public_feishu_events(&db).unwrap(), 0);
        assert!(status
            .requirements
            .iter()
            .any(|item| item.id == "real_event" && item.status == "missing"));
        std::env::remove_var("FEISHU_VERIFICATION_TOKEN");
    }

    #[test]
    fn feishu_websocket_endpoint_marks_verified_receive_ready() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        let db = temp_db();
        std::env::set_var("FEISHU_VERIFICATION_TOKEN", "verify-token");
        std::env::set_var("ATLAS_FEISHU_WS_BRIDGE_TOKEN", "bridge-token");
        let payload = json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-websocket-ready",
                "event_type": "im.message.receive_v1",
                "token": "verify-token"
            },
            "event": {
                "message": {
                    "message_id": "om_websocket_ready",
                    "chat_id": "oc_websocket",
                    "message_type": "text",
                    "content": "{\"text\":\"长连接事件\"}"
                }
            }
        });
        let body = payload.to_string();
        let request = format!(
            "POST /feishu/sdk-events HTTP/1.1\r\nHost: 127.0.0.1:18080\r\nContent-Type: application/json\r\nX-Atlas-Feishu-Bridge-Token: bridge-token\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let result = handle_raw_http_request(&db, request.as_bytes());
        assert_eq!(result.status_code, 200);
        let status = get_feishu_callback_status(&db, true).unwrap();
        assert!(status.receive_ready);
        assert_eq!(count_public_feishu_events(&db).unwrap(), 0);
        assert_eq!(count_websocket_feishu_events(&db).unwrap(), 1);
        assert_eq!(count_verified_feishu_events(&db).unwrap(), 1);
        assert!(status
            .requirements
            .iter()
            .all(|item| item.status == "ready"));
        std::env::remove_var("ATLAS_FEISHU_WS_BRIDGE_TOKEN");
        std::env::remove_var("FEISHU_VERIFICATION_TOKEN");
    }

    #[test]
    fn feishu_verified_receive_survives_receiver_stop() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        let db = temp_db();
        std::env::set_var("FEISHU_VERIFICATION_TOKEN", "verify-token");
        std::env::set_var("ATLAS_FEISHU_WS_BRIDGE_TOKEN", "bridge-token");
        let payload = json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-websocket-stop-ready",
                "event_type": "im.message.receive_v1",
                "token": "verify-token"
            },
            "event": {
                "message": {
                    "message_id": "om_websocket_stop_ready",
                    "chat_id": "oc_websocket",
                    "message_type": "text",
                    "content": "{\"text\":\"停止后保留真实接收证据\"}"
                }
            }
        });
        let body = payload.to_string();
        let request = format!(
            "POST /feishu/sdk-events HTTP/1.1\r\nHost: 127.0.0.1:18080\r\nContent-Type: application/json\r\nX-Atlas-Feishu-Bridge-Token: bridge-token\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let result = handle_raw_http_request(&db, request.as_bytes());
        assert_eq!(result.status_code, 200);
        assert!(get_feishu_callback_status(&db, true).unwrap().receive_ready);

        let stopped = stop_feishu_callback_server(&db).unwrap();
        assert!(!stopped.running);
        assert!(stopped.receive_ready);
        assert_eq!(count_websocket_feishu_events(&db).unwrap(), 1);
        assert_eq!(count_verified_feishu_events(&db).unwrap(), 1);
        assert!(stopped
            .requirements
            .iter()
            .all(|item| item.status == "ready"));
        std::env::remove_var("ATLAS_FEISHU_WS_BRIDGE_TOKEN");
        std::env::remove_var("FEISHU_VERIFICATION_TOKEN");
    }

    #[test]
    fn feishu_websocket_endpoint_requires_bridge_token() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        let db = temp_db();
        std::env::set_var("FEISHU_VERIFICATION_TOKEN", "verify-token");
        std::env::set_var("ATLAS_FEISHU_WS_BRIDGE_TOKEN", "bridge-token");
        let payload = json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-websocket-reject",
                "event_type": "im.message.receive_v1",
                "token": "verify-token"
            }
        });
        let body = payload.to_string();
        let request = format!(
            "POST /feishu/sdk-events HTTP/1.1\r\nHost: 127.0.0.1:18080\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let result = handle_raw_http_request(&db, request.as_bytes());
        assert_eq!(result.status_code, 400);
        assert_eq!(count_verified_feishu_events(&db).unwrap(), 0);
        std::env::remove_var("ATLAS_FEISHU_WS_BRIDGE_TOKEN");
        std::env::remove_var("FEISHU_VERIFICATION_TOKEN");
    }

    #[test]
    fn feishu_verified_event_upgrades_duplicate_manual_event() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        let db = temp_db();
        std::env::set_var("FEISHU_VERIFICATION_TOKEN", "verify-token");
        let payload = json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-upgrade-real",
                "event_type": "im.message.receive_v1",
                "token": "verify-token"
            },
            "event": {
                "message": {
                    "message_id": "om_upgrade_real",
                    "chat_id": "oc_upgrade",
                    "message_type": "text",
                    "content": "{\"text\":\"升级真实事件\"}"
                }
            }
        });
        let manual = handle_feishu_event_payload(&db, payload.clone()).unwrap();
        assert_eq!(manual.event.as_ref().unwrap().source, "manual_ingest");
        assert_eq!(count_verified_feishu_events(&db).unwrap(), 0);

        set_feishu_public_url(
            &db,
            Some("https://callback.example/feishu/events".into()),
            true,
        )
        .unwrap();
        let body = payload.to_string();
        let request = format!(
            "POST /feishu/events HTTP/1.1\r\nHost: callback.example\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let upgraded = handle_raw_http_request(&db, request.as_bytes());
        assert_eq!(upgraded.status_code, 200);
        assert_eq!(
            upgraded.body.get("msg").and_then(Value::as_str),
            Some("duplicate upgraded")
        );
        let events = list_feishu_events(&db, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].source, "public_callback");
        assert_eq!(count_verified_feishu_events(&db).unwrap(), 1);
        std::env::remove_var("FEISHU_VERIFICATION_TOKEN");
    }

    #[test]
    fn feishu_public_host_event_marks_receive_ready() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        let db = temp_db();
        std::env::set_var("FEISHU_VERIFICATION_TOKEN", "verify-token");
        set_feishu_public_url(
            &db,
            Some("https://callback.example/feishu/events".into()),
            true,
        )
        .unwrap();
        let payload = json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-public-ready",
                "event_type": "im.message.receive_v1",
                "token": "verify-token"
            },
            "event": {
                "message": {
                    "message_id": "om_public_ready",
                    "chat_id": "oc_public",
                    "message_type": "text",
                    "content": "{\"text\":\"公网事件\"}"
                }
            }
        });
        let body = payload.to_string();
        let request = format!(
            "POST /feishu/events HTTP/1.1\r\nHost: callback.example\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let result = handle_raw_http_request(&db, request.as_bytes());
        assert_eq!(result.status_code, 200);
        let status = get_feishu_callback_status(&db, true).unwrap();
        assert!(status.receive_ready);
        assert_eq!(count_public_feishu_events(&db).unwrap(), 1);
        assert!(status
            .requirements
            .iter()
            .all(|item| item.status == "ready"));
        std::env::remove_var("FEISHU_VERIFICATION_TOKEN");
    }

    #[test]
    fn feishu_tunnel_url_parser_accepts_localtunnel_output() {
        assert_eq!(
            extract_tunnel_public_url("your url is: https://atlas-test.loca.lt").as_deref(),
            Some("https://atlas-test.loca.lt")
        );
        assert_eq!(extract_tunnel_public_url("waiting for tunnel"), None);
    }

    #[test]
    fn feishu_tunnel_status_defaults_to_not_running() {
        let db = temp_db();
        let status = get_feishu_tunnel_status(&db, false).unwrap();
        assert!(!status.running);
        assert_eq!(status.provider, "localtunnel");
    }

    #[tokio::test]
    async fn feishu_callback_server_accepts_real_http_event() {
        let _guard = crate::TEST_ENV_LOCK.lock().await;
        let db = temp_db();
        std::env::set_var("FEISHU_VERIFICATION_TOKEN", "verify-token");
        let server = start_feishu_callback_server(db.clone(), Some(0))
            .await
            .unwrap();
        let url = format!("http://127.0.0.1:{}/feishu/events", server.port);
        let payload = json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-http-1",
                "event_type": "im.message.receive_v1",
                "token": "verify-token"
            },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_http" } },
                "message": {
                    "message_id": "om_http",
                    "chat_id": "oc_http",
                    "message_type": "text",
                    "content": "{\"text\":\"HTTP 事件\"}"
                }
            }
        });
        let response = reqwest::Client::new()
            .post(url)
            .json(&payload)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(list_feishu_events(&db, 10).unwrap().len(), 1);
        assert_eq!(
            list_feishu_events(&db, 10).unwrap()[0].text.as_deref(),
            Some("HTTP 事件")
        );
        server.stop();
        let status = stop_feishu_callback_server(&db).unwrap();
        assert!(!status.running);
        std::env::remove_var("FEISHU_VERIFICATION_TOKEN");
    }
}
