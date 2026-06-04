use std::fs;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{State, Window};

use crate::{
    config::{normalize_base_url, Config, ModelConnectionConfig, UiConfig},
    AppState,
};

#[derive(Debug, Deserialize)]
pub struct SaveConfigPayload {
    #[serde(default)]
    pub connection_id: Option<String>,
    pub provider: String,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub route_id: Option<String>,
    #[serde(default)]
    pub connection_name: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    pub api_url: String,
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: bool,
    pub model_name: String,
    #[serde(default)]
    pub auth_header: Option<String>,
    pub theme: String,
    #[serde(default)]
    pub sound_enabled: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelSettingsPayload {
    #[serde(default)]
    pub connection_id: Option<String>,
    pub provider: String,
    #[serde(default, alias = "providerId")]
    pub provider_id: Option<String>,
    #[serde(default, alias = "routeId")]
    pub route_id: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default, alias = "apiUrl")]
    pub api_url: String,
    #[serde(default, alias = "apiKey")]
    pub api_key: Option<String>,
    #[serde(default, alias = "clearApiKey")]
    pub clear_api_key: bool,
    #[serde(default, alias = "modelName")]
    pub model_name: String,
    #[serde(default, alias = "authHeader")]
    pub auth_header: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSettingsResult {
    pub status: String,
    pub message: String,
    pub models: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BackendStatus {
    pub provider: String,
    pub provider_supported: bool,
    pub api_key_configured: bool,
    pub autostart_available: bool,
    pub notifications_available: bool,
    pub updater_available: bool,
    pub local_scan_available: bool,
    pub tts_available: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSmokeProofPayload {
    pub section: String,
    pub title: String,
    #[serde(default)]
    pub source: Option<String>,
}

fn settings_persistence_smoke_enabled() -> bool {
    if std::env::var("AURA_SMOKE_EXERCISE_SETTINGS_PERSISTENCE")
        .ok()
        .as_deref()
        != Some("1")
    {
        return false;
    }
    let Some(aura_home) = std::env::var("AURA_HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    aura_home.to_ascii_lowercase().contains("tauri-smoke")
}

fn settings_domain_smoke_enabled() -> bool {
    if std::env::var("AURA_SMOKE_EXERCISE_SETTINGS_19_31")
        .ok()
        .as_deref()
        != Some("1")
    {
        return false;
    }
    let Some(aura_home) = std::env::var("AURA_HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    aura_home.to_ascii_lowercase().contains("tauri-smoke")
}

fn agent_workbench_smoke_enabled() -> bool {
    if std::env::var("AURA_SMOKE_EXERCISE_AGENT_WORKBENCH")
        .ok()
        .as_deref()
        != Some("1")
    {
        return false;
    }
    let Some(aura_home) = std::env::var("AURA_HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    aura_home.to_ascii_lowercase().contains("tauri-smoke")
}

fn agent_cancel_smoke_base_url() -> Option<String> {
    if !agent_workbench_smoke_enabled() {
        return None;
    }
    if std::env::var("AURA_SMOKE_EXERCISE_AGENT_CANCEL")
        .ok()
        .as_deref()
        != Some("1")
    {
        return None;
    }
    std::env::var("AURA_SMOKE_CANCELLABLE_LLM_BASE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| {
            value.starts_with("http://127.0.0.1:") || value.starts_with("http://localhost:")
        })
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<Config, String> {
    Ok(state.config.lock().await.redacted_for_client())
}

#[tauri::command]
pub async fn save_config(
    config: SaveConfigPayload,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let current = state.config.lock().await.clone();
    let config = config.into_config(Some(&current));
    config.save().map_err(|e| e.to_string())?;
    // P0-3: keep the live outbound network policy in sync with the saved config.
    crate::tools::outbound::set_active_policy(config.outbound.clone());
    let mut state_config = state.config.lock().await;
    *state_config = config;
    Ok(())
}

#[tauri::command]
pub async fn get_backend_status(state: State<'_, AppState>) -> Result<BackendStatus, String> {
    let config = state.config.lock().await;
    let active = config.llm.active_connection();
    let provider = active
        .map(|connection| connection.provider_id.clone())
        .unwrap_or_else(|| config.llm.default_provider.clone());
    let provider_supported = active
        .map(|connection| {
            matches!(
                connection.protocol.as_str(),
                "openai-compatible" | "anthropic"
            )
        })
        .unwrap_or(false);
    let api_key_configured = active
        .map(|connection| connection.is_local_runtime() || !connection.api_key.trim().is_empty())
        .unwrap_or(false);

    Ok(BackendStatus {
        provider,
        provider_supported,
        api_key_configured,
        autostart_available: false,
        notifications_available: false,
        updater_available: false,
        local_scan_available: true,
        tts_available: false,
    })
}

#[tauri::command]
pub async fn write_settings_smoke_proof(
    window: Window,
    payload: SettingsSmokeProofPayload,
) -> Result<Value, String> {
    let smoke_run_id = std::env::var("AURA_SMOKE_RUN_ID").unwrap_or_default();
    if std::env::var("AURA_SMOKE_ENABLE_SETTINGS_PROOF")
        .ok()
        .as_deref()
        != Some("1")
        || smoke_run_id.trim().is_empty()
    {
        return Ok(json!({ "ok": false, "enabled": false }));
    }
    if window.label() != "main" {
        return Err("settings smoke proof can only be written from the main window".to_string());
    }

    let proof = json!({
        "ok": true,
        "kind": "settings_open_smoke_proof",
        "smokeRunId": smoke_run_id,
        "windowLabel": window.label(),
        "section": truncate_smoke_text(&payload.section, 80),
        "title": truncate_smoke_text(&payload.title, 120),
        "source": truncate_smoke_text(payload.source.as_deref().unwrap_or("settings"), 80),
        "exercisePersistence": settings_persistence_smoke_enabled(),
        "exerciseSettings19To31": settings_domain_smoke_enabled(),
    });
    let path = std::env::temp_dir().join(format!(
        "aura-settings-open-smoke-{}-{}.json",
        proof
            .get("smokeRunId")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        uuid::Uuid::new_v4()
    ));
    let bytes = serde_json::to_vec(&proof).map_err(|error| error.to_string())?;
    fs::write(&path, bytes).map_err(|error| error.to_string())?;
    Ok(json!({
        "ok": true,
        "path": path,
        "exercisePersistence": settings_persistence_smoke_enabled(),
        "exerciseSettings19To31": settings_domain_smoke_enabled(),
    }))
}

#[tauri::command]
pub async fn write_settings_persistence_smoke_proof(
    window: Window,
    payload: Value,
) -> Result<Value, String> {
    let smoke_run_id = std::env::var("AURA_SMOKE_RUN_ID").unwrap_or_default();
    if std::env::var("AURA_SMOKE_ENABLE_SETTINGS_PROOF")
        .ok()
        .as_deref()
        != Some("1")
        || smoke_run_id.trim().is_empty()
        || !settings_persistence_smoke_enabled()
    {
        return Ok(json!({ "ok": false, "enabled": false }));
    }
    if window.label() != "main" {
        return Err(
            "settings persistence smoke proof can only be written from the main window".to_string(),
        );
    }

    let mut proof = payload
        .as_object()
        .cloned()
        .unwrap_or_else(serde_json::Map::new);
    proof.insert(
        "kind".to_string(),
        Value::String("settings_persistence_smoke_proof".to_string()),
    );
    proof.insert("smokeRunId".to_string(), Value::String(smoke_run_id));
    proof.insert(
        "windowLabel".to_string(),
        Value::String(window.label().to_string()),
    );
    proof.insert(
        "auraHomeIsolated".to_string(),
        Value::Bool(settings_persistence_smoke_enabled()),
    );

    let proof = Value::Object(proof);
    let path = std::env::temp_dir().join(format!(
        "aura-settings-persistence-smoke-{}-{}.json",
        proof
            .get("smokeRunId")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        uuid::Uuid::new_v4()
    ));
    let bytes = serde_json::to_vec(&proof).map_err(|error| error.to_string())?;
    fs::write(&path, bytes).map_err(|error| error.to_string())?;
    Ok(json!({ "ok": true, "path": path }))
}

#[tauri::command]
pub async fn write_settings_domain_smoke_proof(
    window: Window,
    payload: Value,
) -> Result<Value, String> {
    let smoke_run_id = std::env::var("AURA_SMOKE_RUN_ID").unwrap_or_default();
    if std::env::var("AURA_SMOKE_ENABLE_SETTINGS_PROOF")
        .ok()
        .as_deref()
        != Some("1")
        || smoke_run_id.trim().is_empty()
        || !settings_domain_smoke_enabled()
    {
        return Ok(json!({ "ok": false, "enabled": false }));
    }
    if window.label() != "main" {
        return Err(
            "settings domain smoke proof can only be written from the main window".to_string(),
        );
    }

    let mut proof = payload
        .as_object()
        .cloned()
        .unwrap_or_else(serde_json::Map::new);
    proof.insert(
        "kind".to_string(),
        Value::String("settings_domain_smoke_proof".to_string()),
    );
    proof.insert("smokeRunId".to_string(), Value::String(smoke_run_id));
    proof.insert(
        "windowLabel".to_string(),
        Value::String(window.label().to_string()),
    );
    proof.insert(
        "auraHomeIsolated".to_string(),
        Value::Bool(settings_domain_smoke_enabled()),
    );

    let proof = Value::Object(proof);
    let path = std::env::temp_dir().join(format!(
        "aura-settings-domain-smoke-{}-{}.json",
        proof
            .get("smokeRunId")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        uuid::Uuid::new_v4()
    ));
    let bytes = serde_json::to_vec(&proof).map_err(|error| error.to_string())?;
    fs::write(&path, bytes).map_err(|error| error.to_string())?;
    Ok(json!({ "ok": true, "path": path }))
}

#[tauri::command]
pub async fn write_agent_workbench_smoke_proof(
    window: Window,
    payload: Value,
) -> Result<Value, String> {
    let smoke_run_id = std::env::var("AURA_SMOKE_RUN_ID").unwrap_or_default();
    if smoke_run_id.trim().is_empty() || !agent_workbench_smoke_enabled() {
        return Ok(json!({ "ok": false, "enabled": false }));
    }
    if window.label() != "main" {
        return Err(
            "agent workbench smoke proof can only be written from the main window".to_string(),
        );
    }

    let mut proof = payload
        .as_object()
        .cloned()
        .unwrap_or_else(serde_json::Map::new);
    proof.insert(
        "kind".to_string(),
        Value::String("agent_workbench_smoke_proof".to_string()),
    );
    proof.insert("smokeRunId".to_string(), Value::String(smoke_run_id));
    proof.insert(
        "windowLabel".to_string(),
        Value::String(window.label().to_string()),
    );
    proof.insert(
        "auraHomeIsolated".to_string(),
        Value::Bool(agent_workbench_smoke_enabled()),
    );
    if let Some(phase) = proof.get_mut("phase") {
        if let Some(value) = phase.as_str() {
            *phase = Value::String(truncate_smoke_text(value, 80));
        }
    }

    let proof = Value::Object(proof);
    let path = std::env::temp_dir().join(format!(
        "aura-agent-workbench-smoke-{}-{}.json",
        proof
            .get("smokeRunId")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        uuid::Uuid::new_v4()
    ));
    let bytes = serde_json::to_vec(&proof).map_err(|error| error.to_string())?;
    fs::write(&path, bytes).map_err(|error| error.to_string())?;
    Ok(json!({
        "ok": true,
        "enabled": true,
        "path": path,
        "phase": proof.get("phase").cloned().unwrap_or(Value::Null),
        "exerciseCancel": agent_cancel_smoke_base_url().is_some(),
        "cancellableBaseUrl": agent_cancel_smoke_base_url(),
    }))
}

#[tauri::command]
pub async fn check_model_settings(
    payload: ModelSettingsPayload,
    state: State<'_, AppState>,
) -> Result<ModelSettingsResult, String> {
    let config = state.config.lock().await;
    let settings = payload.normalized(Some(&config));
    match settings.protocol.as_str() {
        "anthropic" => check_anthropic_model(&settings).await,
        "openai-compatible" => check_openai_compatible_model(&settings).await,
        other => Err(format!("unsupported model protocol: {other}")),
    }
}

#[tauri::command]
pub async fn list_models(
    payload: ModelSettingsPayload,
    state: State<'_, AppState>,
) -> Result<ModelSettingsResult, String> {
    let config = state.config.lock().await;
    let settings = payload.normalized(Some(&config));
    match settings.protocol.as_str() {
        "anthropic" | "openai-compatible" => list_provider_models(&settings).await,
        other => Err(format!("unsupported model protocol: {other}")),
    }
}

impl SaveConfigPayload {
    fn into_config(self, current: Option<&Config>) -> Config {
        let mut config = current.cloned().unwrap_or_default();
        config.ui = UiConfig {
            theme: if self.theme == "light" {
                "light".to_string()
            } else {
                "dark".to_string()
            },
            sound_enabled: self.sound_enabled,
        };

        let provider_id = provider_id_from_payload(
            self.provider_id.as_deref(),
            self.provider.as_str(),
            self.api_url.as_str(),
            self.model_name.as_str(),
        );
        let protocol = normalize_protocol(self.protocol.as_deref(), &provider_id);
        let route_id = self
            .route_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string())
            .unwrap_or_else(|| default_route_for(&provider_id, &protocol).to_string());
        let connection_id = self
            .connection_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string())
            .unwrap_or_else(|| format!("{provider_id}:{route_id}"));
        let existing = config
            .llm
            .connections
            .iter()
            .find(|connection| connection.id == connection_id)
            .cloned();
        let model = if self.model_name.trim().is_empty() {
            existing
                .as_ref()
                .map(|connection| connection.model.clone())
                .unwrap_or_else(|| {
                    default_model_for(&provider_id, &route_id, &protocol).to_string()
                })
        } else {
            self.model_name.trim().to_string()
        };
        let api_key = self.api_key_for_connection(&connection_id, &provider_id, &route_id, current);
        let base_url = if self.api_url.trim().is_empty() {
            existing
                .as_ref()
                .and_then(|connection| connection.base_url.clone())
                .or_else(|| default_base_url_for(&provider_id, &route_id).map(str::to_string))
        } else {
            Some(normalize_model_base_url(
                &protocol,
                &provider_id,
                &route_id,
                self.api_url.trim(),
            ))
        };

        let connection = ModelConnectionConfig {
            id: connection_id.clone(),
            name: self
                .connection_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| value.trim().to_string())
                .or_else(|| existing.as_ref().map(|connection| connection.name.clone()))
                .unwrap_or_else(|| default_provider_name(&provider_id).to_string()),
            provider_id: provider_id.clone(),
            route_id,
            protocol,
            api_key,
            model,
            base_url,
            enabled: true,
            auth_header: self
                .auth_header
                .as_deref()
                .map(normalize_auth_header)
                .or_else(|| existing.and_then(|connection| connection.auth_header)),
        };
        config.llm.upsert_connection(connection);
        config.llm.default_provider = provider_id;
        config.llm.default_connection_id = Some(connection_id);
        config.llm.sync_legacy_slots_from_connections();
        config
    }

    fn api_key_for_connection(
        &self,
        connection_id: &str,
        provider_id: &str,
        route_id: &str,
        current: Option<&Config>,
    ) -> String {
        if self.clear_api_key {
            return String::new();
        }
        if let Some(api_key) = self
            .api_key
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return api_key.to_string();
        }
        current
            .and_then(|config| {
                config.llm.connections.iter().find(|connection| {
                    connection.id == connection_id
                        && connection.provider_id == provider_id
                        && connection.route_id == route_id
                })
            })
            .map(|connection| connection.api_key.clone())
            .unwrap_or_default()
    }
}

impl ModelSettingsPayload {
    fn normalized(self, current: Option<&Config>) -> NormalizedModelSettings {
        let provider_id = provider_id_from_payload(
            self.provider_id.as_deref(),
            self.provider.as_str(),
            self.api_url.as_str(),
            self.model_name.as_str(),
        );
        let protocol = normalize_protocol(self.protocol.as_deref(), &provider_id);
        let route_id = self
            .route_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string())
            .unwrap_or_else(|| default_route_for(&provider_id, &protocol).to_string());
        let model = self.model_name.trim().to_string();
        let api_url =
            normalize_model_base_url(&protocol, &provider_id, &route_id, self.api_url.trim());
        let connection_id = self
            .connection_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string());
        let saved_connection = current.and_then(|config| {
            connection_id
                .as_deref()
                .and_then(|id| {
                    config.llm.connections.iter().find(|connection| {
                        connection.id == id
                            && connection.provider_id == provider_id
                            && connection.route_id == route_id
                    })
                })
                .or_else(|| {
                    config.llm.connections.iter().find(|connection| {
                        connection.provider_id == provider_id && connection.route_id == route_id
                    })
                })
        });
        let api_key = if self.clear_api_key {
            String::new()
        } else if let Some(api_key) = self
            .api_key
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            api_key.to_string()
        } else {
            saved_connection
                .map(|connection| connection.api_key.clone())
                .unwrap_or_default()
        };
        NormalizedModelSettings {
            provider_id,
            route_id,
            protocol,
            api_url,
            api_key,
            model,
            auth_header: self.auth_header.as_deref().map(normalize_auth_header),
        }
    }
}

