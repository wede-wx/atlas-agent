use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex as StdMutex, OnceLock};

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use tauri::{Manager, State, Window};
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::time::{timeout, Duration};

use crate::storage::LocalDb;
use crate::storage::LogActivityEventPayload;
use crate::AppState;

const ISLAND_SETTINGS_KEY: &str = "agent_island_settings";
const MAX_CAPTURE_SIDE: i32 = 12_000;
const MAX_CAPTURE_PIXELS: i64 = 33_177_600;
const MAX_TEMP_IMAGE_BYTES: u64 = 30 * 1024 * 1024;
const MAX_SAVE_EXPORT_BYTES: usize = 30 * 1024 * 1024;
const MAX_SAVE_DIRECTORY_CHARS: usize = 1024;
const MAX_SAVE_FILE_NAME_CHARS: usize = 160;
const MAX_SHORTCUT_CHARS: usize = 80;
const TEMP_FILE_TTL_MS: u64 = 30 * 60 * 1000;

#[derive(Debug, Clone)]
struct IslandTempRecord {
    size: u64,
    created_at_ms: u64,
}

static ISLAND_TEMP_REGISTRY: OnceLock<StdMutex<HashMap<PathBuf, IslandTempRecord>>> =
    OnceLock::new();
static ISLAND_PS_SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IslandCapabilitySettings {
    pub task_status: bool,
    pub screenshot: bool,
    pub ocr: bool,
    pub window_context: bool,
    pub clipboard: bool,
    pub notifications: bool,
    pub media: bool,
    pub weather: bool,
    pub network: bool,
}

impl Default for IslandCapabilitySettings {
    fn default() -> Self {
        Self {
            task_status: true,
            screenshot: false,
            ocr: false,
            window_context: false,
            clipboard: false,
            notifications: false,
            media: false,
            weather: false,
            network: false,
        }
    }
}

fn default_sticker_opacity() -> f64 {
    1.0
}

fn default_result_enter_action() -> String {
    "copy".to_string()
}

fn default_color_format() -> String {
    "hex".to_string()
}

fn default_alternate_color_format() -> String {
    "rgb".to_string()
}

fn default_magnifier_scale() -> f64 {
    3.0
}

fn default_show_magnifier() -> bool {
    true
}

fn default_save_directory() -> String {
    String::new()
}

fn default_main_shortcut() -> String {
    "Ctrl+Alt+A".to_string()
}

fn default_pin_shortcut() -> String {
    "Ctrl+Alt+P".to_string()
}

fn default_delay_shortcut() -> String {
    "Ctrl+Alt+D".to_string()
}

fn deserialize_sticker_opacity<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let numeric = match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        Some(Value::Null) | None => Some(default_sticker_opacity()),
        _ => None,
    };
    Ok(numeric.unwrap_or_else(default_sticker_opacity))
}

fn normalize_result_enter_action(value: &str) -> String {
    match value.trim() {
        "copy" | "save" | "send" | "none" => value.trim().to_string(),
        _ => default_result_enter_action(),
    }
}

fn normalize_color_format(value: &str, fallback: fn() -> String) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "hex" | "rgb" | "hsl" | "hsv" => value.trim().to_ascii_lowercase(),
        _ => fallback(),
    }
}

fn normalize_magnifier_scale(value: f64) -> f64 {
    if value.is_finite() {
        value.round().clamp(2.0, 5.0)
    } else {
        default_magnifier_scale()
    }
}

fn deserialize_color_format<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let format = match value {
        Some(Value::String(text)) => normalize_color_format(&text, default_color_format),
        Some(Value::Null) | None => default_color_format(),
        _ => default_color_format(),
    };
    Ok(format)
}

fn deserialize_alternate_color_format<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let format = match value {
        Some(Value::String(text)) => normalize_color_format(&text, default_alternate_color_format),
        Some(Value::Null) | None => default_alternate_color_format(),
        _ => default_alternate_color_format(),
    };
    Ok(format)
}

fn deserialize_result_enter_action<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let action = match value {
        Some(Value::String(text)) => normalize_result_enter_action(&text),
        Some(Value::Null) | None => default_result_enter_action(),
        _ => default_result_enter_action(),
    };
    Ok(action)
}

fn deserialize_bool_default_false<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let enabled = match value {
        Some(Value::Bool(value)) => value,
        Some(Value::String(text)) => text.trim().eq_ignore_ascii_case("true"),
        Some(Value::Null) | None => false,
        _ => false,
    };
    Ok(enabled)
}

fn deserialize_bool_default_true<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let enabled = match value {
        Some(Value::Bool(value)) => value,
        Some(Value::String(text)) => !text.trim().eq_ignore_ascii_case("false"),
        Some(Value::Null) | None => true,
        _ => true,
    };
    Ok(enabled)
}

fn deserialize_magnifier_scale<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let scale = match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        Some(Value::Null) | None => Some(default_magnifier_scale()),
        _ => None,
    };
    Ok(normalize_magnifier_scale(
        scale.unwrap_or_else(default_magnifier_scale),
    ))
}

fn normalize_save_directory(value: &str) -> String {
    value
        .trim()
        .chars()
        .take(MAX_SAVE_DIRECTORY_CHARS)
        .collect()
}

fn normalize_shortcut_accelerator(value: &str) -> String {
    value.trim().chars().take(MAX_SHORTCUT_CHARS).collect()
}

fn deserialize_save_directory<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let directory = match value {
        Some(Value::String(text)) => normalize_save_directory(&text),
        Some(Value::Null) | None => default_save_directory(),
        _ => default_save_directory(),
    };
    Ok(directory)
}

fn deserialize_main_shortcut<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let shortcut = match value {
        Some(Value::String(text)) => normalize_shortcut_accelerator(&text),
        Some(Value::Null) | None => default_main_shortcut(),
        _ => default_main_shortcut(),
    };
    Ok(if shortcut.is_empty() {
        default_main_shortcut()
    } else {
        shortcut
    })
}

fn deserialize_pin_shortcut<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let shortcut = match value {
        Some(Value::String(text)) => normalize_shortcut_accelerator(&text),
        Some(Value::Null) | None => default_pin_shortcut(),
        _ => default_pin_shortcut(),
    };
    Ok(if shortcut.is_empty() {
        default_pin_shortcut()
    } else {
        shortcut
    })
}

fn deserialize_delay_shortcut<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let shortcut = match value {
        Some(Value::String(text)) => normalize_shortcut_accelerator(&text),
        Some(Value::Null) | None => default_delay_shortcut(),
        _ => default_delay_shortcut(),
    };
    Ok(if shortcut.is_empty() {
        default_delay_shortcut()
    } else {
        shortcut
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IslandScreenshotSettings {
    #[serde(
        default = "default_sticker_opacity",
        deserialize_with = "deserialize_sticker_opacity"
    )]
    pub sticker_default_opacity: f64,
    #[serde(default, deserialize_with = "deserialize_bool_default_false")]
    pub auto_ocr_after_capture: bool,
    #[serde(
        default = "default_save_directory",
        deserialize_with = "deserialize_save_directory"
    )]
    pub default_save_directory: String,
    #[serde(
        default = "default_main_shortcut",
        deserialize_with = "deserialize_main_shortcut"
    )]
    pub main_shortcut: String,
    #[serde(
        default = "default_pin_shortcut",
        deserialize_with = "deserialize_pin_shortcut"
    )]
    pub pin_shortcut: String,
    #[serde(
        default = "default_delay_shortcut",
        deserialize_with = "deserialize_delay_shortcut"
    )]
    pub delay_shortcut: String,
    #[serde(
        default = "default_result_enter_action",
        deserialize_with = "deserialize_result_enter_action"
    )]
    pub result_enter_action: String,
    #[serde(
        default = "default_result_enter_action",
        deserialize_with = "deserialize_result_enter_action"
    )]
    pub result_ctrl_c_action: String,
    #[serde(
        default = "default_result_enter_action",
        deserialize_with = "deserialize_result_enter_action"
    )]
    pub result_double_click_action: String,
    #[serde(
        default = "default_color_format",
        deserialize_with = "deserialize_color_format"
    )]
    pub color_format: String,
    #[serde(
        default = "default_alternate_color_format",
        deserialize_with = "deserialize_alternate_color_format"
    )]
    pub alternate_color_format: String,
    #[serde(
        default = "default_show_magnifier",
        deserialize_with = "deserialize_bool_default_true"
    )]
    pub show_magnifier: bool,
    #[serde(
        default = "default_magnifier_scale",
        deserialize_with = "deserialize_magnifier_scale"
    )]
    pub magnifier_scale: f64,
}

impl Default for IslandScreenshotSettings {
    fn default() -> Self {
        Self {
            sticker_default_opacity: default_sticker_opacity(),
            auto_ocr_after_capture: false,
            default_save_directory: default_save_directory(),
            main_shortcut: default_main_shortcut(),
            pin_shortcut: default_pin_shortcut(),
            delay_shortcut: default_delay_shortcut(),
            result_enter_action: default_result_enter_action(),
            result_ctrl_c_action: default_result_enter_action(),
            result_double_click_action: default_result_enter_action(),
            color_format: default_color_format(),
            alternate_color_format: default_alternate_color_format(),
            show_magnifier: default_show_magnifier(),
            magnifier_scale: default_magnifier_scale(),
        }
    }
}

