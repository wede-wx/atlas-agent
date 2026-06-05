use tauri::State;

use crate::feishu::{
    get_feishu_callback_status as load_feishu_callback_status,
    get_feishu_setup_links as load_feishu_setup_links,
    get_feishu_tunnel_status as load_feishu_tunnel_status, handle_feishu_event_payload,
    list_feishu_events, set_feishu_public_url as store_feishu_public_url,
    start_feishu_callback_server as start_server, start_feishu_public_tunnel as start_tunnel,
    stop_feishu_callback_server as stop_server, FeishuCallbackResult, FeishuCallbackStatus,
    FeishuReceivedEvent, FeishuSetupLinks, FeishuTunnelStatus,
};
use crate::AppState;

#[tauri::command]
pub async fn start_feishu_callback_server(
    port: Option<u16>,
    state: State<'_, AppState>,
) -> Result<FeishuCallbackStatus, String> {
    {
        let running = state.feishu_callback_server.lock().await;
        if let Some(server) = running.as_ref() {
            return load_feishu_callback_status(&state.local_db, true).map(|mut status| {
                status.local_url = Some(format!("http://127.0.0.1:{}/feishu/events", server.port));
                status
            });
        }
    }
    let server = start_server(state.local_db.clone(), port).await?;
    let status = load_feishu_callback_status(&state.local_db, true)?;
    let mut running = state.feishu_callback_server.lock().await;
    *running = Some(server);
    Ok(status)
}

#[tauri::command]
pub async fn stop_feishu_callback_server(
    state: State<'_, AppState>,
) -> Result<FeishuCallbackStatus, String> {
    let mut tunnel = state.feishu_tunnel_process.lock().await;
    if let Some(process) = tunnel.take() {
        let _ = process.stop(&state.local_db).await;
    }
    drop(tunnel);
    let mut running = state.feishu_callback_server.lock().await;
    if let Some(server) = running.take() {
        server.stop();
    }
    stop_server(&state.local_db)
}

#[tauri::command]
pub async fn get_feishu_callback_status(
    state: State<'_, AppState>,
) -> Result<FeishuCallbackStatus, String> {
    let running = state.feishu_callback_server.lock().await.is_some();
    load_feishu_callback_status(&state.local_db, running)
}

#[tauri::command]
pub async fn set_feishu_public_url(
    public_url: Option<String>,
    state: State<'_, AppState>,
) -> Result<FeishuCallbackStatus, String> {
    let running = state.feishu_callback_server.lock().await.is_some();
    store_feishu_public_url(&state.local_db, public_url, running)
}

#[tauri::command]
pub async fn get_feishu_setup_links() -> Result<FeishuSetupLinks, String> {
    load_feishu_setup_links()
}

#[tauri::command]
pub async fn start_feishu_public_tunnel(
    state: State<'_, AppState>,
) -> Result<FeishuTunnelStatus, String> {
    let mut tunnel_running = state.feishu_tunnel_process.lock().await;
    if let Some(process) = tunnel_running.as_ref() {
        return Ok(process.status.clone());
    }
    let port = {
        let running = state.feishu_callback_server.lock().await;
        running.as_ref().map(|server| server.port)
    };
    let port = match port {
        Some(port) => port,
        None => {
            let server = start_server(state.local_db.clone(), None).await?;
            let port = server.port;
            let mut running = state.feishu_callback_server.lock().await;
            *running = Some(server);
            port
        }
    };
    let process = start_tunnel(state.local_db.clone(), port).await?;
    let status = process.status.clone();
    *tunnel_running = Some(process);
    Ok(status)
}

#[tauri::command]
pub async fn stop_feishu_public_tunnel(
    state: State<'_, AppState>,
) -> Result<FeishuTunnelStatus, String> {
    let mut running = state.feishu_tunnel_process.lock().await;
    if let Some(process) = running.take() {
        return process.stop(&state.local_db).await;
    }
    load_feishu_tunnel_status(&state.local_db, false)
}

#[tauri::command]
pub async fn get_feishu_tunnel_status(
    state: State<'_, AppState>,
) -> Result<FeishuTunnelStatus, String> {
    let running = state.feishu_tunnel_process.lock().await.is_some();
    load_feishu_tunnel_status(&state.local_db, running)
}

#[tauri::command]
pub async fn get_feishu_received_events(
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<FeishuReceivedEvent>, String> {
    list_feishu_events(&state.local_db, limit.unwrap_or(80))
}

#[tauri::command]
pub async fn ingest_feishu_event_payload(
    payload: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<FeishuCallbackResult, String> {
    handle_feishu_event_payload(&state.local_db, payload)
}