#[derive(Debug, Clone)]
struct NormalizedModelSettings {
    provider_id: String,
    route_id: String,
    protocol: String,
    api_url: String,
    api_key: String,
    model: String,
    auth_header: Option<String>,
}

async fn check_openai_compatible_model(
    settings: &NormalizedModelSettings,
) -> Result<ModelSettingsResult, String> {
    if settings.api_url.trim().is_empty() {
        return Err("请填写 Base URL。".to_string());
    }
    if settings.model.trim().is_empty() {
        return Err("请先获取模型列表并选择模型，或手动填写模型名称。".to_string());
    }
    if settings.api_key.trim().is_empty() && !model_settings_allows_empty_key(settings) {
        return Err("请先填写 API Key，再测试连接。".to_string());
    }
    let url = format!(
        "{}/chat/completions",
        settings.api_url.trim_end_matches('/')
    );
    let mut request = Client::new().post(url).json(&json!({
        "model": settings.model,
        "messages": [{ "role": "user", "content": "Reply with OK." }],
        "max_tokens": 8,
        "stream": false
    }));
    if !settings.api_key.is_empty() {
        request = apply_openai_auth_header(request, settings);
    }

    let response = request.send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(upstream_error("模型测试失败", response).await);
    }

    Ok(ModelSettingsResult {
        status: "passed".to_string(),
        message: format!(
            "模型测试通过：{} / {}。",
            settings.provider_id, settings.route_id
        ),
        models: vec![settings.model.clone()],
    })
}