impl IslandScreenshotSettings {
    fn normalize(&mut self) {
        if !self.sticker_default_opacity.is_finite() {
            self.sticker_default_opacity = default_sticker_opacity();
        }
        self.sticker_default_opacity = self.sticker_default_opacity.clamp(0.35, 1.0);
        self.default_save_directory = normalize_save_directory(&self.default_save_directory);
        self.main_shortcut = normalize_shortcut_accelerator(&self.main_shortcut);
        if self.main_shortcut.is_empty() {
            self.main_shortcut = default_main_shortcut();
        }
        self.pin_shortcut = normalize_shortcut_accelerator(&self.pin_shortcut);
        if self.pin_shortcut.is_empty() {
            self.pin_shortcut = default_pin_shortcut();
        }
        self.delay_shortcut = normalize_shortcut_accelerator(&self.delay_shortcut);
        if self.delay_shortcut.is_empty() {
            self.delay_shortcut = default_delay_shortcut();
        }
        self.result_enter_action = normalize_result_enter_action(&self.result_enter_action);
        self.result_ctrl_c_action = normalize_result_enter_action(&self.result_ctrl_c_action);
        self.result_double_click_action =
            normalize_result_enter_action(&self.result_double_click_action);
        self.color_format = normalize_color_format(&self.color_format, default_color_format);
        self.alternate_color_format =
            normalize_color_format(&self.alternate_color_format, default_alternate_color_format);
        self.magnifier_scale = normalize_magnifier_scale(self.magnifier_scale);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IslandSettingsPayload {
    pub enabled: bool,
    pub idle_hide: bool,
    pub manual_hidden: bool,
    pub privacy_paused: bool,
    pub confirm_before_send: bool,
    pub screenshot: IslandScreenshotSettings,
    pub capabilities: IslandCapabilitySettings,
}

impl Default for IslandSettingsPayload {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_hide: true,
            manual_hidden: false,
            privacy_paused: false,
            confirm_before_send: true,
            screenshot: IslandScreenshotSettings::default(),
            capabilities: IslandCapabilitySettings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum IslandCapability {
    Screenshot,
    Ocr,
    WindowContext,
    Clipboard,
    Media,
    Network,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IslandScreenshotRequest {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub height: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IslandScreenPixelRequest {
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub size: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandOcrRequest {
    pub image_path: String,
    #[serde(default)]
    pub languages: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandShortcutCheckInput {
    pub id: String,
    pub label: String,
    pub accelerator: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandShortcutConflictRequest {
    #[serde(default)]
    pub shortcuts: Vec<IslandShortcutCheckInput>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandShortcutCheckItem {
    pub id: String,
    pub label: String,
    pub accelerator: String,
    pub ok: bool,
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandShortcutConflictResult {
    pub ok: bool,
    pub status: String,
    pub items: Vec<IslandShortcutCheckItem>,
    pub checked_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandSavePathPermissionRequest {
    pub directory: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandSavePathPermissionResult {
    pub ok: bool,
    pub status: String,
    pub directory: String,
    pub reason: Option<String>,
    pub checked_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandContextExportRequest {
    pub directory: String,
    pub file_name: String,
    #[serde(default)]
    pub image_path: Option<String>,
    #[serde(default)]
    pub data_url: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandMediaControlRequest {
    pub action: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandKeyboardSmokeProofPayload {
    pub action: String,
    #[serde(default)]
    pub detail: Option<i64>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub control_aria_label: Option<String>,
    #[serde(default)]
    pub control_dataset: Option<Value>,
    #[serde(default)]
    pub active_element: Option<String>,
    pub expanded_before: bool,
    pub expanded_after: bool,
    #[serde(default)]
    pub source: Option<String>,
}

#[tauri::command]
pub async fn island_get_settings(
    window: Window,
    state: State<'_, AppState>,
) -> Result<IslandSettingsPayload, String> {
    ensure_island_ui_window(&window)?;
    load_island_settings(&state)
}

#[tauri::command]
pub async fn island_write_keyboard_smoke_proof(
    window: Window,
    payload: IslandKeyboardSmokeProofPayload,
) -> Result<Value, String> {
    ensure_island_ui_window(&window)?;
    let smoke_run_id = std::env::var("ATLAS_SMOKE_RUN_ID").unwrap_or_default();
    if !keyboard_smoke_proof_enabled() || smoke_run_id.trim().is_empty() {
        return Ok(json!({
            "ok": false,
            "skipped": true,
            "reason": "smoke_disabled"
        }));
    }

    let action = match payload.action.trim() {
        "expand" => "expand".to_string(),
        "collapse" => "collapse".to_string(),
        other => truncate_smoke_text(other, 48),
    };
    let proof = json!({
        "ok": true,
        "kind": "keyboard_toggle_smoke_proof",
        "smokeRunId": smoke_run_id,
        "windowLabel": window.label(),
        "detail": payload.detail,
        "key": payload.key.as_deref().map(|value| truncate_smoke_text(value, 48)),
        "action": action,
        "controlAriaLabel": payload.control_aria_label.as_deref().map(|value| truncate_smoke_text(value, 160)),
        "controlDataset": payload.control_dataset,
        "activeElement": payload.active_element.as_deref().map(|value| truncate_smoke_text(value, 160)),
        "expandedBefore": payload.expanded_before,
        "expandedAfter": payload.expanded_after,
        "source": payload.source.as_deref().map(|value| truncate_smoke_text(value, 160)).unwrap_or_else(|| "FloatWindow.toggleExpandedOnClick".to_string()),
        "capturedAt": now_ms(),
    });
    let path = std::env::temp_dir().join(format!(
        "atlas-island-keyboard-toggle-smoke-{}-{}.json",
        proof
            .get("smokeRunId")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        uuid::Uuid::new_v4()
    ));
    let bytes = serde_json::to_vec(&proof)
        .map_err(|error| format!("序列化键盘 smoke proof 失败: {error}"))?;
    std::fs::write(&path, bytes).map_err(|error| format!("写入键盘 smoke proof 失败: {error}"))?;
    Ok(json!({
        "ok": true,
        "path": path.to_string_lossy().to_string(),
        "proof": proof
    }))
}

#[tauri::command]
pub async fn island_save_settings(
    window: Window,
    mut settings: IslandSettingsPayload,
    state: State<'_, AppState>,
) -> Result<IslandSettingsPayload, String> {
    ensure_island_ui_window(&window)?;
    normalize_island_settings(&mut settings);
    let persisted = load_island_settings_from_db(&state.local_db)?;
    let mut settings_to_save = settings.clone();
    preserve_persisted_smoke_overrides_before_save(&mut settings_to_save, &persisted);
    normalize_island_settings(&mut settings_to_save);
    state
        .local_db
        .set_app_state(
            ISLAND_SETTINGS_KEY,
            serde_json::to_value(&settings_to_save).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("保存 Agent 浮层设置失败: {error}"))?;
    crate::sync_island_screenshot_shortcuts(window.app_handle());
    apply_smoke_island_settings(&mut settings_to_save);
    Ok(settings_to_save)
}

#[tauri::command]
pub async fn island_show_main_window(window: Window) -> Result<(), String> {
    ensure_island_show_main_window_label(window.label())?;
    let app = window.app_handle();
    let main = app
        .get_webview_window("main")
        .ok_or_else(|| "Atlas 主窗口不存在。".to_string())?;
    main.unminimize()
        .map_err(|error| format!("恢复 Atlas 主窗口失败: {error}"))?;
    main.show()
        .map_err(|error| format!("显示 Atlas 主窗口失败: {error}"))?;
    main.set_focus()
        .map_err(|error| format!("聚焦 Atlas 主窗口失败: {error}"))?;
    Ok(())
}

#[tauri::command]
pub async fn island_get_window_context(
    window: Window,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    ensure_island_collection_allowed(&window, &state, IslandCapability::WindowContext)?;
    let script = r#"
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class AtlasWin32 {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", SetLastError=true, CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int count);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
}
"@
$hwnd = [AtlasWin32]::GetForegroundWindow()
$titleBuilder = New-Object System.Text.StringBuilder 2048
[void][AtlasWin32]::GetWindowText($hwnd, $titleBuilder, $titleBuilder.Capacity)
$processId = 0
[void][AtlasWin32]::GetWindowThreadProcessId($hwnd, [ref]$processId)
$process = if ($processId -gt 0) { Get-Process -Id $processId -ErrorAction SilentlyContinue } else { $null }
[pscustomobject]@{
  ok = $true
  title = $titleBuilder.ToString()
  processId = [int]$processId
  processName = if ($process) { $process.ProcessName } else { $null }
  executablePath = if ($process) { $process.Path } else { $null }
  capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  source = "GetForegroundWindow"
} | ConvertTo-Json -Compress -Depth 8
"#;
    let mut value = powershell_json(script, 10).await?;
    annotate_window_context(&mut value);
    log_island_activity(
        &state,
        "window_context",
        "采集当前窗口元信息",
        value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("当前窗口"),
        json!({ "source": "GetForegroundWindow", "sentToAtlas": false }),
    )?;
    Ok(value)
}

#[tauri::command]
pub async fn island_read_clipboard(
    window: Window,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    ensure_island_collection_allowed(&window, &state, IslandCapability::Clipboard)?;
    let script = r#"
try {
  $text = ""
  try {
    $text = Get-Clipboard -Raw -Format Text -ErrorAction Stop
  } catch {
    $text = Get-Clipboard -Raw -ErrorAction Stop
  }
  if ($null -eq $text) { $text = "" }
  $originalLength = $text.Length
  $truncated = $false
  if ($text.Length -gt 6000) {
    $text = $text.Substring(0, 6000) + "`n`n[剪贴板内容已截断，仅导入前 6000 字符]"
    $truncated = $true
  }
  $highRisk = $text -match "(?i)(sk-[A-Za-z0-9_-]{20,}|token|password|secret|api[_-]?key)"
  [pscustomobject]@{
    ok = $true
    text = $text
    originalLength = $originalLength
    truncated = $truncated
    highRisk = [bool]$highRisk
    capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    source = "Get-Clipboard"
  } | ConvertTo-Json -Compress -Depth 8
} catch {
  [pscustomobject]@{
    ok = $false
    text = ""
    originalLength = 0
    truncated = $false
    highRisk = $false
    reason = $_.Exception.Message
    capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    source = "Get-Clipboard"
  } | ConvertTo-Json -Compress -Depth 8
}
"#;
    let value = powershell_json(script, 8).await?;
    log_island_activity(
        &state,
        "clipboard",
        "手动读取剪贴板",
        if value
            .get("highRisk")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            "剪贴板文本可能包含敏感字段"
        } else {
            "剪贴板文本"
        },
        json!({
            "source": "Get-Clipboard",
            "length": value.get("originalLength").cloned().unwrap_or(Value::Null),
            "truncated": value.get("truncated").cloned().unwrap_or(Value::Null),
            "highRisk": value.get("highRisk").cloned().unwrap_or(Value::Null),
            "sentToAtlas": false
        }),
    )?;
    Ok(value)
}

#[tauri::command]
pub async fn island_capture_screenshot(
    window: Window,
    payload: Option<IslandScreenshotRequest>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    ensure_island_collection_allowed(&window, &state, IslandCapability::Screenshot)?;
    cleanup_expired_atlas_temp_files();
    let payload = payload.unwrap_or_default();
    let mode = match payload
        .mode
        .unwrap_or_else(|| "screen".to_string())
        .as_str()
    {
        "area" => "area",
        "window" => "window",
        _ => "screen",
    };
    let x = payload.x.unwrap_or(0);
    let y = payload.y.unwrap_or(0);
    let width = payload.width.unwrap_or(0).max(0);
    let height = payload.height.unwrap_or(0).max(0);
    if mode == "area" {
        validate_capture_bounds(width, height)?;
    }

    let script = format!(
        r#"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$mode = '{mode}'
if ($mode -eq 'window') {{
  Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class AtlasWindowRect {{
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
  public struct RECT {{ public int Left; public int Top; public int Right; public int Bottom; }}
}}
"@
  $hwnd = [AtlasWindowRect]::GetForegroundWindow()
  if ($hwnd -eq [IntPtr]::Zero -or -not [AtlasWindowRect]::IsWindow($hwnd)) {{ throw "没有可截图的前台窗口。" }}
  if ([AtlasWindowRect]::IsIconic($hwnd)) {{ throw "当前窗口已最小化，无法可靠截取窗口。" }}
  $rectNative = New-Object AtlasWindowRect+RECT
  if (-not [AtlasWindowRect]::GetWindowRect($hwnd, [ref]$rectNative)) {{ throw "无法读取当前窗口位置。" }}
  $rect = New-Object System.Drawing.Rectangle($rectNative.Left, $rectNative.Top, ($rectNative.Right - $rectNative.Left), ($rectNative.Bottom - $rectNative.Top))
}} elseif ($mode -eq 'area' -and {width} -gt 0 -and {height} -gt 0) {{
  $rect = New-Object System.Drawing.Rectangle({x}, {y}, {width}, {height})
}} else {{
  $bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $rect = New-Object System.Drawing.Rectangle($bounds.Left, $bounds.Top, $bounds.Width, $bounds.Height)
}}
if ($rect.Width -le 0 -or $rect.Height -le 0) {{ throw "截图区域无效。" }}
if ($rect.Width -gt {max_side} -or $rect.Height -gt {max_side} -or ([int64]$rect.Width * [int64]$rect.Height) -gt {max_pixels}) {{ throw "截图区域过大。" }}
$bitmap = New-Object System.Drawing.Bitmap($rect.Width, $rect.Height)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $rect.Size)
$output = Join-Path ([System.IO.Path]::GetTempPath()) ("atlas-island-" + [guid]::NewGuid().ToString() + ".png")
$bitmap.Save($output, [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose()
$bitmap.Dispose()
[pscustomobject]@{{
  ok = $true
  mode = $mode
  tempPath = $output
  mime = "image/png"
  width = $rect.Width
  height = $rect.Height
  x = $rect.Left
  y = $rect.Top
  capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  source = "System.Drawing.CopyFromScreen"
}} | ConvertTo-Json -Compress -Depth 8
"#,
        max_side = MAX_CAPTURE_SIDE,
        max_pixels = MAX_CAPTURE_PIXELS
    );
    let mut value = powershell_json(&script, 20)
        .await
        .map_err(|error| describe_capture_error(&error))?;
    let temp_path = value
        .get("tempPath")
        .and_then(Value::as_str)
        .ok_or_else(|| "截图成功但未返回图片路径。".to_string())?
        .to_string();
    let bytes = match std::fs::read(&temp_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = cleanup_atlas_temp_file(&temp_path);
            return Err(format!("读取截图文件失败: {error}"));
        }
    };
    if let Err(error) = validate_temp_image_bytes(&bytes) {
        let _ = cleanup_atlas_temp_file(&temp_path);
        return Err(error);
    }
    if let Err(error) = register_atlas_temp_file(&temp_path, bytes.len() as u64) {
        let _ = cleanup_atlas_temp_file(&temp_path);
        return Err(error);
    }
    let data_url = format!(
        "data:image/png;base64,{}",
        general_purpose::STANDARD.encode(&bytes)
    );
    if let Some(object) = value.as_object_mut() {
        object.insert("dataUrl".to_string(), Value::String(data_url));
        object.insert("size".to_string(), json!(bytes.len()));
    }
    write_screenshot_smoke_proof(&value);
    if let Err(error) = log_island_activity(
        &state,
        "screenshot",
        "采集截图",
        value
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("screen"),
        json!({
            "source": "System.Drawing.CopyFromScreen",
            "mode": mode,
            "sentToAtlas": false,
            "bytes": bytes.len()
        }),
    ) {
        let _ = cleanup_atlas_temp_file(&temp_path);
        return Err(error);
    }
    Ok(value)
}

#[tauri::command]
pub async fn island_sample_screen_pixel(
    window: Window,
    payload: IslandScreenPixelRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    ensure_capture_overlay_window_label(window.label())?;
    let settings = load_island_settings(&state)?;
    if !settings.enabled {
        return Err("Atlas 灵动岛未启用。".to_string());
    }
    if settings.privacy_paused {
        return Err("隐私暂停中，已阻止屏幕取色。".to_string());
    }
    if !settings.capabilities.screenshot {
        return Err("截图能力未启用，无法读取屏幕像素。".to_string());
    }
    if payload.x.abs() > 100_000 || payload.y.abs() > 100_000 {
        return Err("取色坐标超出安全范围。".to_string());
    }
    let size = payload.size.unwrap_or(9).clamp(1, 15);
    let script = format!(
        r##"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$requestedX = {x}
$requestedY = {y}
$sampleSize = {size}
$bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
$right = $bounds.Left + $bounds.Width
$bottom = $bounds.Top + $bounds.Height
$centerX = [Math]::Max($bounds.Left, [Math]::Min($requestedX, $right - 1))
$centerY = [Math]::Max($bounds.Top, [Math]::Min($requestedY, $bottom - 1))
$half = [Math]::Floor($sampleSize / 2)
$left = $centerX - $half
$top = $centerY - $half
if ($left -lt $bounds.Left) {{ $left = $bounds.Left }}
if ($top -lt $bounds.Top) {{ $top = $bounds.Top }}
if (($left + $sampleSize) -gt $right) {{ $left = $right - $sampleSize }}
if (($top + $sampleSize) -gt $bottom) {{ $top = $bottom - $sampleSize }}
if ($left -lt $bounds.Left) {{ $left = $bounds.Left }}
if ($top -lt $bounds.Top) {{ $top = $bounds.Top }}
$bitmap = New-Object System.Drawing.Bitmap($sampleSize, $sampleSize)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen($left, $top, 0, 0, $bitmap.Size)
$centerPixelX = [Math]::Max(0, [Math]::Min($sampleSize - 1, $centerX - $left))
$centerPixelY = [Math]::Max(0, [Math]::Min($sampleSize - 1, $centerY - $top))
$center = $bitmap.GetPixel($centerPixelX, $centerPixelY)
$rows = @()
for ($row = 0; $row -lt $sampleSize; $row++) {{
  $items = @()
  for ($column = 0; $column -lt $sampleSize; $column++) {{
    $pixel = $bitmap.GetPixel($column, $row)
    $items += ("#{{0:X2}}{{1:X2}}{{2:X2}}" -f $pixel.R, $pixel.G, $pixel.B)
  }}
  $rows += ,$items
}}
$graphics.Dispose()
$bitmap.Dispose()
[pscustomobject]@{{
  ok = $true
  x = $centerX
  y = $centerY
  requestedX = $requestedX
  requestedY = $requestedY
  sampleX = $left
  sampleY = $top
  size = $sampleSize
  centerColumn = $centerPixelX
  centerRow = $centerPixelY
  r = $center.R
  g = $center.G
  b = $center.B
  hex = ("#{{0:X2}}{{1:X2}}{{2:X2}}" -f $center.R, $center.G, $center.B)
  pixels = $rows
  capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  source = "System.Drawing.CopyFromScreen"
}} | ConvertTo-Json -Compress -Depth 8
"##,
        x = payload.x,
        y = payload.y,
        size = size
    );
    let value = powershell_json(&script, 6)
        .await
        .map_err(|error| describe_capture_error(&error))?;
    write_screen_pixel_smoke_proof(&value);
    Ok(value)
}

#[tauri::command]
pub async fn island_run_ocr(
    window: Window,
    payload: IslandOcrRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    ensure_island_collection_allowed(&window, &state, IslandCapability::Ocr)?;
    let path = resolve_registered_atlas_temp_png(payload.image_path.trim())?;
    let languages = payload
        .languages
        .as_deref()
        .unwrap_or("zh-Hans,en")
        .trim()
        .to_string();
    let ps_path = ps_single_quote(&winrt_file_path(&path));
    let ps_languages = ps_single_quote(&languages);
    let script = format!(
        r#"
$imagePath = {ps_path}
$languages = {ps_languages}
$windowsError = $null
$ocrStage = "initializing"
try {{
  $ocrStage = "loading-winrt-types"
  Add-Type -AssemblyName System.Runtime.WindowsRuntime
  [Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime] | Out-Null
  [Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics.Imaging, ContentType = WindowsRuntime] | Out-Null
  [Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType = WindowsRuntime] | Out-Null
  [Windows.Globalization.Language, Windows.Globalization, ContentType = WindowsRuntime] | Out-Null
  $asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {{ $_.Name -eq 'AsTask' -and $_.IsGenericMethod -and $_.GetParameters().Count -eq 1 }})[0]
  function Await($operation, $resultType) {{
    $asTask = $asTaskGeneric.MakeGenericMethod($resultType)
    $task = $asTask.Invoke($null, @($operation))
    try {{
      $task.Wait()
    }} catch {{
      $inner = $_.Exception
      if ($inner.InnerException) {{ $inner = $inner.InnerException }}
      if ($inner.InnerException) {{ $inner = $inner.InnerException }}
      throw $inner.Message
    }}
    return $task.Result
  }}
  function TryEngineFromLanguage($language) {{
    if ($null -eq $language) {{ return $null }}
    try {{ return [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage($language) }} catch {{ return $null }}
  }}
  function NewOcrEngine($requestedLanguages) {{
    $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
    if ($null -ne $engine) {{ return $engine }}
    $requested = @($requestedLanguages -split "[,; ]+" | Where-Object {{ -not [string]::IsNullOrWhiteSpace($_) }})
    $candidates = @($requested + @("zh-Hans", "zh-CN", "en-US", "en")) |
      Where-Object {{ -not [string]::IsNullOrWhiteSpace($_) }} |
      Select-Object -Unique
    foreach ($tag in $candidates) {{
      $language = $null
      try {{ $language = [Windows.Globalization.Language]::new($tag.Trim()) }} catch {{ $language = $null }}
      $engine = TryEngineFromLanguage $language
      if ($null -ne $engine) {{ return $engine }}
    }}
    return $null
  }}
  $ocrStage = "opening-image"
  $file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($imagePath)) ([Windows.Storage.StorageFile])
  $ocrStage = "reading-image"
  $stream = Await ($file.OpenReadAsync()) ([Windows.Storage.Streams.IRandomAccessStreamWithContentType])
  $ocrStage = "decoding-image"
  $decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
  $bitmap = Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])
  $ocrStage = "creating-engine"
  $engine = NewOcrEngine $languages
  if ($null -eq $engine) {{
    throw ("系统 OCR 引擎不可用或未安装匹配语言包。请求语言：" + $languages)
  }}
  $ocrStage = "recognizing-image"
  $result = Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
  $lineTexts = @()
  foreach ($line in $result.Lines) {{ $lineTexts += $line.Text }}
  [pscustomobject]@{{
    ok = $true
    available = $true
    source = "Windows.Media.Ocr"
    text = $result.Text
    lines = $lineTexts
    language = $engine.RecognizerLanguage.LanguageTag
    confidence = $null
    warning = if ([string]::IsNullOrWhiteSpace($result.Text)) {{ "OCR 未识别到文字。" }} else {{ $null }}
    capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  }} | ConvertTo-Json -Compress -Depth 8
  exit 0
}} catch {{
  $windowsError = "[" + $ocrStage + "] " + $_.Exception.Message
}}
$tesseract = Get-Command tesseract -ErrorAction SilentlyContinue | Select-Object -First 1
if ($tesseract) {{
  $lang = if ($languages -match "zh") {{ "chi_sim+eng" }} else {{ "eng" }}
  $stderrPath = Join-Path ([System.IO.Path]::GetTempPath()) ("atlas-island-ocr-" + [guid]::NewGuid().ToString() + ".err")
  $text = & $tesseract.Source $imagePath stdout -l $lang 2>$stderrPath
  $exitCode = $LASTEXITCODE
  $stderr = if (Test-Path $stderrPath) {{ Get-Content $stderrPath -Raw -ErrorAction SilentlyContinue }} else {{ "" }}
  Remove-Item -Force -ErrorAction SilentlyContinue $stderrPath
  if ($exitCode -ne 0) {{
    [pscustomobject]@{{
      ok = $false
      available = $true
      source = "tesseract"
      text = ""
      lines = @()
      language = $lang
      warning = "Tesseract OCR 执行失败。"
      reason = $stderr
      exitCode = $exitCode
      windowsOcrError = $windowsError
      capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    }} | ConvertTo-Json -Compress -Depth 8
    exit 0
  }}
  [pscustomobject]@{{
    ok = $true
    available = $true
    source = "tesseract"
    text = ($text -join "`n").Trim()
    lines = @($text)
    language = $lang
    confidence = $null
    warning = if (($text -join "").Trim().Length -eq 0) {{ "OCR 未识别到文字。" }} else {{ $null }}
    windowsOcrError = $windowsError
    capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  }} | ConvertTo-Json -Compress -Depth 8
}} else {{
  [pscustomobject]@{{
    ok = $false
    available = $false
    source = "Windows.Media.Ocr"
    text = ""
    lines = @()
    language = $languages
    warning = "Windows OCR 不可用，且未检测到 Tesseract。"
    reason = $windowsError
    capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  }} | ConvertTo-Json -Compress -Depth 8
}}
"#
    );
    let mut value = powershell_json(&script, 25).await.unwrap_or_else(|error| {
        json!({
            "ok": false,
            "available": false,
            "source": "Windows.Media.Ocr",
            "text": "",
            "lines": [],
            "language": languages,
            "warning": "OCR 执行失败，已保留截图。",
            "reason": error,
            "capturedAt": now_ms(),
        })
    });
    annotate_ocr_quality(&mut value);
    write_ocr_smoke_proof(&payload.image_path, &value);
    log_island_activity(
        &state,
        "ocr",
        "运行截图 OCR",
        value.get("source").and_then(Value::as_str).unwrap_or("OCR"),
        json!({
            "source": value.get("source").cloned().unwrap_or(Value::Null),
            "available": value.get("available").cloned().unwrap_or(Value::Null),
            "sentToAtlas": false
        }),
    )?;
    Ok(value)
}

#[tauri::command]
pub async fn island_get_media_status(
    window: Window,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    ensure_island_collection_allowed(&window, &state, IslandCapability::Media)?;
    let script = r#"
try {
  Add-Type -AssemblyName System.Runtime.WindowsRuntime
  [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager, Windows.Media.Control, ContentType = WindowsRuntime] | Out-Null
  [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties, Windows.Media.Control, ContentType = WindowsRuntime] | Out-Null
  [Windows.Storage.Streams.DataReader, Windows.Storage.Streams, ContentType = WindowsRuntime] | Out-Null
  [Windows.Storage.Streams.IRandomAccessStreamWithContentType, Windows.Storage.Streams, ContentType = WindowsRuntime] | Out-Null
  $asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.IsGenericMethod -and $_.GetParameters().Count -eq 1 })[0]
  function Await($operation, $resultType) {
    $asTask = $asTaskGeneric.MakeGenericMethod($resultType)
    $task = $asTask.Invoke($null, @($operation))
    $task.Wait()
    return $task.Result
  }
  $manager = Await ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager]::RequestAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager])
  $session = $manager.GetCurrentSession()
  if ($null -eq $session) {
    [pscustomobject]@{
      ok = $true
      available = $false
      reason = "当前没有系统媒体会话。"
      capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
      source = "GlobalSystemMediaTransportControlsSessionManager"
    } | ConvertTo-Json -Compress -Depth 8
    exit 0
  }
  $properties = Await ($session.TryGetMediaPropertiesAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties])
  $timeline = $session.GetTimelineProperties()
  $playback = $session.GetPlaybackInfo()
  $thumbnailDataUrl = $null
  $thumbnailMime = $null
  try {
    if ($properties.Thumbnail) {
      $stream = Await ($properties.Thumbnail.OpenReadAsync()) ([Windows.Storage.Streams.IRandomAccessStreamWithContentType])
      $thumbnailMime = $stream.ContentType
      $reader = New-Object Windows.Storage.Streams.DataReader $stream
      $size = [uint32]([Math]::Min($stream.Size, 4194304))
      $loaded = Await ($reader.LoadAsync($size)) ([uint32])
      $bytes = New-Object byte[] $loaded
      $reader.ReadBytes($bytes)
      $thumbnailDataUrl = "data:$thumbnailMime;base64," + [Convert]::ToBase64String($bytes)
      $reader.Dispose()
      $stream.Dispose()
    }
  } catch {}
  [pscustomobject]@{
    ok = $true
    available = $true
    source = "GlobalSystemMediaTransportControlsSessionManager"
    appUserModelId = $session.SourceAppUserModelId
    title = $properties.Title
    artist = $properties.Artist
    albumTitle = $properties.AlbumTitle
    albumArtist = $properties.AlbumArtist
    trackNumber = $properties.TrackNumber
    genres = @($properties.Genres)
    playbackStatus = $playback.PlaybackStatus.ToString()
    positionMs = [int64]$timeline.Position.TotalMilliseconds
    startTimeMs = [int64]$timeline.StartTime.TotalMilliseconds
    endTimeMs = [int64]$timeline.EndTime.TotalMilliseconds
    minSeekTimeMs = [int64]$timeline.MinSeekTime.TotalMilliseconds
    maxSeekTimeMs = [int64]$timeline.MaxSeekTime.TotalMilliseconds
    thumbnailMime = $thumbnailMime
    thumbnailDataUrl = $thumbnailDataUrl
    capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  } | ConvertTo-Json -Compress -Depth 8
} catch {
  [pscustomobject]@{
    ok = $false
    available = $false
    reason = $_.Exception.Message
    capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    source = "GlobalSystemMediaTransportControlsSessionManager"
  } | ConvertTo-Json -Compress -Depth 8
}
"#;
    let value = powershell_json(script, 20).await?;
    log_island_activity(
        &state,
        "media",
        "读取媒体会话",
        value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("系统媒体会话"),
        json!({
            "source": "GlobalSystemMediaTransportControlsSessionManager",
            "available": value.get("available").cloned().unwrap_or(Value::Null),
            "sentToAtlas": false
        }),
    )?;
    Ok(value)
}

#[tauri::command]
pub async fn island_control_media(
    window: Window,
    payload: IslandMediaControlRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    ensure_island_collection_allowed(&window, &state, IslandCapability::Media)?;
    let action = match payload.action.trim() {
        "playPause" | "toggle" => "playPause",
        "next" => "next",
        "previous" | "prev" => "previous",
        _ => return Err("不支持的媒体控制动作。".to_string()),
    };
    let ps_action = ps_single_quote(action);
    let script = format!(
        r#"
$action = {ps_action}
try {{
  Add-Type -AssemblyName System.Runtime.WindowsRuntime
  [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager, Windows.Media.Control, ContentType = WindowsRuntime] | Out-Null
  $asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {{ $_.Name -eq 'AsTask' -and $_.IsGenericMethod -and $_.GetParameters().Count -eq 1 }})[0]
  function Await($operation, $resultType) {{
    $asTask = $asTaskGeneric.MakeGenericMethod($resultType)
    $task = $asTask.Invoke($null, @($operation))
    $task.Wait()
    return $task.Result
  }}
  $manager = Await ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager]::RequestAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager])
  $session = $manager.GetCurrentSession()
  if ($null -eq $session) {{
    [pscustomobject]@{{
      ok = $false
      available = $false
      action = $action
      reason = "当前没有系统媒体会话。"
      capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
      source = "GlobalSystemMediaTransportControlsSessionManager"
    }} | ConvertTo-Json -Compress -Depth 8
    exit 0
  }}
  $result = switch ($action) {{
    "next" {{ Await ($session.TrySkipNextAsync()) ([bool]) }}
    "previous" {{ Await ($session.TrySkipPreviousAsync()) ([bool]) }}
    default {{ Await ($session.TryTogglePlayPauseAsync()) ([bool]) }}
  }}
  [pscustomobject]@{{
    ok = [bool]$result
    available = $true
    action = $action
    supported = [bool]$result
    appUserModelId = $session.SourceAppUserModelId
    capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    source = "GlobalSystemMediaTransportControlsSessionManager"
  }} | ConvertTo-Json -Compress -Depth 8
}} catch {{
  [pscustomobject]@{{
    ok = $false
    available = $false
    action = $action
    reason = $_.Exception.Message
    capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    source = "GlobalSystemMediaTransportControlsSessionManager"
  }} | ConvertTo-Json -Compress -Depth 8
}}
"#
    );
    let value = powershell_json(&script, 12).await?;
    log_island_activity(
        &state,
        "media_control",
        "控制系统媒体会话",
        action,
        json!({
            "source": "GlobalSystemMediaTransportControlsSessionManager",
            "action": action,
            "supported": value.get("supported").cloned().unwrap_or(Value::Null),
            "sentToAtlas": false
        }),
    )?;
    Ok(value)
}

#[tauri::command]
pub async fn island_get_system_status(
    window: Window,
    state: State<'_, AppState>,
    request_kind: Option<String>,
) -> Result<Value, String> {
    ensure_island_collection_allowed(&window, &state, IslandCapability::Network)?;
    let request_kind = match request_kind.as_deref() {
        Some("auto_glance") => "auto_glance",
        Some("manual_button") => "manual_button",
        _ => "unknown",
    };
    let script = r#"
$network = $null
try {
  $before = Get-NetAdapterStatistics -ErrorAction Stop
  Start-Sleep -Milliseconds 1000
  $after = Get-NetAdapterStatistics -ErrorAction Stop
  $rxBefore = ($before | Measure-Object -Property ReceivedBytes -Sum).Sum
  $txBefore = ($before | Measure-Object -Property SentBytes -Sum).Sum
  $rxAfter = ($after | Measure-Object -Property ReceivedBytes -Sum).Sum
  $txAfter = ($after | Measure-Object -Property SentBytes -Sum).Sum
  $network = [pscustomobject]@{
    ok = $true
    rxBytesPerSec = [int64]($rxAfter - $rxBefore)
    txBytesPerSec = [int64]($txAfter - $txBefore)
    adapters = @($after | Select-Object Name,ReceivedBytes,SentBytes)
    source = "Get-NetAdapterStatistics"
  }
} catch {
  $network = [pscustomobject]@{ ok = $false; reason = $_.Exception.Message; source = "Get-NetAdapterStatistics" }
}
$battery = $null
try {
  $batteryItems = Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue
  if ($batteryItems) {
    $battery = @($batteryItems | Select-Object EstimatedChargeRemaining,BatteryStatus,EstimatedRunTime)
  }
} catch {}
$os = Get-CimInstance Win32_OperatingSystem -ErrorAction SilentlyContinue
$cpu = Get-CimInstance Win32_Processor -ErrorAction SilentlyContinue | Measure-Object -Property LoadPercentage -Average
$disk = Get-CimInstance Win32_LogicalDisk -Filter "DriveType=3" -ErrorAction SilentlyContinue | Select-Object DeviceID,Size,FreeSpace
[pscustomobject]@{
  ok = $true
  capturedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  source = "CIM/Get-NetAdapterStatistics"
  time = [pscustomobject]@{
    iso = (Get-Date).ToString("o")
    local = (Get-Date).ToString("yyyy-MM-dd HH:mm:ss")
    weekday = (Get-Date).DayOfWeek.ToString()
  }
  network = $network
  battery = $battery
  cpu = [pscustomobject]@{ loadPercent = if ($cpu) { [double]$cpu.Average } else { $null } }
  memory = [pscustomobject]@{
    totalBytes = if ($os) { [int64]$os.TotalVisibleMemorySize * 1024 } else { $null }
    freeBytes = if ($os) { [int64]$os.FreePhysicalMemory * 1024 } else { $null }
  }
  disks = @($disk)
    } | ConvertTo-Json -Compress -Depth 8
"#;
    let value = powershell_json(script, 15).await?;
    write_system_status_smoke_proof(&value, request_kind);
    log_island_activity(
        &state,
        "system_status",
        "读取辅助系统状态",
        "日期、网络、电量、CPU、内存、磁盘",
        json!({ "source": "CIM/Get-NetAdapterStatistics", "requestKind": request_kind, "sentToAtlas": false }),
    )?;
    Ok(value)
}

#[tauri::command]
pub async fn island_log_context_sent(
    window: Window,
    package_id: String,
    source: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    ensure_island_ui_window(&window)?;
    log_island_activity(
        &state,
        "context_sent",
        "发送浮层上下文给 Atlas",
        source.trim(),
        json!({ "packageId": package_id, "source": source, "sentToAtlas": true }),
    )?;
    Ok(())
}

#[tauri::command]
pub async fn island_log_context_imported(
    window: Window,
    package_id: String,
    source: String,
    proof: Option<Value>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    ensure_island_ui_window(&window)?;
    write_context_import_smoke_proof(&package_id, &source, proof.as_ref());
    log_island_activity(
        &state,
        "context_imported",
        "导入浮层上下文到输入框",
        source.trim(),
        json!({ "packageId": package_id, "source": source, "sentToAtlas": false, "proof": proof }),
    )?;
    Ok(())
}

#[tauri::command]
pub async fn island_cleanup_temp_file(window: Window, path: String) -> Result<(), String> {
    ensure_island_ui_window(&window)?;
    cleanup_registered_atlas_temp_file(&path)
}

#[tauri::command]
pub async fn island_read_temp_image(window: Window, path: String) -> Result<Value, String> {
    ensure_island_ui_window(&window)?;
    let real = resolve_registered_atlas_temp_png(&path)?;
    validate_temp_image_file(&real)?;
    let bytes = match std::fs::read(&real) {
        Ok(bytes) => bytes,
        Err(error) => {
            unregister_atlas_temp_file(&real);
            let _ = std::fs::remove_file(&real);
            return Err(format!("读取临时截图失败: {error}"));
        }
    };
    if let Err(error) = validate_temp_image_bytes(&bytes) {
        unregister_atlas_temp_file(&real);
        let _ = std::fs::remove_file(&real);
        return Err(error);
    }
    let value = json!({
        "tempPath": real.to_string_lossy(),
        "mime": "image/png",
        "size": bytes.len(),
        "dataUrl": format!("data:image/png;base64,{}", general_purpose::STANDARD.encode(bytes))
    });
    unregister_atlas_temp_file(&real);
    let _ = std::fs::remove_file(&real);
    Ok(value)
}

#[tauri::command]
pub async fn island_check_shortcut_conflicts(
    window: Window,
    payload: IslandShortcutConflictRequest,
) -> Result<IslandShortcutConflictResult, String> {
    ensure_island_ui_window(&window)?;
    Ok(check_shortcut_conflicts(&window, payload))
}

#[tauri::command]
pub async fn island_check_save_path_permission(
    window: Window,
    payload: IslandSavePathPermissionRequest,
) -> Result<IslandSavePathPermissionResult, String> {
    ensure_island_ui_window(&window)?;
    Ok(check_save_directory_permission(&payload.directory))
}

#[tauri::command]
pub async fn island_save_context_export(
    window: Window,
    payload: IslandContextExportRequest,
) -> Result<Value, String> {
    ensure_island_ui_window(&window)?;
    let permission = check_save_directory_permission(&payload.directory);
    if !permission.ok {
        let reason = permission
            .reason
            .clone()
            .unwrap_or_else(|| permission.status.clone());
        return Err(format!("默认保存路径不可写：{reason}"));
    }
    let directory = resolve_existing_save_directory(&payload.directory)?;
    let file_name = sanitize_save_file_name(&payload.file_name);
    let target = unique_save_path(&directory, &file_name)?;
    let bytes_written = if let Some(image_path) = payload
        .image_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let source = resolve_registered_atlas_temp_png(image_path)?;
        validate_temp_image_file(&source)?;
        std::fs::copy(&source, &target).map_err(|error| format!("保存截图失败: {error}"))? as usize
    } else if let Some(data_url) = payload
        .data_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let bytes = decode_png_data_url(data_url)?;
        write_export_bytes(&target, &bytes)?
    } else {
        let text = payload.text.unwrap_or_default();
        let bytes = text.as_bytes();
        write_export_bytes(&target, bytes)?
    };
    let saved_file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_string)
        .unwrap_or(file_name);
    Ok(json!({
        "ok": true,
        "status": "saved",
        "path": target.to_string_lossy(),
        "fileName": saved_file_name,
        "bytes": bytes_written,
        "savedAt": now_ms()
    }))
}

