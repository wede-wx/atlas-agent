use base64::Engine as _;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use uuid::Uuid;

use crate::agent::{
    browser_action_fingerprint, detect_browser_action_loop, judge_browser_step,
    BrowserActionFingerprintInput, BrowserDomSummary, BrowserJudgeResult,
};
use crate::storage::{LocalDb, RecordBrowserAgentStepPayload};

const BROWSER_AUDIT_KEY: &str = "browser_automation_audit_v1";
const BROWSER_AUDIT_LIMIT: usize = 200;
const BROWSER_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scripts/atlas-browser-automation.mjs"
));
const BROWSER_RUNTIME_PACKAGE: &str = r#"{
  "name": "atlas-browser-runtime",
  "private": true,
  "type": "module",
  "dependencies": {
    "playwright": "^1.59.1"
  }
}
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAutomationRequest {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    pub action: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub confirmed: bool,
    #[serde(default)]
    pub headless: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAutomationResult {
    pub ok: bool,
    pub action: String,
    pub target: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub screenshot_path: Option<String>,
    pub error: Option<String>,
    pub dom_summary: Option<BrowserDomSummary>,
    pub fingerprint: Option<String>,
    pub judge: Option<BrowserJudgeResult>,
    pub loop_detected: bool,
    pub step_id: Option<String>,
    pub step_index: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAutomationAuditEvent {
    pub id: String,
    pub action: String,
    pub target: Option<String>,
    pub confirmed: bool,
    pub status: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub screenshot_path: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
}

pub async fn run_browser_automation(
    db: &LocalDb,
    request: BrowserAutomationRequest,
) -> Result<BrowserAutomationResult, String> {
    validate_browser_request(&request)?;
    let (script, runtime_dir) = prepare_browser_runtime().await?;
    let payload = json!({
        "action": request.action.clone(),
        "target": request.target.clone(),
        "keyword": request.keyword.clone(),
        "url": request.url.clone(),
        "selector": request.selector.clone(),
        "text": request.text.clone(),
        "key": request.key.clone(),
        "confirmed": request.confirmed,
        "headless": request.headless.unwrap_or(true),
    });
    let encoded = base64_url_json(&payload)?;
    let output = tokio::process::Command::new("node")
        .arg(script)
        .arg(encoded)
        .current_dir(runtime_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| format!("启动浏览器自动化失败：{error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let parsed: Value = serde_json::from_str(stdout.trim())
        .map_err(|error| format!("浏览器自动化返回解析失败：{error}; {}", stderr.trim()))?;
    let dom_summary = parsed
        .get("domSummary")
        .cloned()
        .and_then(|value| serde_json::from_value::<BrowserDomSummary>(value).ok());
    let fingerprint = browser_action_fingerprint(&BrowserActionFingerprintInput {
        action: request.action.clone(),
        target: request.target.clone(),
        url: parsed
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| request.url.clone()),
        selector: request.selector.clone(),
        key: request.key.clone(),
        text: request.text.clone(),
        dom_summary: dom_summary.clone(),
    });
    let recent_steps = db
        .list_browser_agent_steps(request.run_id.as_deref(), request.session_id.as_deref(), 20)
        .unwrap_or_default();
    let loop_verdict = detect_browser_action_loop(&recent_steps, &fingerprint, 3);
    let judge = judge_browser_step(
        parsed.get("ok").and_then(Value::as_bool).unwrap_or(false),
        &request.action,
        parsed
            .get("url")
            .and_then(Value::as_str)
            .or(request.url.as_deref()),
        dom_summary.as_ref(),
        &loop_verdict,
        parsed.get("error").and_then(Value::as_str),
    );
    let mut result = BrowserAutomationResult {
        ok: parsed.get("ok").and_then(Value::as_bool).unwrap_or(false),
        action: parsed
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        target: parsed
            .get("target")
            .and_then(Value::as_str)
            .map(str::to_string),
        title: parsed
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string),
        url: parsed
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_string),
        screenshot_path: parsed
            .get("screenshotPath")
            .and_then(Value::as_str)
            .map(str::to_string),
        error: parsed
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string),
        dom_summary: dom_summary.clone(),
        fingerprint: Some(fingerprint.clone()),
        judge: Some(judge.clone()),
        loop_detected: loop_verdict.repeated,
        step_id: None,
        step_index: None,
    };
    let action_json = json!({
        "action": request.action.clone(),
        "target": request.target.clone(),
        "keyword": request.keyword.clone(),
        "url": request.url.clone(),
        "selector": request.selector.clone(),
        "key": request.key.clone(),
        "confirmed": request.confirmed,
        "headless": request.headless,
        "textChars": request.text.as_ref().map(|value| value.chars().count()),
    });
    let step = db
        .record_browser_agent_step(RecordBrowserAgentStepPayload {
            session_id: request.session_id.clone(),
            run_id: request.run_id.clone(),
            action: result.action.clone(),
            target: request.target.clone(),
            status: judge.status.clone(),
            title: result.title.clone(),
            url: result.url.clone(),
            screenshot_path: result.screenshot_path.clone(),
            dom_summary: serde_json::to_value(&dom_summary).unwrap_or_else(|_| json!({})),
            action_json,
            result_json: parsed.clone(),
            fingerprint: fingerprint.clone(),
            judge: serde_json::to_value(&judge).unwrap_or_else(|_| json!({})),
            loop_detected: loop_verdict.repeated,
        })
        .map_err(|error| error.to_string())?;
    result.step_id = Some(step.id.clone());
    result.step_index = Some(step.step_index);
    append_browser_audit(
        db,
        BrowserAutomationAuditEvent {
            id: format!("browser_{}", Uuid::new_v4()),
            action: result.action.clone(),
            target: request.target.clone(),
            confirmed: request.confirmed,
            status: judge.status.clone(),
            title: result.title.clone(),
            url: result.url.clone(),
            screenshot_path: result.screenshot_path.clone(),
            error: result.error.clone(),
            created_at: Utc::now().timestamp_millis(),
        },
    )?;
    if result.ok && !judge.blocks_completion {
        Ok(result)
    } else {
        Err(result
            .error
            .clone()
            .or_else(|| Some(judge.reason.clone()))
            .unwrap_or_else(|| "浏览器自动化失败。".to_string()))
    }
}