async fn check_anthropic_model(
    settings: &NormalizedModelSettings,
) -> Result<ModelSettingsResult, String> {
    if settings.api_url.trim().is_empty() {
        return Err("请填写 Base URL。".to_string());
    }
    if settings.model.trim().is_empty() {
        return Err("请先获取模型列表并选择模型，或手动填写模型名称。".to_string());
    }
    if settings.api_key.is_empty() {
        return Err("缺少 API Key：Anthropic。".to_string());
    }
    let url = format!("{}/messages", settings.api_url.trim_end_matches('/'));
    let response = Client::new()
        .post(url)
        .header("x-api-key", &settings.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": settings.model,
            "messages": [{ "role": "user", "content": "Reply with OK." }],
            "max_tokens": 8
        }))
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.status().is_success() {
        return Err(upstream_error("模型测试失败", response).await);
    }

    Ok(ModelSettingsResult {
        status: "passed".to_string(),
        message: format!(
            "模型测试通过：{} / {}。",
            settings.provider_id, settings.route_id
        ),
        models: vec![settings.model.clone()],
    })
}

async fn list_provider_models(
    settings: &NormalizedModelSettings,
) -> Result<ModelSettingsResult, String> {
    if settings.api_url.trim().is_empty() {
        return Err("请填写 Base URL。".to_string());
    }
    if settings.api_key.trim().is_empty() && !model_settings_allows_empty_key(settings) {
        return Err("请先填写 API Key，再获取模型列表。".to_string());
    }
    let url = format!("{}/models", settings.api_url.trim_end_matches('/'));
    let mut request = Client::new().get(url);
    if !settings.api_key.is_empty() {
        request = if settings.protocol == "anthropic" {
            request
                .header("x-api-key", &settings.api_key)
                .header("anthropic-version", "2023-06-01")
        } else {
            apply_openai_auth_header(request, settings)
        };
    }

    let response = request.send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(upstream_error("模型列表检测失败", response).await);
    }

    let value = response
        .json::<Value>()
        .await
        .map_err(|error| format!("模型列表响应解析失败：{error}"))?;
    let models = extract_model_ids(&value);
    Ok(ModelSettingsResult {
        status: if models.is_empty() { "empty" } else { "passed" }.to_string(),
        message: if models.is_empty() {
            "没有检测到模型，请检查 URL、密钥或本地服务。".to_string()
        } else {
            format!("检测到 {} 个模型。", models.len())
        },
        models,
    })
}