fn cleanup_atlas_temp_file(path: &str) -> Result<(), String> {
    let real = match resolve_atlas_temp_png(path) {
        Ok(path) => path,
        Err(error) if error.contains("不存在") => return Ok(()),
        Err(error) => return Err(error),
    };
    unregister_atlas_temp_file(&real);
    match std::fs::remove_file(&real) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("清理临时截图失败: {error}")),
    }
}

fn cleanup_registered_atlas_temp_file(path: &str) -> Result<(), String> {
    let real = match resolve_registered_atlas_temp_png(path) {
        Ok(path) => path,
        Err(error) if error.contains("不存在") => return Ok(()),
        Err(error) => return Err(error),
    };
    unregister_atlas_temp_file(&real);
    match std::fs::remove_file(&real) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("清理临时截图失败: {error}")),
    }
}

fn resolve_atlas_temp_png(path: &str) -> Result<PathBuf, String> {
    let target = PathBuf::from(path.trim());
    if target.as_os_str().is_empty() {
        return Err("临时截图路径为空。".to_string());
    }
    let temp_dir = std::env::temp_dir()
        .canonicalize()
        .map_err(|error| format!("无法解析临时目录: {error}"))?;
    let real = match target.canonicalize() {
        Ok(path) => path,
        Err(_) => return Err("临时截图不存在。".to_string()),
    };
    let file_name = real
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let is_atlas_temp = file_name.starts_with("atlas-island-") && file_name.ends_with(".png");
    if !real.starts_with(&temp_dir) || !is_atlas_temp {
        return Err("拒绝访问非 Atlas 浮层临时截图。".to_string());
    }
    Ok(real)
}