pub fn list_browser_audit_events(
    db: &LocalDb,
    limit: usize,
) -> Result<Vec<BrowserAutomationAuditEvent>, String> {
    let stored = db
        .get_app_state(BROWSER_AUDIT_KEY)
        .map_err(|error| error.to_string())?;
    let mut events: Vec<BrowserAutomationAuditEvent> = match stored {
        Some(value) => serde_json::from_value(value).map_err(|error| error.to_string())?,
        None => Vec::new(),
    };
    events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
    events.truncate(limit.min(BROWSER_AUDIT_LIMIT));
    Ok(events)
}

fn append_browser_audit(db: &LocalDb, event: BrowserAutomationAuditEvent) -> Result<(), String> {
    let mut events = list_browser_audit_events(db, BROWSER_AUDIT_LIMIT)?;
    events.push(event);
    events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
    events.truncate(BROWSER_AUDIT_LIMIT);
    db.set_app_state(BROWSER_AUDIT_KEY, json!(events))
        .map_err(|error| error.to_string())
}

fn validate_browser_request(request: &BrowserAutomationRequest) -> Result<(), String> {
    let action = request.action.trim();
    if !matches!(
        action,
        "search" | "open" | "screenshot" | "click" | "type" | "press"
    ) {
        return Err("浏览器动作只支持 search/open/screenshot/click/type/press。".to_string());
    }
    if matches!(action, "click" | "type" | "press") && !request.confirmed {
        return Err("点击、输入、按键必须先由用户确认。".to_string());
    }
    if matches!(action, "click" | "type")
        && request.selector.as_deref().unwrap_or("").trim().is_empty()
    {
        return Err("点击或输入需要 CSS selector。".to_string());
    }
    if matches!(action, "open" | "screenshot")
        && request.url.as_deref().unwrap_or("").trim().is_empty()
    {
        return Err("打开或截图需要 URL。".to_string());
    }
    Ok(())
}

async fn prepare_browser_runtime() -> Result<(PathBuf, PathBuf), String> {
    let script = materialize_packaged_browser_script()?;
    let runtime_dir = browser_runtime_dir();
    ensure_node_available().await?;
    if !playwright_available(&runtime_dir).await {
        install_playwright_runtime(&runtime_dir).await?;
    }
    if !playwright_available(&runtime_dir).await {
        return Err(
            "浏览器自动化运行环境未准备完成：Node 可用，但 Playwright 依赖仍不可用。".to_string(),
        );
    }
    Ok((script, runtime_dir))
}