fn model_settings_allows_empty_key(settings: &NormalizedModelSettings) -> bool {
    let base = settings.api_url.to_ascii_lowercase();
    matches!(settings.provider_id.as_str(), "ollama" | "lmstudio")
        || base.contains("localhost")
        || base.contains("127.0.0.1")
}

async fn upstream_error(prefix: &str, response: reqwest::Response) -> String {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        format!("{prefix}：HTTP {status}")
    } else {
        format!("{prefix}：HTTP {status} {trimmed}")
    }
}

fn extract_model_ids(value: &Value) -> Vec<String> {
    let raw_models = value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    raw_models
        .into_iter()
        .filter_map(|item| {
            if let Some(id) = item.as_str() {
                return Some(id.to_string());
            }
            item.get("id")
                .or_else(|| item.get("name"))
                .or_else(|| item.get("model"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn apply_openai_auth_header(
    request: reqwest::RequestBuilder,
    settings: &NormalizedModelSettings,
) -> reqwest::RequestBuilder {
    match settings.auth_header.as_deref() {
        Some("api-key") => request.header("api-key", &settings.api_key),
        Some("x-api-key") => request.header("x-api-key", &settings.api_key),
        _ => request.bearer_auth(&settings.api_key),
    }
}

fn provider_id_from_payload(
    explicit: Option<&str>,
    provider: &str,
    api_url: &str,
    model: &str,
) -> String {
    if let Some(value) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        return normalize_provider_id(value);
    }
    let provider = normalize_provider_id(provider);
    if provider != "openai" {
        return provider;
    }
    infer_provider_id(api_url, model)
}

fn normalize_provider_id(provider: &str) -> String {
    match provider.trim() {
        "deepseek" => "deepseek".to_string(),
        "anthropic" | "claude" => "anthropic".to_string(),
        "ollama" => "ollama".to_string(),
        "lmstudio" | "lm-studio" => "lmstudio".to_string(),
        "xiaomi" | "mimo" => "xiaomi-mimo".to_string(),
        other if other.trim().is_empty() => "openai".to_string(),
        other => other.to_string(),
    }
}

fn normalize_protocol(protocol: Option<&str>, provider_id: &str) -> String {
    match protocol.map(str::trim).filter(|value| !value.is_empty()) {
        Some("anthropic") | Some("claude") => "anthropic".to_string(),
        Some("openai") | Some("openai-compatible") => "openai-compatible".to_string(),
        _ if provider_id == "anthropic" => "anthropic".to_string(),
        _ => "openai-compatible".to_string(),
    }
}

fn normalize_auth_header(value: &str) -> String {
    match value.trim() {
        "api-key" => "api-key".to_string(),
        "x-api-key" => "x-api-key".to_string(),
        _ => "authorization".to_string(),
    }
}

fn infer_provider_id(api_url: &str, model: &str) -> String {
    let base = api_url.to_lowercase();
    let model = model.to_lowercase();
    if base.contains("xiaomimimo.com") || model.starts_with("mimo-") {
        "xiaomi-mimo".to_string()
    } else if base.contains("deepseek.com") || model.contains("deepseek") {
        "deepseek".to_string()
    } else if base.contains("dashscope") || model.starts_with("qwen") {
        "aliyun-bailian".to_string()
    } else if base.contains("ark.cn-beijing.volces.com") || model.contains("doubao") {
        "volcengine-ark".to_string()
    } else if base.contains("bigmodel.cn") || model.starts_with("glm-") {
        "zai".to_string()
    } else if base.contains("moonshot.ai") || model.contains("kimi") || model.contains("moonshot") {
        "moonshot-kimi".to_string()
    } else if base.contains("qianfan") || model.contains("ernie") {
        "baidu-qianfan".to_string()
    } else if base.contains("hunyuan") || model.contains("hunyuan") {
        "tencent-hunyuan".to_string()
    } else if base.contains("minimaxi.com")
        || base.contains("minimax.io")
        || model.starts_with("minimax-")
    {
        "minimax".to_string()
    } else if base.contains("siliconflow.cn") {
        "siliconflow".to_string()
    } else if base.contains("generativelanguage.googleapis.com") || model.contains("gemini") {
        "gemini".to_string()
    } else if base.contains("openrouter.ai") {
        "openrouter".to_string()
    } else if base.contains("localhost:11434") {
        "ollama".to_string()
    } else if base.contains("localhost:1234") {
        "lmstudio".to_string()
    } else if base.contains("anthropic.com") || model.contains("claude") {
        "anthropic".to_string()
    } else {
        "openai".to_string()
    }
}

fn default_provider_name(provider_id: &str) -> &str {
    match provider_id {
        "xiaomi-mimo" => "小米 MiMo",
        "deepseek" => "DeepSeek",
        "aliyun-bailian" => "阿里云百炼",
        "volcengine-ark" => "火山方舟",
        "zai" => "智谱 AI / Z.ai",
        "moonshot-kimi" => "Kimi / 月之暗面",
        "baidu-qianfan" => "百度千帆",
        "tencent-hunyuan" => "腾讯混元",
        "minimax" => "MiniMax",
        "siliconflow" => "硅基流动",
        "anthropic" => "Anthropic",
        "gemini" => "Gemini",
        "openrouter" => "OpenRouter",
        "ollama" => "Ollama",
        "lmstudio" => "LM Studio",
        "custom" => "自定义",
        _ => "OpenAI",
    }
}

fn default_route_for(provider_id: &str, protocol: &str) -> &'static str {
    match provider_id {
        "xiaomi-mimo" => "mimo-standard",
        "deepseek" => "deepseek-openai",
        "aliyun-bailian" => "bailian-cn",
        "volcengine-ark" => "ark-standard",
        "zai" => "zai-openai",
        "moonshot-kimi" => "kimi-openai",
        "baidu-qianfan" => "qianfan-cn",
        "tencent-hunyuan" => "hunyuan-openai",
        "minimax" => "minimax-cn",
        "siliconflow" => "siliconflow-openai",
        "anthropic" => "anthropic-default",
        "gemini" => "gemini-openai",
        "openrouter" => "openrouter-default",
        "ollama" => "ollama-local",
        "lmstudio" => "lmstudio-local",
        "custom" if protocol == "anthropic" => "custom-anthropic",
        "custom" => "custom-openai",
        _ => "openai-default",
    }
}

fn default_model_for(provider_id: &str, route_id: &str, protocol: &str) -> &'static str {
    match (provider_id, route_id) {
        ("xiaomi-mimo", _) => "mimo-v4-flash",
        ("deepseek", _) => "deepseek-chat",
        ("aliyun-bailian", _) => "qwen-plus",
        ("volcengine-ark", _) => "doubao-seed-1-6",
        ("zai", _) => "glm-4.5-flash",
        ("moonshot-kimi", _) => "kimi-k2-0905-preview",
        ("baidu-qianfan", _) => "ernie-4.5-turbo-128k",
        ("tencent-hunyuan", _) => "hunyuan-turbos-latest",
        ("minimax", _) => "MiniMax-M2.7",
        ("siliconflow", _) => "Qwen/Qwen3-Coder-480B-A35B-Instruct",
        ("anthropic", _) => "claude-opus-4-8",
        ("gemini", _) => "gemini-2.5-flash",
        ("openrouter", _) => "openai/gpt-5.2",
        ("ollama", _) => "qwen2.5:7b",
        ("lmstudio", _) => "local-model",
        ("custom", _) if protocol == "anthropic" => "claude-compatible-model",
        ("custom", "custom-gemini") => "gemini-compatible-model",
        ("custom", _) => "custom-model",
        _ => "gpt-4o-mini",
    }
}

fn default_base_url_for(provider_id: &str, route_id: &str) -> Option<&'static str> {
    match (provider_id, route_id) {
        ("xiaomi-mimo", "mimo-standard") => Some("https://api.xiaomimimo.com/v1"),
        ("xiaomi-mimo", "mimo-token-plan") => None,
        ("deepseek", _) => Some("https://api.deepseek.com/v1"),
        ("aliyun-bailian", "bailian-intl") => {
            Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1")
        }
        ("aliyun-bailian", _) => Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
        ("volcengine-ark", "ark-coding-plan") => {
            Some("https://ark.cn-beijing.volces.com/api/coding/v3")
        }
        ("volcengine-ark", _) => Some("https://ark.cn-beijing.volces.com/api/v3"),
        ("zai", _) => Some("https://open.bigmodel.cn/api/paas/v4"),
        ("moonshot-kimi", _) => Some("https://api.moonshot.ai/v1"),
        ("baidu-qianfan", _) => Some("https://qianfan.baidubce.com/v2"),
        ("tencent-hunyuan", _) => Some("https://api.hunyuan.cloud.tencent.com/v1"),
        ("minimax", "minimax-global") => Some("https://api.minimax.io/v1"),
        ("minimax", _) => Some("https://api.minimaxi.com/v1"),
        ("siliconflow", _) => Some("https://api.siliconflow.cn/v1"),
        ("anthropic", _) => Some("https://api.anthropic.com/v1"),
        ("gemini", _) => Some("https://generativelanguage.googleapis.com/v1beta/openai"),
        ("openrouter", _) => Some("https://openrouter.ai/api/v1"),
        ("ollama", _) => Some("http://localhost:11434/v1"),
        ("lmstudio", _) => Some("http://localhost:1234/v1"),
        ("openai", _) => Some("https://api.openai.com/v1"),
        _ => None,
    }
}

fn normalize_model_base_url(
    _protocol: &str,
    provider_id: &str,
    route_id: &str,
    api_url: &str,
) -> String {
    if !api_url.trim().is_empty() {
        return normalize_base_url(api_url);
    }
    default_base_url_for(provider_id, route_id)
        .unwrap_or("")
        .to_string()
}

fn truncate_smoke_text(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(provider: &str) -> SaveConfigPayload {
        SaveConfigPayload {
            connection_id: None,
            provider: provider.to_string(),
            provider_id: None,
            route_id: None,
            connection_name: None,
            protocol: None,
            api_url: String::new(),
            api_key: None,
            clear_api_key: false,
            model_name: String::new(),
            auth_header: None,
            theme: "dark".to_string(),
            sound_enabled: true,
        }
    }

    #[test]
    fn save_config_payload_builds_minimal_openai_config() {
        let mut input = payload("openai");
        input.api_key = Some("test-key".to_string());
        input.model_name = "custom-model".to_string();
        let config = input.into_config(None);

        assert_eq!(config.llm.default_provider, "openai");
        assert_eq!(
            config.llm.default_connection_id.as_deref(),
            Some("openai:openai-default")
        );
        assert_eq!(config.llm.connections.len(), 2);
        let openai = config.llm.openai.unwrap();
        assert_eq!(openai.api_key, "test-key");
        assert_eq!(openai.model, "custom-model");
        assert_eq!(
            openai.base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert!(config.ui.sound_enabled);
    }

    #[test]
    fn normalize_legacy_deepseek_provider() {
        assert_eq!(normalize_provider_id("deepseek"), "deepseek");
        assert_eq!(normalize_provider_id("openai"), "openai");
    }

    #[test]
    fn save_config_does_not_reuse_openai_key_for_detected_deepseek_provider() {
        let mut current = Config::default();
        if let Some(openai) = current.llm.active_connection_mut() {
            openai.api_key = "existing-openai-key".to_string();
        }
        current.llm.sync_legacy_slots_from_connections();

        let mut input = payload("openai");
        input.api_url = "https://api.deepseek.com/v1/chat/completions".to_string();
        input.api_key = Some(String::new());
        input.model_name = "deepseek-v4-pro".to_string();
        let config = input.into_config(Some(&current));

        assert_eq!(config.llm.default_provider, "deepseek");
        let active = config.llm.active_connection().unwrap();
        assert_eq!(active.provider_id, "deepseek");
        assert_eq!(active.api_key, "");
        assert_eq!(active.model, "deepseek-v4-pro");
        assert_eq!(
            active.base_url.as_deref(),
            Some("https://api.deepseek.com/v1")
        );
        let saved_openai = config
            .llm
            .connections
            .iter()
            .find(|connection| connection.provider_id == "openai")
            .unwrap();
        assert_eq!(saved_openai.api_key, "existing-openai-key");
    }

    #[test]
    fn save_config_keeps_claude_base_url_and_key() {
        let mut input = payload("anthropic");
        input.api_url = "https://api.anthropic.com/v1/messages".to_string();
        input.api_key = Some("claude-key".to_string());
        input.model_name = "claude-opus-4-8".to_string();
        let config = input.into_config(None);

        assert_eq!(config.llm.default_provider, "anthropic");
        assert_eq!(
            config.llm.active_connection().unwrap().base_url.as_deref(),
            Some("https://api.anthropic.com/v1")
        );
        let anthropic = config.llm.anthropic.unwrap();
        assert_eq!(anthropic.api_key, "claude-key");
        assert_eq!(anthropic.model, "claude-opus-4-8");
        assert_eq!(
            anthropic.base_url.as_deref(),
            Some("https://api.anthropic.com/v1")
        );
    }

    #[test]
    fn model_settings_payload_normalizes_provider_without_guessing_model() {
        let settings = ModelSettingsPayload {
            connection_id: None,
            provider: "deepseek".to_string(),
            provider_id: None,
            route_id: None,
            protocol: None,
            api_url: "".to_string(),
            api_key: Some(" key ".to_string()),
            clear_api_key: false,
            model_name: "".to_string(),
            auth_header: None,
        }
        .normalized(None);

        assert_eq!(settings.provider_id, "deepseek");
        assert_eq!(settings.protocol, "openai-compatible");
        assert_eq!(settings.api_url, "https://api.deepseek.com/v1");
        assert_eq!(settings.api_key, "key");
        assert_eq!(settings.model, "");
    }

    #[test]
    fn model_settings_payload_reuses_saved_key_when_input_is_blank() {
        let current = SaveConfigPayload {
            api_key: Some(" saved-key ".to_string()),
            api_url: "https://api.deepseek.com/v1/chat/completions".to_string(),
            model_name: "deepseek-chat".to_string(),
            ..payload("deepseek")
        }
        .into_config(Some(&Config::default()));

        let settings = ModelSettingsPayload {
            connection_id: Some("deepseek:deepseek-openai".to_string()),
            provider: "deepseek".to_string(),
            provider_id: Some("deepseek".to_string()),
            route_id: Some("deepseek-openai".to_string()),
            protocol: Some("openai-compatible".to_string()),
            api_url: "https://api.deepseek.com/v1".to_string(),
            api_key: Some(" ".to_string()),
            clear_api_key: false,
            model_name: "deepseek-chat".to_string(),
            auth_header: None,
        }
        .normalized(Some(&current));

        assert_eq!(settings.api_key, "saved-key");
        assert_eq!(settings.api_url, "https://api.deepseek.com/v1");
    }

    #[test]
    fn model_settings_payload_does_not_reuse_openai_key_for_other_providers() {
        let mut current = Config::default();
        if let Some(openai) = current.llm.active_connection_mut() {
            openai.api_key = "openai-secret".to_string();
        }
        current.llm.sync_legacy_slots_from_connections();

        let settings = ModelSettingsPayload {
            connection_id: Some("openai:openai-default".to_string()),
            provider: "deepseek".to_string(),
            provider_id: Some("deepseek".to_string()),
            route_id: Some("deepseek-openai".to_string()),
            protocol: Some("openai-compatible".to_string()),
            api_url: "https://api.deepseek.com/v1".to_string(),
            api_key: None,
            clear_api_key: false,
            model_name: "deepseek-chat".to_string(),
            auth_header: None,
        }
        .normalized(Some(&current));

        assert_eq!(settings.api_key, "");
    }

    #[test]
    fn token_plan_without_base_url_stays_empty_in_command_payload() {
        let settings = ModelSettingsPayload {
            connection_id: None,
            provider: "xiaomi-mimo".to_string(),
            provider_id: Some("xiaomi-mimo".to_string()),
            route_id: Some("mimo-token-plan".to_string()),
            protocol: Some("openai-compatible".to_string()),
            api_url: "".to_string(),
            api_key: Some("mimo-key".to_string()),
            clear_api_key: false,
            model_name: "".to_string(),
            auth_header: None,
        }
        .normalized(None);

        assert_eq!(settings.api_url, "");
        assert_eq!(settings.provider_id, "xiaomi-mimo");
        assert_eq!(settings.route_id, "mimo-token-plan");
    }

    #[test]
    fn model_id_extractor_accepts_openai_shape() {
        let ids = extract_model_ids(&json!({
            "data": [
                { "id": "gpt-4o-mini" },
                { "name": "local-model" },
                "raw-model"
            ]
        }));
        assert_eq!(ids, vec!["gpt-4o-mini", "local-model", "raw-model"]);
    }
}