fn temp_registry() -> &'static StdMutex<HashMap<PathBuf, IslandTempRecord>> {
    ISLAND_TEMP_REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn island_powershell_semaphore() -> &'static Semaphore {
    ISLAND_PS_SEMAPHORE.get_or_init(|| Semaphore::new(2))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn register_atlas_temp_file(path: &str, size: u64) -> Result<(), String> {
    let real = resolve_atlas_temp_png(path)?;
    if size > MAX_TEMP_IMAGE_BYTES {
        return Err("临时截图文件过大。".to_string());
    }
    let mut registry = temp_registry()
        .lock()
        .map_err(|_| "临时截图登记表不可用。".to_string())?;
    registry.insert(
        real,
        IslandTempRecord {
            size,
            created_at_ms: now_ms(),
        },
    );
    Ok(())
}

fn unregister_atlas_temp_file(path: &Path) {
    if let Ok(mut registry) = temp_registry().lock() {
        registry.remove(path);
    }
}

fn resolve_registered_atlas_temp_png(path: &str) -> Result<PathBuf, String> {
    let real = resolve_atlas_temp_png(path)?;
    let mut registry = temp_registry()
        .lock()
        .map_err(|_| "临时截图登记表不可用。".to_string())?;
    let Some(record) = registry.get(&real).cloned() else {
        return Err("临时截图不在 Atlas 本次会话登记范围内。".to_string());
    };
    let expired = now_ms().saturating_sub(record.created_at_ms) > TEMP_FILE_TTL_MS;
    let size = std::fs::metadata(&real)
        .map_err(|error| format!("读取临时截图信息失败: {error}"))?
        .len();
    if expired {
        registry.remove(&real);
        let _ = std::fs::remove_file(&real);
        return Err("临时截图已过期。".to_string());
    }
    if size != record.size || size > MAX_TEMP_IMAGE_BYTES {
        registry.remove(&real);
        return Err("临时截图文件已变化或过大。".to_string());
    }
    Ok(real)
}

fn validate_temp_image_file(path: &Path) -> Result<(), String> {
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("读取临时截图信息失败: {error}"))?;
    if metadata.len() > MAX_TEMP_IMAGE_BYTES {
        return Err("临时截图文件过大。".to_string());
    }
    Ok(())
}

fn validate_temp_image_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > MAX_TEMP_IMAGE_BYTES {
        return Err("临时截图文件过大。".to_string());
    }
    if !bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]) {
        return Err("临时截图不是有效 PNG。".to_string());
    }
    Ok(())
}