fn materialize_packaged_browser_script() -> Result<PathBuf, String> {
    let runtime_dir = browser_runtime_dir();
    fs::create_dir_all(runtime_dir.join("scripts"))
        .map_err(|error| format!("创建浏览器自动化脚本目录失败：{error}"))?;
    let package = runtime_dir.join("package.json");
    let current_package = fs::read_to_string(&package).unwrap_or_default();
    if current_package != BROWSER_RUNTIME_PACKAGE {
        fs::write(&package, BROWSER_RUNTIME_PACKAGE)
            .map_err(|error| format!("写入浏览器自动化 package.json 失败：{error}"))?;
    }
    let script = runtime_dir
        .join("scripts")
        .join("atlas-browser-automation.mjs");
    let current = fs::read_to_string(&script).unwrap_or_default();
    if current != BROWSER_SCRIPT {
        fs::write(&script, BROWSER_SCRIPT)
            .map_err(|error| format!("写入浏览器自动化脚本失败：{error}"))?;
    }
    Ok(script)
}

fn browser_runtime_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Atlas")
        .join("browser-runtime")
}

async fn ensure_node_available() -> Result<(), String> {
    let output = tokio::process::Command::new("node")
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| format!("浏览器自动化需要 Node.js：{error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err("浏览器自动化需要可用的 Node.js。".to_string())
    }
}

async fn playwright_available(runtime_dir: &PathBuf) -> bool {
    tokio::process::Command::new("node")
        .arg("-e")
        .arg("import('playwright').then(()=>process.exit(0)).catch(()=>process.exit(1))")
        .current_dir(runtime_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|status| status.success())
        .unwrap_or(false)
}

async fn install_playwright_runtime(runtime_dir: &PathBuf) -> Result<(), String> {
    let output = tokio::process::Command::new(npm_command())
        .arg("install")
        .arg("--silent")
        .arg("--no-audit")
        .arg("--no-fund")
        .arg("--omit=dev")
        .current_dir(runtime_dir)
        .env("PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| format!("准备 Playwright 依赖失败：{error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "准备 Playwright 依赖失败：{}",
            stderr.trim().chars().take(500).collect::<String>()
        ))
    }
}

fn npm_command() -> &'static str {
    if cfg!(windows) {
        "npm.cmd"
    } else {
        "npm"
    }
}

fn base64_url_json(value: &Value) -> Result<String, String> {
    let raw = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw))
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
        LocalDb::open(std::env::temp_dir().join(format!("atlas_browser_{unique}.db"))).unwrap()
    }

    #[test]
    fn browser_write_like_actions_require_confirmation() {
        let request = BrowserAutomationRequest {
            action: "click".to_string(),
            session_id: None,
            run_id: None,
            target: None,
            keyword: None,
            url: None,
            selector: Some("button".to_string()),
            text: None,
            key: None,
            confirmed: false,
            headless: Some(true),
        };
        assert!(validate_browser_request(&request)
            .unwrap_err()
            .contains("确认"));
    }

    #[test]
    fn browser_audit_round_trips() {
        let db = temp_db();
        append_browser_audit(
            &db,
            BrowserAutomationAuditEvent {
                id: "browser-test".to_string(),
                action: "search".to_string(),
                target: Some("baidu".to_string()),
                confirmed: false,
                status: "ready".to_string(),
                title: Some("百度一下".to_string()),
                url: Some("https://www.baidu.com".to_string()),
                screenshot_path: None,
                error: None,
                created_at: Utc::now().timestamp_millis(),
            },
        )
        .unwrap();
        let events = list_browser_audit_events(&db, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].target.as_deref(), Some("baidu"));
    }

    #[test]
    fn packaged_browser_script_can_be_materialized() {
        let script = materialize_packaged_browser_script().unwrap();
        let content = std::fs::read_to_string(script).unwrap();
        assert!(content.contains("async function main"));
        assert!(content.contains("playwright"));
        let package = std::fs::read_to_string(browser_runtime_dir().join("package.json")).unwrap();
        assert!(package.contains("atlas-browser-runtime"));
        assert!(package.contains("playwright"));
    }

    #[tokio::test]
    #[ignore]
    async fn browser_automation_live_probe_uses_packaged_runtime() {
        let db = temp_db();
        let result = run_browser_automation(
            &db,
            BrowserAutomationRequest {
                session_id: None,
                run_id: None,
                action: "search".to_string(),
                target: Some("baidu".to_string()),
                keyword: Some("Atlas 自动化".to_string()),
                url: None,
                selector: None,
                text: None,
                key: None,
                confirmed: false,
                headless: Some(true),
            },
        )
        .await
        .unwrap();
        assert!(result.ok);
        assert!(result.url.unwrap_or_default().contains("baidu"));
        let screenshot = result.screenshot_path.unwrap();
        assert!(std::path::Path::new(&screenshot).exists());
        let events = list_browser_audit_events(&db, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, "ready");
    }
}
