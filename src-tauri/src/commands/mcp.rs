use std::path::Path;
use tauri::State;

use crate::mcp::{
    invoke_mcp_tool as invoke_mcp_tool_any, list_mcp_audit_events, list_real_mcp_tools,
    load_mcp_servers, log_mcp_discovery_audit, save_mcp_server as persist_mcp_server,
    set_mcp_server_status, set_mcp_server_trust as persist_mcp_server_trust, McpAuditEvent,
    McpInvokeResult, McpServerConfig, McpServerInput, McpServerStatus,
};
use crate::AppState;

#[tauri::command]
pub async fn get_mcp_servers(state: State<'_, AppState>) -> Result<Vec<McpServerConfig>, String> {
    load_mcp_servers(&state.local_db)
}

#[tauri::command]
pub async fn save_mcp_server(
    payload: McpServerInput,
    state: State<'_, AppState>,
) -> Result<McpServerConfig, String> {
    persist_mcp_server(&state.local_db, payload)
}

#[tauri::command]
pub async fn delete_mcp_server(id: String, state: State<'_, AppState>) -> Result<(), String> {
    crate::mcp::delete_mcp_server(&state.local_db, &id)
}

#[tauri::command]
pub async fn set_mcp_server_trust(
    id: String,
    trusted: bool,
    state: State<'_, AppState>,
) -> Result<McpServerConfig, String> {
    persist_mcp_server_trust(&state.local_db, &id, trusted)
}

#[tauri::command]
pub async fn test_mcp_server(
    id: String,
    state: State<'_, AppState>,
) -> Result<McpServerStatus, String> {
    let servers = load_mcp_servers(&state.local_db)?;
    let server = servers
        .into_iter()
        .find(|server| server.id == id)
        .ok_or_else(|| "没有找到 MCP 服务。".to_string())?;

    let result = match server.transport.as_str() {
        "stdio" => {
            let command = server.command.as_deref().unwrap_or_default();
            if command_exists(command) {
                match list_real_mcp_tools(&server).await {
                    Ok(tools) => {
                        let updated_server = set_mcp_server_status(
                            &state.local_db,
                            &server.id,
                            Some("ready".to_string()),
                            None,
                        )?;
                        log_mcp_discovery_audit(
                            &state.local_db,
                            &updated_server,
                            "ok",
                            "stdio MCP tools/list 握手成功。",
                        )?;
                        Ok(McpServerStatus {
                            server: updated_server,
                            status: "ready".to_string(),
                            message: "stdio MCP 已完成 tools/list 握手。".to_string(),
                            tools,
                            resources: Vec::new(),
                            prompts: Vec::new(),
                        })
                    }
                    Err(error) => Err(error),
                }
            } else {
                Err(format!("找不到 stdio MCP 命令：{command}"))
            }
        }
        "http_sse" => match list_real_mcp_tools(&server).await {
            Ok(tools) => {
                let updated_server = set_mcp_server_status(
                    &state.local_db,
                    &server.id,
                    Some("ready".to_string()),
                    None,
                )?;
                log_mcp_discovery_audit(
                    &state.local_db,
                    &updated_server,
                    "ok",
                    "HTTP/SSE MCP tools/list 请求成功。",
                )?;
                Ok(McpServerStatus {
                    server: updated_server,
                    status: "ready".to_string(),
                    message: "HTTP/SSE MCP 已完成 tools/list 请求。".to_string(),
                    tools,
                    resources: Vec::new(),
                    prompts: Vec::new(),
                })
            }
            Err(error) => Err(error),
        },
        other => Err(format!("不支持的 MCP transport：{other}")),
    };

    match result {
        Ok(status) => Ok(status),
        Err(error) => {
            let failed_server = set_mcp_server_status(
                &state.local_db,
                &server.id,
                Some("failed".to_string()),
                Some(error.clone()),
            )
            .ok();
            if let Some(failed_server) = failed_server {
                log_mcp_discovery_audit(&state.local_db, &failed_server, "failed", &error).ok();
            }
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn invoke_mcp_tool(
    server_id: String,
    tool_name: String,
    arguments: serde_json::Value,
    confirmed: Option<bool>,
    state: State<'_, AppState>,
) -> Result<McpInvokeResult, String> {
    invoke_mcp_tool_any(
        &state.local_db,
        &server_id,
        &tool_name,
        arguments,
        confirmed.unwrap_or(false),
    )
    .await
}

#[tauri::command]
pub async fn get_mcp_audit_events(
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<McpAuditEvent>, String> {
    list_mcp_audit_events(&state.local_db, limit.unwrap_or(80))
}

fn command_exists(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }
    let path = Path::new(command);
    if path.is_absolute() || command.contains('\\') || command.contains('/') {
        return path.exists();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let direct = dir.join(command);
        direct.exists()
            || direct.with_extension("exe").exists()
            || direct.with_extension("cmd").exists()
    })
}