fn save_path_result(
    ok: bool,
    status: &str,
    directory: String,
    reason: Option<String>,
) -> IslandSavePathPermissionResult {
    IslandSavePathPermissionResult {
        ok,
        status: status.to_string(),
        directory,
        reason,
        checked_at: now_ms(),
    }
}

fn shortcut_check_item(
    input: &IslandShortcutCheckInput,
    accelerator: String,
    ok: bool,
    status: &str,
    reason: Option<String>,
) -> IslandShortcutCheckItem {
    IslandShortcutCheckItem {
        id: input.id.trim().chars().take(64).collect(),
        label: input.label.trim().chars().take(80).collect(),
        accelerator,
        ok,
        status: status.to_string(),
        reason,
    }
}

fn aggregate_shortcut_status(items: &[IslandShortcutCheckItem]) -> String {
    for status in [
        "invalid",
        "conflict",
        "registered_by_atlas",
        "failed",
        "unavailable",
        "unconfigured",
    ] {
        if items.iter().any(|item| item.status == status) {
            return status.to_string();
        }
    }
    "available".to_string()
}

fn check_shortcut_conflicts(
    window: &Window,
    payload: IslandShortcutConflictRequest,
) -> IslandShortcutConflictResult {
    let checked_at = now_ms();
    let mut items = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();

    #[cfg(desktop)]
    {
        use std::str::FromStr;
        use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

        let app = window.app_handle().clone();
        let shortcut_manager = app.global_shortcut();
        for input in payload.shortcuts {
            let accelerator = normalize_shortcut_accelerator(&input.accelerator);
            if accelerator.is_empty() {
                items.push(shortcut_check_item(
                    &input,
                    accelerator,
                    false,
                    "unconfigured",
                    Some("快捷键未设置。".to_string()),
                ));
                continue;
            }
            let dedupe_key = accelerator.to_ascii_lowercase().replace(' ', "");
            if let Some(first_label) = seen.get(&dedupe_key) {
                items.push(shortcut_check_item(
                    &input,
                    accelerator,
                    false,
                    "conflict",
                    Some(format!("与“{first_label}”使用了同一个快捷键。")),
                ));
                continue;
            }
            seen.insert(dedupe_key, input.label.clone());

            let shortcut = match Shortcut::from_str(&accelerator) {
                Ok(shortcut) => shortcut,
                Err(error) => {
                    items.push(shortcut_check_item(
                        &input,
                        accelerator,
                        false,
                        "invalid",
                        Some(format!("快捷键格式不可识别: {error}")),
                    ));
                    continue;
                }
            };
            if crate::is_registered_island_screenshot_shortcut(&shortcut) {
                items.push(shortcut_check_item(
                    &input,
                    accelerator,
                    true,
                    "available",
                    Some("这个快捷键已由 Atlas 截图动作注册，可继续使用。".to_string()),
                ));
                continue;
            }
            if shortcut_manager.is_registered(shortcut) {
                items.push(shortcut_check_item(
                    &input,
                    accelerator,
                    false,
                    "registered_by_atlas",
                    Some("这个快捷键已被 Atlas 当前进程占用，不能再分配给截图动作。".to_string()),
                ));
                continue;
            }
            match shortcut_manager.register(shortcut) {
                Ok(_) => match shortcut_manager.unregister(shortcut) {
                    Ok(_) => items.push(shortcut_check_item(
                        &input,
                        accelerator,
                        true,
                        "available",
                        None,
                    )),
                    Err(error) => items.push(shortcut_check_item(
                        &input,
                        accelerator,
                        false,
                        "failed",
                        Some(format!("快捷键探针注册成功但注销失败: {error}")),
                    )),
                },
                Err(error) => {
                    items.push(shortcut_check_item(
                        &input,
                        accelerator,
                        false,
                        "conflict",
                        Some(format!(
                            "系统拒绝注册这个快捷键，可能已被其他应用占用: {error}"
                        )),
                    ));
                }
            }
        }
    }

    #[cfg(not(desktop))]
    {
        let _ = window;
        for input in payload.shortcuts {
            let accelerator = normalize_shortcut_accelerator(&input.accelerator);
            items.push(shortcut_check_item(
                &input,
                accelerator,
                false,
                "unavailable",
                Some("当前平台不支持全局快捷键注册探针。".to_string()),
            ));
        }
    }

    if items.is_empty() {
        items.push(IslandShortcutCheckItem {
            id: "none".to_string(),
            label: "截图快捷键".to_string(),
            accelerator: String::new(),
            ok: false,
            status: "unconfigured".to_string(),
            reason: Some("没有可检测的快捷键。".to_string()),
        });
    }
    let status = aggregate_shortcut_status(&items);
    IslandShortcutConflictResult {
        ok: items.iter().all(|item| item.ok),
        status,
        items,
        checked_at,
    }
}

fn resolve_existing_save_directory(directory: &str) -> Result<PathBuf, String> {
    let directory = normalize_save_directory(directory);
    if directory.is_empty() {
        return Err("默认保存路径未设置。".to_string());
    }
    let path = PathBuf::from(&directory);
    let real = path
        .canonicalize()
        .map_err(|error| format!("默认保存路径不存在或无法访问: {error}"))?;
    let metadata =
        std::fs::metadata(&real).map_err(|error| format!("读取默认保存路径失败: {error}"))?;
    if !metadata.is_dir() {
        return Err("默认保存路径不是文件夹。".to_string());
    }
    Ok(real)
}

fn check_save_directory_permission(directory: &str) -> IslandSavePathPermissionResult {
    let directory = normalize_save_directory(directory);
    if directory.is_empty() {
        return save_path_result(
            false,
            "unconfigured",
            directory,
            Some("未设置默认保存路径。".to_string()),
        );
    }
    let real = match PathBuf::from(&directory).canonicalize() {
        Ok(path) => path,
        Err(error) => {
            return save_path_result(
                false,
                "missing",
                directory,
                Some(format!("路径不存在或无法访问: {error}")),
            )
        }
    };
    let metadata = match std::fs::metadata(&real) {
        Ok(metadata) => metadata,
        Err(error) => {
            return save_path_result(
                false,
                "missing",
                real.to_string_lossy().to_string(),
                Some(format!("读取路径信息失败: {error}")),
            )
        }
    };
    if !metadata.is_dir() {
        return save_path_result(
            false,
            "not_directory",
            real.to_string_lossy().to_string(),
            Some("目标路径不是文件夹。".to_string()),
        );
    }
    let probe = real.join(format!(
        ".atlas-island-save-probe-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => file,
        Err(error) => {
            return save_path_result(
                false,
                "denied",
                real.to_string_lossy().to_string(),
                Some(format!("无法创建写入探针: {error}")),
            )
        }
    };
    let write_result = file.write_all(b"atlas-save-probe");
    drop(file);
    let _ = std::fs::remove_file(&probe);
    match write_result {
        Ok(_) => save_path_result(true, "writable", real.to_string_lossy().to_string(), None),
        Err(error) => save_path_result(
            false,
            "denied",
            real.to_string_lossy().to_string(),
            Some(format!("写入探针失败: {error}")),
        ),
    }
}

fn sanitize_save_file_name(value: &str) -> String {
    let replaced: String = value
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            ch if ch.is_control() => '-',
            ch => ch,
        })
        .collect();
    let trimmed = replaced
        .trim()
        .trim_matches(|ch| ch == '.' || ch == ' ')
        .chars()
        .take(MAX_SAVE_FILE_NAME_CHARS)
        .collect::<String>();
    let name = trimmed
        .trim()
        .trim_matches(|ch| ch == '.' || ch == ' ')
        .to_string();
    if name.is_empty() {
        "atlas-island-export.txt".to_string()
    } else {
        name
    }
}

fn unique_save_path(directory: &Path, file_name: &str) -> Result<PathBuf, String> {
    let file_name = sanitize_save_file_name(file_name);
    let candidate = directory.join(&file_name);
    if !candidate.exists() {
        return Ok(candidate);
    }
    let path = Path::new(&file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("atlas-island-export");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    for index in 1..1000 {
        let next_name = if extension.is_empty() {
            format!("{stem} ({index})")
        } else {
            format!("{stem} ({index}).{extension}")
        };
        let next = directory.join(next_name);
        if !next.exists() {
            return Ok(next);
        }
    }
    Err("默认保存路径中同名文件过多，无法生成唯一文件名。".to_string())
}

fn write_export_bytes(path: &Path, bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() > MAX_SAVE_EXPORT_BYTES {
        return Err("导出内容过大，已拒绝保存。".to_string());
    }
    std::fs::write(path, bytes).map_err(|error| format!("写入导出文件失败: {error}"))?;
    Ok(bytes.len())
}

fn decode_png_data_url(data_url: &str) -> Result<Vec<u8>, String> {
    let trimmed = data_url.trim();
    let Some((header, payload)) = trimmed.split_once(',') else {
        return Err("图片 data URL 格式无效。".to_string());
    };
    if !header.eq_ignore_ascii_case("data:image/png;base64") {
        return Err("默认保存路径导出目前只接受 PNG 图片。".to_string());
    }
    if payload.len() > MAX_SAVE_EXPORT_BYTES * 2 {
        return Err("图片 data URL 过大，已拒绝保存。".to_string());
    }
    let bytes = general_purpose::STANDARD
        .decode(payload)
        .map_err(|error| format!("解析图片 data URL 失败: {error}"))?;
    validate_temp_image_bytes(&bytes)?;
    Ok(bytes)
}

async fn powershell_json(script: &str, timeout_secs: u64) -> Result<Value, String> {
    #[cfg(not(windows))]
    {
        let _ = script;
        let _ = timeout_secs;
        Err("Agent 浮层系统能力目前仅支持 Windows 桌面端。".to_string())
    }

    #[cfg(windows)]
    {
        let _permit = island_powershell_semaphore()
            .acquire()
            .await
            .map_err(|_| "系统能力队列不可用。".to_string())?;
        let encoded = encode_powershell(script);
        let mut command = Command::new("powershell");
        command.kill_on_drop(true).args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            encoded.as_str(),
        ]);
        let output = timeout(Duration::from_secs(timeout_secs), command.output())
            .await
            .map_err(|_| "系统能力调用超时。".to_string())?
            .map_err(|error| format!("无法启动 PowerShell: {error}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() {
            return Err(if stderr.is_empty() { stdout } else { stderr });
        }
        parse_json_output(&stdout)
    }
}

fn encode_powershell(script: &str) -> String {
    let full = format!(
        "$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'; [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new(); {script}"
    );
    let bytes: Vec<u8> = full
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    general_purpose::STANDARD.encode(bytes)
}

fn parse_json_output(output: &str) -> Result<Value, String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Err("系统能力调用没有返回结果。".to_string());
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Ok(value);
    }
    for line in trimmed.lines().rev() {
        let candidate = line.trim();
        if candidate.starts_with('{') || candidate.starts_with('[') {
            if let Ok(value) = serde_json::from_str::<Value>(candidate) {
                return Ok(value);
            }
        }
    }
    Err(format!("系统能力返回了非 JSON 数据: {trimmed}"))
}

fn ps_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn winrt_file_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = value.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
    }
    value.to_string()
}

fn value_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn value_i64(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or_default()
}

fn value_string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn push_unique(warnings: &mut Vec<String>, warning: &str) {
    if !warnings.iter().any(|item| item == warning) {
        warnings.push(warning.to_string());
    }
}

fn annotate_ocr_quality(value: &mut Value) {
    let text = value_string(value, "text");
    let mut lines = value_string_array(value, "lines");
    if lines.is_empty() && !text.is_empty() {
        lines = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect();
    }
    let mut warnings = value_string_array(value, "qualityWarnings");
    let confidence_available = value
        .get("confidence")
        .map(|item| !item.is_null())
        .unwrap_or(false);

    if let Some(confidence) = value.get("confidence").and_then(Value::as_f64) {
        if confidence < 0.60 {
            push_unique(
                &mut warnings,
                "OCR 置信度偏低，可能存在漏字或错字，发送前请人工核对。",
            );
        }
    } else if !text.is_empty() {
        push_unique(
            &mut warnings,
            "当前 OCR 引擎没有返回置信度，结果需要人工核对。",
        );
    }

    if text.is_empty() {
        push_unique(
            &mut warnings,
            "未识别到文字，可能是截图过小、模糊、被遮挡或语言包不匹配。",
        );
    } else {
        let compact_chars = text.chars().filter(|ch| !ch.is_whitespace()).count();
        let trimmed_lines: Vec<&str> = lines
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect();
        let avg_line_len = if trimmed_lines.is_empty() {
            compact_chars as f64
        } else {
            trimmed_lines
                .iter()
                .map(|line| line.chars().count())
                .sum::<usize>() as f64
                / trimmed_lines.len() as f64
        };
        if (trimmed_lines.len() >= 3 && avg_line_len <= 4.0) || compact_chars <= 8 {
            push_unique(
                &mut warnings,
                "OCR 文本疑似来自小字或模糊截图，短行结果容易误识别。",
            );
        }
        let table_like_lines = trimmed_lines
            .iter()
            .filter(|line| {
                line.contains('\t')
                    || line.contains('|')
                    || line.matches("  ").count() >= 2
                    || line.matches('　').count() >= 2
            })
            .count();
        if table_like_lines >= 2 {
            push_unique(
                &mut warnings,
                "截图可能包含表格或多列内容，OCR 行列顺序可能不可靠。",
            );
        }
    }

    if let Some(object) = value.as_object_mut() {
        object.insert(
            "confidenceAvailable".to_string(),
            json!(confidence_available),
        );
        object.insert("qualityWarnings".to_string(), json!(warnings.clone()));
        let current_warning = object
            .get("warning")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if current_warning.is_empty() && !warnings.is_empty() {
            object.insert("warning".to_string(), Value::String(warnings.join(" ")));
        }
    }
}

fn is_browser_process(process_name: &str) -> bool {
    let normalized = normalized_process_name(process_name);
    matches!(
        normalized.as_str(),
        "chrome" | "msedge" | "firefox" | "brave" | "opera" | "vivaldi" | "arc" | "iexplore"
    )
}

fn normalized_process_name(process_name: &str) -> String {
    let trimmed = process_name.trim().trim_matches('"');
    Path::new(trimmed)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(trimmed)
        .trim_end_matches(".exe")
        .to_ascii_lowercase()
}

fn annotate_window_context(value: &mut Value) {
    let title = value_string(value, "title");
    let process_name = value_string(value, "processName");
    let executable_path = value_string(value, "executablePath");
    let process_id = value_i64(value, "processId");
    let mut warnings = value_string_array(value, "warnings");
    let mut context_level = "window_metadata";

    if process_id <= 0 {
        push_unique(
            &mut warnings,
            "无法确定前台窗口进程，可能是系统窗口、权限受限窗口或窗口已切换。",
        );
    }
    if title.is_empty() {
        push_unique(
            &mut warnings,
            "前台窗口没有可读取标题，Atlas 只能发送有限元信息。",
        );
    }
    if process_name.is_empty() {
        push_unique(
            &mut warnings,
            "无法读取前台进程名称，可能受到权限或系统限制。",
        );
    }
    if !process_name.is_empty() && executable_path.is_empty() {
        push_unique(
            &mut warnings,
            "无法读取可执行路径，常见于 UWP、管理员权限或受保护进程。",
        );
    }
    if is_browser_process(&process_name) {
        context_level = "title_only";
        push_unique(
            &mut warnings,
            "浏览器窗口只采集标题级上下文，未读取 URL、DOM 或页面正文。",
        );
    }
    if normalized_process_name(&process_name) == "applicationframehost" {
        push_unique(
            &mut warnings,
            "该窗口可能是 UWP 容器，应用身份和正文需要额外授权路径确认。",
        );
    }

    if let Some(object) = value.as_object_mut() {
        object.insert("contextLevel".to_string(), json!(context_level));
        object.insert("warnings".to_string(), json!(warnings.clone()));
        if !warnings.is_empty() {
            object.insert("warning".to_string(), Value::String(warnings.join(" ")));
        }
    }
}

fn log_island_activity(
    state: &State<'_, AppState>,
    kind: &str,
    title: &str,
    detail: &str,
    metadata: Value,
) -> Result<(), String> {
    state
        .local_db
        .log_activity_event(LogActivityEventPayload {
            date: None,
            kind: kind.to_string(),
            title: title.to_string(),
            detail: detail.to_string(),
            metadata,
        })
        .map(|_| ())
        .map_err(|error| format!("浮层审计写入失败: {error}"))
}

fn load_island_settings(state: &State<'_, AppState>) -> Result<IslandSettingsPayload, String> {
    let mut settings = match state
        .local_db
        .get_app_state(ISLAND_SETTINGS_KEY)
        .map_err(|error| format!("读取 Agent 浮层设置失败: {error}"))?
    {
        Some(value) => serde_json::from_value(value)
            .map_err(|error| format!("解析 Agent 浮层设置失败: {error}")),
        None => Ok(IslandSettingsPayload::default()),
    }?;
    normalize_island_settings(&mut settings);
    apply_smoke_island_settings(&mut settings);
    Ok(settings)
}

pub fn persist_island_manual_hidden(
    local_db: &LocalDb,
    manual_hidden: bool,
) -> Result<IslandSettingsPayload, String> {
    let mut settings = load_island_settings_from_db(local_db)?;
    settings.manual_hidden = manual_hidden;
    save_island_settings_to_db(local_db, &settings)?;
    apply_smoke_island_settings(&mut settings);
    Ok(settings)
}

pub fn persist_island_privacy_paused(
    local_db: &LocalDb,
    privacy_paused: bool,
) -> Result<IslandSettingsPayload, String> {
    let mut settings = load_island_settings_from_db(local_db)?;
    settings.privacy_paused = privacy_paused;
    save_island_settings_to_db(local_db, &settings)?;
    apply_smoke_island_settings(&mut settings);
    Ok(settings)
}

fn normalize_island_settings(settings: &mut IslandSettingsPayload) {
    settings.confirm_before_send = true;
    settings.screenshot.normalize();
}

pub(crate) fn load_island_settings_from_db(
    local_db: &LocalDb,
) -> Result<IslandSettingsPayload, String> {
    let mut settings = match local_db
        .get_app_state(ISLAND_SETTINGS_KEY)
        .map_err(|error| format!("读取 Agent 浮层设置失败: {error}"))?
    {
        Some(value) => serde_json::from_value(value)
            .map_err(|error| format!("解析 Agent 浮层设置失败: {error}")),
        None => Ok(IslandSettingsPayload::default()),
    }?;
    normalize_island_settings(&mut settings);
    Ok(settings)
}

fn apply_smoke_island_settings(settings: &mut IslandSettingsPayload) {
    if std::env::var("ATLAS_SMOKE_SHOW_FLOAT").ok().as_deref() == Some("1") {
        settings.enabled = true;
        settings.idle_hide = false;
        settings.manual_hidden = false;
        settings.capabilities.task_status = true;
    }
    if std::env::var("ATLAS_SMOKE_ENABLE_ISLAND_CAPABILITIES")
        .ok()
        .as_deref()
        == Some("1")
    {
        settings.privacy_paused = false;
        settings.capabilities.screenshot = true;
        settings.capabilities.ocr = true;
        settings.capabilities.window_context = true;
        settings.capabilities.clipboard = true;
        settings.capabilities.media = true;
        settings.capabilities.network = true;
    }
    if let Ok(directory) = std::env::var("ATLAS_SMOKE_SCREENSHOT_DEFAULT_SAVE_DIR") {
        settings.screenshot.default_save_directory = normalize_save_directory(&directory);
    }
}

fn preserve_persisted_smoke_overrides_before_save(
    settings: &mut IslandSettingsPayload,
    persisted: &IslandSettingsPayload,
) {
    if std::env::var("ATLAS_SMOKE_SHOW_FLOAT").ok().as_deref() == Some("1") {
        settings.enabled = persisted.enabled;
        settings.idle_hide = persisted.idle_hide;
        settings.manual_hidden = persisted.manual_hidden;
        settings.capabilities.task_status = persisted.capabilities.task_status;
    }
    if std::env::var("ATLAS_SMOKE_ENABLE_ISLAND_CAPABILITIES")
        .ok()
        .as_deref()
        == Some("1")
    {
        settings.privacy_paused = persisted.privacy_paused;
        settings.capabilities.screenshot = persisted.capabilities.screenshot;
        settings.capabilities.ocr = persisted.capabilities.ocr;
        settings.capabilities.window_context = persisted.capabilities.window_context;
        settings.capabilities.clipboard = persisted.capabilities.clipboard;
        settings.capabilities.media = persisted.capabilities.media;
        settings.capabilities.network = persisted.capabilities.network;
    }
    if let Ok(directory) = std::env::var("ATLAS_SMOKE_SCREENSHOT_DEFAULT_SAVE_DIR") {
        settings.screenshot.default_save_directory = normalize_save_directory(&directory);
    }
}

fn write_ocr_smoke_proof(image_path: &str, value: &Value) {
    if std::env::var("ATLAS_SMOKE_ENABLE_ISLAND_CAPABILITIES")
        .ok()
        .as_deref()
        != Some("1")
    {
        return;
    }

    let smoke_run_id = std::env::var("ATLAS_SMOKE_RUN_ID").unwrap_or_default();
    let proof = json!({
        "ok": value.get("ok").and_then(Value::as_bool).unwrap_or(false),
        "kind": "ocr_smoke_proof",
        "smokeRunId": smoke_run_id,
        "imagePath": image_path,
        "source": value.get("source").cloned().unwrap_or(Value::Null),
        "available": value.get("available").cloned().unwrap_or(Value::Null),
        "textLength": value.get("text").and_then(Value::as_str).map(|text| text.chars().count()).unwrap_or(0),
        "lineCount": value.get("lines").and_then(Value::as_array).map(|lines| lines.len()).unwrap_or(0),
        "language": value.get("language").cloned().unwrap_or(Value::Null),
        "warning": value.get("warning").cloned().unwrap_or(Value::Null),
        "reason": value.get("reason").cloned().unwrap_or(Value::Null),
        "windowsOcrError": value.get("windowsOcrError").cloned().unwrap_or(Value::Null),
        "exitCode": value.get("exitCode").cloned().unwrap_or(Value::Null),
        "qualityWarnings": value.get("qualityWarnings").cloned().unwrap_or(Value::Null),
        "capturedAt": now_ms(),
    });
    let path = std::env::temp_dir().join(format!(
        "atlas-island-ocr-smoke-{}-{}.json",
        proof
            .get("smokeRunId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("unknown"),
        uuid::Uuid::new_v4()
    ));
    if let Ok(bytes) = serde_json::to_vec(&proof) {
        let _ = std::fs::write(path, bytes);
    }
}

fn write_screenshot_smoke_proof(value: &Value) {
    if std::env::var("ATLAS_SMOKE_ENABLE_ISLAND_CAPABILITIES")
        .ok()
        .as_deref()
        != Some("1")
    {
        return;
    }

    let smoke_run_id = std::env::var("ATLAS_SMOKE_RUN_ID").unwrap_or_default();
    let proof = json!({
        "ok": value.get("ok").and_then(Value::as_bool).unwrap_or(false),
        "kind": "screenshot_smoke_proof",
        "smokeRunId": smoke_run_id,
        "mode": value.get("mode").cloned().unwrap_or(Value::Null),
        "tempPath": value.get("tempPath").cloned().unwrap_or(Value::Null),
        "mime": value.get("mime").cloned().unwrap_or(Value::Null),
        "width": value.get("width").cloned().unwrap_or(Value::Null),
        "height": value.get("height").cloned().unwrap_or(Value::Null),
        "x": value.get("x").cloned().unwrap_or(Value::Null),
        "y": value.get("y").cloned().unwrap_or(Value::Null),
        "source": value.get("source").cloned().unwrap_or(Value::Null),
        "size": value.get("size").cloned().unwrap_or(Value::Null),
        "backendCapturedAt": value.get("capturedAt").cloned().unwrap_or(Value::Null),
        "capturedAt": now_ms(),
    });
    let path = std::env::temp_dir().join(format!(
        "atlas-island-screenshot-smoke-{}-{}.json",
        proof
            .get("smokeRunId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("unknown"),
        uuid::Uuid::new_v4()
    ));
    if let Ok(bytes) = serde_json::to_vec(&proof) {
        let _ = std::fs::write(path, bytes);
    }
}

fn write_screen_pixel_smoke_proof(value: &Value) {
    if std::env::var("ATLAS_SMOKE_ENABLE_ISLAND_CAPABILITIES")
        .ok()
        .as_deref()
        != Some("1")
    {
        return;
    }

    let smoke_run_id = std::env::var("ATLAS_SMOKE_RUN_ID").unwrap_or_default();
    let sample_count = value
        .get("pixels")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| row.as_array().map(|columns| columns.len()).unwrap_or(0))
                .sum::<usize>()
        })
        .unwrap_or(0);
    let proof = json!({
        "ok": value.get("ok").and_then(Value::as_bool).unwrap_or(false),
        "kind": "screen_pixel_smoke_proof",
        "smokeRunId": smoke_run_id,
        "x": value.get("x").cloned().unwrap_or(Value::Null),
        "y": value.get("y").cloned().unwrap_or(Value::Null),
        "requestedX": value.get("requestedX").cloned().unwrap_or(Value::Null),
        "requestedY": value.get("requestedY").cloned().unwrap_or(Value::Null),
        "sampleX": value.get("sampleX").cloned().unwrap_or(Value::Null),
        "sampleY": value.get("sampleY").cloned().unwrap_or(Value::Null),
        "size": value.get("size").cloned().unwrap_or(Value::Null),
        "centerColumn": value.get("centerColumn").cloned().unwrap_or(Value::Null),
        "centerRow": value.get("centerRow").cloned().unwrap_or(Value::Null),
        "r": value.get("r").cloned().unwrap_or(Value::Null),
        "g": value.get("g").cloned().unwrap_or(Value::Null),
        "b": value.get("b").cloned().unwrap_or(Value::Null),
        "hex": value.get("hex").cloned().unwrap_or(Value::Null),
        "source": value.get("source").cloned().unwrap_or(Value::Null),
        "sampleCount": sample_count,
        "backendCapturedAt": value.get("capturedAt").cloned().unwrap_or(Value::Null),
        "capturedAt": now_ms(),
    });
    let path = std::env::temp_dir().join(format!(
        "atlas-island-screen-pixel-smoke-{}-{}.json",
        proof
            .get("smokeRunId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("unknown"),
        uuid::Uuid::new_v4()
    ));
    if let Ok(bytes) = serde_json::to_vec(&proof) {
        let _ = std::fs::write(path, bytes);
    }
}

fn write_system_status_smoke_proof(value: &Value, request_kind: &str) {
    if std::env::var("ATLAS_SMOKE_ENABLE_ISLAND_CAPABILITIES")
        .ok()
        .as_deref()
        != Some("1")
    {
        return;
    }

    let smoke_run_id = std::env::var("ATLAS_SMOKE_RUN_ID").unwrap_or_default();
    let proof = json!({
        "ok": value.get("ok").and_then(Value::as_bool).unwrap_or(false),
        "kind": "system_status_smoke_proof",
        "smokeRunId": smoke_run_id,
        "requestKind": request_kind,
        "source": value.get("source").cloned().unwrap_or(Value::Null),
        "backendCapturedAt": value.get("capturedAt").cloned().unwrap_or(Value::Null),
        "networkOk": value.pointer("/network/ok").and_then(Value::as_bool).unwrap_or(false),
        "rxBytesPerSec": value.pointer("/network/rxBytesPerSec").cloned().unwrap_or(Value::Null),
        "txBytesPerSec": value.pointer("/network/txBytesPerSec").cloned().unwrap_or(Value::Null),
        "cpuLoadPercent": value.pointer("/cpu/loadPercent").cloned().unwrap_or(Value::Null),
        "memoryTotalBytes": value.pointer("/memory/totalBytes").cloned().unwrap_or(Value::Null),
        "memoryFreeBytes": value.pointer("/memory/freeBytes").cloned().unwrap_or(Value::Null),
        "hasCpuLoad": value.pointer("/cpu/loadPercent").and_then(Value::as_f64).map(f64::is_finite).unwrap_or(false),
        "hasMemory": value.pointer("/memory/totalBytes").and_then(Value::as_i64).is_some_and(|bytes| bytes > 0)
            && value.pointer("/memory/freeBytes").and_then(Value::as_i64).is_some_and(|bytes| bytes >= 0),
        "capturedAt": now_ms(),
    });
    let path = std::env::temp_dir().join(format!(
        "atlas-island-system-status-smoke-{}-{}.json",
        proof
            .get("smokeRunId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("unknown"),
        uuid::Uuid::new_v4()
    ));
    if let Ok(bytes) = serde_json::to_vec(&proof) {
        let _ = std::fs::write(path, bytes);
    }
}

fn write_context_import_smoke_proof(package_id: &str, source: &str, proof: Option<&Value>) {
    if std::env::var("ATLAS_SMOKE_ENABLE_ISLAND_CAPABILITIES")
        .ok()
        .as_deref()
        != Some("1")
    {
        return;
    }

    let smoke_run_id = std::env::var("ATLAS_SMOKE_RUN_ID").unwrap_or_default();
    let detail = proof.unwrap_or(&Value::Null);
    let smoke_proof = json!({
        "ok": true,
        "kind": "context_import_smoke_proof",
        "smokeRunId": smoke_run_id,
        "packageId": truncate_smoke_text(package_id, 160),
        "source": truncate_smoke_text(source, 80),
        "attachmentCount": detail.get("attachmentCount").and_then(Value::as_u64).unwrap_or(0),
        "imageAttached": detail.get("imageAttached").and_then(Value::as_bool).unwrap_or(false),
        "tempImageImported": detail.get("tempImageImported").and_then(Value::as_bool).unwrap_or(false),
        "textLength": detail.get("textLength").and_then(Value::as_u64).unwrap_or(0),
        "capturedAt": now_ms(),
    });
    let path = std::env::temp_dir().join(format!(
        "atlas-island-context-import-smoke-{}-{}.json",
        smoke_proof
            .get("smokeRunId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("unknown"),
        uuid::Uuid::new_v4()
    ));
    if let Ok(bytes) = serde_json::to_vec(&smoke_proof) {
        let _ = std::fs::write(path, bytes);
    }
}

fn keyboard_smoke_proof_enabled() -> bool {
    std::env::var("ATLAS_SMOKE_ENABLE_ISLAND_KEYBOARD_PROOF")
        .ok()
        .as_deref()
        == Some("1")
        || std::env::var("ATLAS_SMOKE_ENABLE_ISLAND_CAPABILITIES")
            .ok()
            .as_deref()
            == Some("1")
}

fn truncate_smoke_text(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn save_island_settings_to_db(
    local_db: &LocalDb,
    settings: &IslandSettingsPayload,
) -> Result<(), String> {
    let mut normalized = settings.clone();
    normalize_island_settings(&mut normalized);
    local_db
        .set_app_state(
            ISLAND_SETTINGS_KEY,
            serde_json::to_value(&normalized).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("保存 Agent 浮层设置失败: {error}"))
}

fn ensure_island_ui_window(window: &Window) -> Result<(), String> {
    ensure_island_ui_window_label(window.label())
}

fn ensure_island_ui_window_label(label: &str) -> Result<(), String> {
    match label {
        "main" | "float" => Ok(()),
        other => Err(format!("窗口 {other} 不允许调用 Agent 浮层系统能力。")),
    }
}

fn ensure_capture_overlay_window_label(label: &str) -> Result<(), String> {
    if label == "capture-overlay" {
        return Ok(());
    }
    Err(format!("窗口 {label} 不允许调用截图 overlay 取色能力。"))
}

fn ensure_island_show_main_window_label(label: &str) -> Result<(), String> {
    if ensure_island_ui_window_label(label).is_ok() || is_island_sticker_window_label(label) {
        return Ok(());
    }
    Err(format!("窗口 {label} 不允许唤起 Atlas 主窗口。"))
}

fn ensure_island_collection_allowed(
    window: &Window,
    state: &State<'_, AppState>,
    capability: IslandCapability,
) -> Result<(), String> {
    if !(matches!(capability, IslandCapability::Ocr)
        && is_island_sticker_window_label(window.label()))
    {
        ensure_island_collection_window_label(window.label())?;
    }

    let settings = load_island_settings(state)?;
    if !settings.enabled {
        return Err("Agent 浮层已关闭，系统感知不可用。".to_string());
    }
    if settings.privacy_paused {
        return Err("隐私暂停开启中，系统感知不可用。".to_string());
    }

    let allowed = match capability {
        IslandCapability::Screenshot => settings.capabilities.screenshot,
        IslandCapability::Ocr => settings.capabilities.ocr,
        IslandCapability::WindowContext => settings.capabilities.window_context,
        IslandCapability::Clipboard => settings.capabilities.clipboard,
        IslandCapability::Media => settings.capabilities.media,
        IslandCapability::Network => settings.capabilities.network,
    };
    if !allowed {
        return Err(format!(
            "{} 已在 Agent 浮层设置中关闭。",
            capability_label(capability)
        ));
    }
    Ok(())
}

fn ensure_island_collection_window_label(label: &str) -> Result<(), String> {
    if label == "float" {
        return Ok(());
    }
    Err(format!("窗口 {label} 不允许直接调用 Agent 浮层采集能力。"))
}

fn is_island_sticker_window_label(label: &str) -> bool {
    label
        .strip_prefix("sticker-")
        .map(|suffix| {
            !suffix.is_empty()
                && suffix
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        })
        .unwrap_or(false)
}

fn capability_label(capability: IslandCapability) -> &'static str {
    match capability {
        IslandCapability::Screenshot => "手动截图",
        IslandCapability::Ocr => "OCR 识别",
        IslandCapability::WindowContext => "前台窗口上下文",
        IslandCapability::Clipboard => "剪贴板导入",
        IslandCapability::Media => "媒体识别",
        IslandCapability::Network => "网速和系统状态",
    }
}

fn validate_capture_bounds(width: i32, height: i32) -> Result<(), String> {
    if width <= 0 || height <= 0 {
        return Err("截图区域无效。".to_string());
    }
    if width > MAX_CAPTURE_SIDE
        || height > MAX_CAPTURE_SIDE
        || i64::from(width) * i64::from(height) > MAX_CAPTURE_PIXELS
    {
        return Err("截图区域过大。".to_string());
    }
    Ok(())
}

fn describe_capture_error(error: &str) -> String {
    let normalized = error.to_ascii_lowercase();
    let reason = if normalized.contains("没有可截图的前台窗口") {
        "没有可截图的前台窗口"
    } else if normalized.contains("已最小化") || normalized.contains("iconic") {
        "当前窗口已最小化或不可见"
    } else if normalized.contains("无法读取当前窗口位置") || normalized.contains("getwindowrect")
    {
        "窗口不可捕获或位置不可读"
    } else if normalized.contains("截图区域无效") {
        "截图区域无效"
    } else if normalized.contains("截图区域过大") {
        "截图区域过大"
    } else if normalized.contains("access is denied")
        || normalized.contains("拒绝访问")
        || normalized.contains("protected")
        || normalized.contains("copyfromscreen")
        || normalized.contains("gdi")
    {
        "系统拒绝屏幕捕获，可能是权限、受保护窗口、屏幕保护或系统策略限制"
    } else if normalized.contains("timeout") || normalized.contains("超时") {
        "系统截图调用超时"
    } else {
        "系统截图调用失败"
    };
    let raw = error.trim();
    if raw.is_empty() {
        format!("截图失败：{reason}。")
    } else {
        format!("截图失败：{reason}。原始错误：{raw}")
    }
}

fn cleanup_expired_atlas_temp_files() {
    let temp_dir = std::env::temp_dir();
    let Ok(entries) = std::fs::read_dir(temp_dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !file_name.starts_with("atlas-island-") || !file_name.ends_with(".png") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let expired = now
            .duration_since(modified)
            .map(|duration| duration.as_millis() > u128::from(TEMP_FILE_TTL_MS))
            .unwrap_or(false);
        if expired {
            let _ = std::fs::remove_file(path);
        }
    }
    if let Ok(mut registry) = temp_registry().lock() {
        registry.retain(|path, record| {
            let expired = now_ms().saturating_sub(record.created_at_ms) > TEMP_FILE_TTL_MS;
            !expired && path.exists()
        });
    }
}

pub fn cleanup_expired_island_temp_files() {
    cleanup_expired_atlas_temp_files();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn island_collection_commands_are_float_only() {
        assert!(ensure_island_collection_window_label("float").is_ok());
        assert!(ensure_island_collection_window_label("main").is_err());
        assert!(ensure_island_collection_window_label("sticker-sticker_123").is_err());
        assert!(ensure_island_collection_window_label("audio-player").is_err());
    }

    #[test]
    fn sticker_window_labels_are_scoped() {
        assert!(is_island_sticker_window_label("sticker-sticker_123"));
        assert!(is_island_sticker_window_label("sticker-a-b_c"));
        assert!(!is_island_sticker_window_label("sticker-"));
        assert!(!is_island_sticker_window_label("sticker-../main"));
        assert!(!is_island_sticker_window_label("main"));
    }

    #[test]
    fn sticker_windows_are_not_general_island_ui_windows() {
        assert!(ensure_island_ui_window_label("main").is_ok());
        assert!(ensure_island_ui_window_label("float").is_ok());
        assert!(ensure_island_ui_window_label("sticker-sticker_123").is_err());
        assert!(ensure_island_show_main_window_label("sticker-sticker_123").is_ok());
    }

    #[test]
    fn screen_pixel_sampling_is_overlay_only() {
        assert!(ensure_capture_overlay_window_label("capture-overlay").is_ok());
        assert!(ensure_capture_overlay_window_label("float").is_err());
        assert!(ensure_capture_overlay_window_label("main").is_err());
        assert!(ensure_capture_overlay_window_label("sticker-sticker_123").is_err());
    }

    #[test]
    fn capture_bounds_reject_oversized_areas() {
        assert!(validate_capture_bounds(800, 600).is_ok());
        assert!(validate_capture_bounds(0, 600).is_err());
        assert!(validate_capture_bounds(20_000, 600).is_err());
        assert!(validate_capture_bounds(10_000, 10_000).is_err());
    }

    #[test]
    fn notification_and_weather_capabilities_default_closed() {
        let settings = IslandSettingsPayload::default();
        assert!(!settings.capabilities.notifications);
        assert!(!settings.capabilities.weather);
    }

    #[test]
    fn screenshot_sticker_default_opacity_is_persisted_and_normalized() {
        let mut missing_screenshot: IslandSettingsPayload = serde_json::from_value(json!({
            "enabled": true,
            "capabilities": {}
        }))
        .unwrap();
        normalize_island_settings(&mut missing_screenshot);
        assert_eq!(missing_screenshot.screenshot.sticker_default_opacity, 1.0);
        assert!(!missing_screenshot.screenshot.auto_ocr_after_capture);
        assert_eq!(missing_screenshot.screenshot.default_save_directory, "");
        assert_eq!(missing_screenshot.screenshot.main_shortcut, "Ctrl+Alt+A");
        assert_eq!(missing_screenshot.screenshot.pin_shortcut, "Ctrl+Alt+P");
        assert_eq!(missing_screenshot.screenshot.delay_shortcut, "Ctrl+Alt+D");
        assert_eq!(missing_screenshot.screenshot.result_enter_action, "copy");
        assert_eq!(missing_screenshot.screenshot.result_ctrl_c_action, "copy");
        assert_eq!(
            missing_screenshot.screenshot.result_double_click_action,
            "copy"
        );
        assert_eq!(missing_screenshot.screenshot.color_format, "hex");
        assert_eq!(missing_screenshot.screenshot.alternate_color_format, "rgb");
        assert!(missing_screenshot.screenshot.show_magnifier);
        assert_eq!(missing_screenshot.screenshot.magnifier_scale, 3.0);

        let mut custom_opacity: IslandSettingsPayload = serde_json::from_value(json!({
            "enabled": true,
            "screenshot": {
                "stickerDefaultOpacity": 0.55,
                "autoOcrAfterCapture": true,
                "defaultSaveDirectory": "  C:\\\\AtlasSmokeWritable  ",
                "mainShortcut": "  Ctrl+Shift+1  ",
                "pinShortcut": "Ctrl+Shift+2",
                "delayShortcut": "Ctrl+Shift+3",
                "resultEnterAction": "save",
                "resultCtrlCAction": "save",
                "resultDoubleClickAction": "none",
                "colorFormat": "hsl",
                "alternateColorFormat": "hsv",
                "showMagnifier": false,
                "magnifierScale": 4
            },
            "capabilities": {}
        }))
        .unwrap();
        normalize_island_settings(&mut custom_opacity);
        assert_eq!(custom_opacity.screenshot.sticker_default_opacity, 0.55);
        assert!(custom_opacity.screenshot.auto_ocr_after_capture);
        assert_eq!(
            custom_opacity.screenshot.default_save_directory,
            "C:\\\\AtlasSmokeWritable"
        );
        assert_eq!(custom_opacity.screenshot.main_shortcut, "Ctrl+Shift+1");
        assert_eq!(custom_opacity.screenshot.pin_shortcut, "Ctrl+Shift+2");
        assert_eq!(custom_opacity.screenshot.delay_shortcut, "Ctrl+Shift+3");
        assert_eq!(custom_opacity.screenshot.result_enter_action, "save");
        assert_eq!(custom_opacity.screenshot.result_ctrl_c_action, "save");
        assert_eq!(custom_opacity.screenshot.result_double_click_action, "none");
        assert_eq!(custom_opacity.screenshot.color_format, "hsl");
        assert_eq!(custom_opacity.screenshot.alternate_color_format, "hsv");
        assert!(!custom_opacity.screenshot.show_magnifier);
        assert_eq!(custom_opacity.screenshot.magnifier_scale, 4.0);

        custom_opacity.screenshot.sticker_default_opacity = 0.1;
        normalize_island_settings(&mut custom_opacity);
        assert_eq!(custom_opacity.screenshot.sticker_default_opacity, 0.35);

        let mut null_opacity: IslandSettingsPayload = serde_json::from_value(json!({
            "enabled": true,
            "screenshot": { "stickerDefaultOpacity": null },
            "capabilities": {}
        }))
        .unwrap();
        normalize_island_settings(&mut null_opacity);
        assert_eq!(null_opacity.screenshot.sticker_default_opacity, 1.0);

        let mut string_opacity: IslandSettingsPayload = serde_json::from_value(json!({
            "enabled": true,
            "screenshot": {
                "stickerDefaultOpacity": "0.72",
                "autoOcrAfterCapture": "true",
                "defaultSaveDirectory": 42,
                "mainShortcut": 42,
                "pinShortcut": null,
                "delayShortcut": "",
                "resultEnterAction": "send",
                "resultCtrlCAction": "send",
                "resultDoubleClickAction": "save",
                "colorFormat": "RGB",
                "alternateColorFormat": "HEX",
                "showMagnifier": "false",
                "magnifierScale": "5"
            },
            "capabilities": {}
        }))
        .unwrap();
        normalize_island_settings(&mut string_opacity);
        assert_eq!(string_opacity.screenshot.sticker_default_opacity, 0.72);
        assert!(string_opacity.screenshot.auto_ocr_after_capture);
        assert_eq!(string_opacity.screenshot.default_save_directory, "");
        assert_eq!(string_opacity.screenshot.main_shortcut, "Ctrl+Alt+A");
        assert_eq!(string_opacity.screenshot.pin_shortcut, "Ctrl+Alt+P");
        assert_eq!(string_opacity.screenshot.delay_shortcut, "Ctrl+Alt+D");
        assert_eq!(string_opacity.screenshot.result_enter_action, "send");
        assert_eq!(string_opacity.screenshot.result_ctrl_c_action, "send");
        assert_eq!(string_opacity.screenshot.result_double_click_action, "save");
        assert_eq!(string_opacity.screenshot.color_format, "rgb");
        assert_eq!(string_opacity.screenshot.alternate_color_format, "hex");
        assert!(!string_opacity.screenshot.show_magnifier);
        assert_eq!(string_opacity.screenshot.magnifier_scale, 5.0);

        let mut object_opacity: IslandSettingsPayload = serde_json::from_value(json!({
            "enabled": true,
            "screenshot": {
                "stickerDefaultOpacity": { "bad": true },
                "autoOcrAfterCapture": { "bad": true },
                "defaultSaveDirectory": { "bad": true },
                "mainShortcut": { "bad": true },
                "pinShortcut": { "bad": true },
                "delayShortcut": { "bad": true },
                "resultEnterAction": { "bad": true },
                "resultCtrlCAction": { "bad": true },
                "resultDoubleClickAction": { "bad": true },
                "colorFormat": { "bad": true },
                "alternateColorFormat": { "bad": true },
                "showMagnifier": { "bad": true },
                "magnifierScale": { "bad": true }
            },
            "capabilities": {}
        }))
        .unwrap();
        normalize_island_settings(&mut object_opacity);
        assert_eq!(object_opacity.screenshot.sticker_default_opacity, 1.0);
        assert!(!object_opacity.screenshot.auto_ocr_after_capture);
        assert_eq!(object_opacity.screenshot.default_save_directory, "");
        assert_eq!(object_opacity.screenshot.main_shortcut, "Ctrl+Alt+A");
        assert_eq!(object_opacity.screenshot.pin_shortcut, "Ctrl+Alt+P");
        assert_eq!(object_opacity.screenshot.delay_shortcut, "Ctrl+Alt+D");
        assert_eq!(object_opacity.screenshot.result_enter_action, "copy");
        assert_eq!(object_opacity.screenshot.result_ctrl_c_action, "copy");
        assert_eq!(object_opacity.screenshot.result_double_click_action, "copy");
        assert_eq!(object_opacity.screenshot.color_format, "hex");
        assert_eq!(object_opacity.screenshot.alternate_color_format, "rgb");
        assert!(object_opacity.screenshot.show_magnifier);
        assert_eq!(object_opacity.screenshot.magnifier_scale, 3.0);

        let mut bad_action: IslandSettingsPayload = serde_json::from_value(json!({
            "enabled": true,
            "screenshot": {
                "stickerDefaultOpacity": 0.8,
                "resultEnterAction": "delete-everything",
                "resultCtrlCAction": "delete-everything",
                "resultDoubleClickAction": "delete-everything",
                "colorFormat": "pantone",
                "alternateColorFormat": "cmyk",
                "magnifierScale": 99
            },
            "capabilities": {}
        }))
        .unwrap();
        normalize_island_settings(&mut bad_action);
        assert_eq!(bad_action.screenshot.result_enter_action, "copy");
        assert_eq!(bad_action.screenshot.result_ctrl_c_action, "copy");
        assert_eq!(bad_action.screenshot.result_double_click_action, "copy");
        assert_eq!(bad_action.screenshot.color_format, "hex");
        assert_eq!(bad_action.screenshot.alternate_color_format, "rgb");
        assert_eq!(bad_action.screenshot.magnifier_scale, 5.0);
    }

    #[test]
    fn save_path_permission_probe_reports_real_directory_states() {
        let base = std::env::temp_dir().join(format!("atlas-save-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&base).unwrap();
        let writable = check_save_directory_permission(&base.to_string_lossy());
        assert!(writable.ok);
        assert_eq!(writable.status, "writable");
        assert!(base.exists());

        let missing = check_save_directory_permission(&base.join("missing").to_string_lossy());
        assert!(!missing.ok);
        assert_eq!(missing.status, "missing");

        let file_path = base.join("not-directory.txt");
        std::fs::write(&file_path, b"not a dir").unwrap();
        let not_directory = check_save_directory_permission(&file_path.to_string_lossy());
        assert!(!not_directory.ok);
        assert_eq!(not_directory.status, "not_directory");

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir(&base);
    }

    #[test]
    fn save_export_file_names_are_sanitized_and_unique() {
        let base = std::env::temp_dir().join(format!("atlas-export-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&base).unwrap();
        let sanitized = sanitize_save_file_name(" ..atlas:<bad>|name?.png ");
        assert_eq!(sanitized, "atlas--bad--name-.png");
        let first = unique_save_path(&base, &sanitized).unwrap();
        assert_eq!(
            first.file_name().and_then(|value| value.to_str()),
            Some("atlas--bad--name-.png")
        );
        std::fs::write(&first, b"existing").unwrap();
        let second = unique_save_path(&base, &sanitized).unwrap();
        assert_eq!(
            second.file_name().and_then(|value| value.to_str()),
            Some("atlas--bad--name- (1).png")
        );
        let bytes_written = write_export_bytes(&second, b"hello").unwrap();
        assert_eq!(bytes_written, 5);
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
        let _ = std::fs::remove_dir(&base);
    }

    #[test]
    fn capture_errors_are_mapped_to_user_visible_reasons() {
        assert!(
            describe_capture_error("当前窗口已最小化，无法可靠截取窗口。")
                .contains("当前窗口已最小化或不可见")
        );
        assert!(
            describe_capture_error("CopyFromScreen failed: access is denied")
                .contains("权限、受保护窗口、屏幕保护或系统策略限制")
        );
        assert!(describe_capture_error("截图区域过大。").contains("截图区域过大"));
    }

    #[test]
    fn resolve_atlas_temp_png_rejects_non_atlas_paths() {
        let path = std::env::temp_dir().join(format!("not-atlas-{}.png", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"not an image").unwrap();
        let result = resolve_atlas_temp_png(&path.to_string_lossy());
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn registered_temp_png_is_required_for_read_paths() {
        let path = std::env::temp_dir().join(format!("atlas-island-{}.png", uuid::Uuid::new_v4()));
        std::fs::write(&path, [137, 80, 78, 71, 13, 10, 26, 10, 0]).unwrap();
        assert!(resolve_atlas_temp_png(&path.to_string_lossy()).is_ok());
        assert!(resolve_registered_atlas_temp_png(&path.to_string_lossy()).is_err());
        register_atlas_temp_file(&path.to_string_lossy(), 9).unwrap();
        assert!(resolve_registered_atlas_temp_png(&path.to_string_lossy()).is_ok());
        unregister_atlas_temp_file(&path);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn temp_image_validation_rejects_non_png_and_large_files() {
        assert!(validate_temp_image_bytes(b"not png").is_err());
        assert!(validate_temp_image_bytes(&[137, 80, 78, 71, 13, 10, 26, 10]).is_ok());
        let oversized = vec![0_u8; MAX_TEMP_IMAGE_BYTES as usize + 1];
        assert!(validate_temp_image_bytes(&oversized).is_err());
    }

    #[test]
    fn ocr_quality_annotation_flags_tables_and_missing_confidence() {
        let mut value = json!({
            "ok": true,
            "available": true,
            "text": "名称  数量  金额\nA  10  20\nB  30  40",
            "lines": ["名称  数量  金额", "A  10  20", "B  30  40"],
            "confidence": null
        });
        annotate_ocr_quality(&mut value);
        let warnings = value
            .get("qualityWarnings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(warnings
            .iter()
            .any(|item| item.as_str().unwrap_or_default().contains("没有返回置信度")));
        assert!(warnings
            .iter()
            .any(|item| item.as_str().unwrap_or_default().contains("表格或多列")));
    }

    #[test]
    fn ocr_quality_annotation_flags_empty_or_short_text() {
        let mut empty = json!({
            "ok": true,
            "available": true,
            "text": "",
            "lines": [],
            "confidence": null
        });
        annotate_ocr_quality(&mut empty);
        assert!(empty
            .get("warning")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("未识别到文字"));

        let mut short = json!({
            "ok": true,
            "available": true,
            "text": "I\n1\nl",
            "lines": ["I", "1", "l"],
            "confidence": 0.42
        });
        annotate_ocr_quality(&mut short);
        let warning = short
            .get("warning")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(warning.contains("置信度偏低"));
        assert!(warning.contains("小字或模糊"));
    }

    #[test]
    fn window_context_annotation_flags_browser_and_limited_metadata() {
        let mut value = json!({
            "ok": true,
            "title": "Very long browser title",
            "processId": 42,
            "processName": "chrome",
            "executablePath": null
        });
        annotate_window_context(&mut value);
        assert_eq!(
            value.get("contextLevel").and_then(Value::as_str),
            Some("title_only")
        );
        let warning = value
            .get("warning")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(warning.contains("浏览器窗口只采集标题级上下文"));
        assert!(warning.contains("无法读取可执行路径"));
    }

    #[test]
    fn window_context_annotation_normalizes_exe_process_names() {
        let mut browser = json!({
            "ok": true,
            "title": "Browser title",
            "processId": 42,
            "processName": "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
            "executablePath": null
        });
        annotate_window_context(&mut browser);
        assert_eq!(
            browser.get("contextLevel").and_then(Value::as_str),
            Some("title_only")
        );

        let mut uwp = json!({
            "ok": true,
            "title": "Settings",
            "processId": 84,
            "processName": "ApplicationFrameHost.exe",
            "executablePath": null
        });
        annotate_window_context(&mut uwp);
        assert!(uwp
            .get("warning")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("UWP 容器"));
    }

    #[test]
    fn window_context_annotation_flags_no_title_and_unknown_process() {
        let mut value = json!({
            "ok": true,
            "title": "",
            "processId": 0,
            "processName": null,
            "executablePath": null
        });
        annotate_window_context(&mut value);
        let warning = value
            .get("warning")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(warning.contains("无法确定前台窗口进程"));
        assert!(warning.contains("没有可读取标题"));
        assert!(warning.contains("无法读取前台进程名称"));
    }
}
