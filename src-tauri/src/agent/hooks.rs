//! Aura agent hooks runtime（Patch 17 / #18）。
//!
//! 用户可在 `~/.aura/hooks.toml` 配置 hook 命令，每条 hook 绑定一个事件 kind
//! + 可选 matcher（regex 字符串），命中后跑 shell 命令。
//!
//! 当前接通的事件：AfterCommand / BeforeTaskDone。其他 3 个 kind 仍保留枚举
//! 让现有 import 不挂；将来需要再接。
//!
//! 失败语义：
//!   - 命令退出码 0 → Pass
//!   - 非 0 + on_failure=block → HookOutcome::Block
//!   - 非 0 + on_failure=warn（默认） → HookOutcome::Warn
//!   - 超时（默认 10s）→ 按 on_failure 决定 Warn/Block
//!
//! BeforeTaskDone 的 Block 结果会把 plan_task 状态从 done 翻回 blocked。
//! AfterCommand 的 Block 当前只在事件总线显眼记录，不会回滚命令执行。

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentHookKind {
    AfterWrite,
    AfterCommand,
    AfterToolBatch,
    BeforeTaskDone,
    BeforeFinalResponse,
}

impl AgentHookKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AfterWrite => "after_write",
            Self::AfterCommand => "after_command",
            Self::AfterToolBatch => "after_tool_batch",
            Self::BeforeTaskDone => "before_task_done",
            Self::BeforeFinalResponse => "before_final_response",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHookContext {
    pub kind: AgentHookKind,
    pub run_id: String,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    /// 注入到子进程的额外环境变量（key 会被大写 + 前缀 AURA_HOOK_）。
    /// AfterCommand 时一般包含 command/exit_code/stdout_tail；
    /// BeforeTaskDone 时包含 task_title/evidence。
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

impl AgentHookContext {
    pub fn new(kind: AgentHookKind, run_id: impl Into<String>) -> Self {
        Self {
            kind,
            run_id: run_id.into(),
            session_id: None,
            task_id: None,
            extra: HashMap::new(),
        }
    }

    pub fn with_session(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }

    pub fn with_task(mut self, task_id: Option<String>) -> Self {
        self.task_id = task_id;
        self
    }

    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }
}

// ============================================================
// Config
// ============================================================

#[derive(Debug, Clone, Deserialize)]
pub struct HookConfig {
    pub event: AgentHookKind,
    /// 可选 regex；针对 dispatch 的 match_target 匹配。空 / "*" / 缺失 = 永远命中。
    #[serde(default)]
    pub matcher: Option<String>,
    pub command: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub on_failure: HookFailureBehavior,
}

fn default_timeout_ms() -> u64 {
    10_000
}

#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HookFailureBehavior {
    #[default]
    Warn,
    Block,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct HooksFile {
    #[serde(default)]
    hooks: Vec<HookConfig>,
}

pub struct HookRegistry {
    hooks: Vec<HookConfig>,
}

impl HookRegistry {
    pub fn empty() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn load_from_home() -> Self {
        let Some(path) = default_hooks_path() else {
            return Self::empty();
        };
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &PathBuf) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::empty();
        };
        match toml::from_str::<HooksFile>(&text) {
            Ok(file) => Self { hooks: file.hooks },
            Err(error) => {
                eprintln!("[hooks] failed to parse {path:?}: {error}");
                Self::empty()
            }
        }
    }

    pub fn matches<'a>(
        &'a self,
        kind: AgentHookKind,
        match_target: Option<&str>,
    ) -> Vec<&'a HookConfig> {
        self.hooks
            .iter()
            .filter(|h| h.event == kind)
            .filter(|h| match_target_passes(h.matcher.as_deref(), match_target))
            .collect()
    }

    pub fn all(&self) -> &[HookConfig] {
        &self.hooks
    }
}

fn match_target_passes(matcher: Option<&str>, target: Option<&str>) -> bool {
    let Some(pat) = matcher else {
        return true;
    };
    if pat.is_empty() || pat == "*" {
        return true;
    }
    let Some(target) = target else {
        return false;
    };
    match Regex::new(pat) {
        Ok(re) => re.is_match(target),
        Err(error) => {
            eprintln!("[hooks] invalid regex {pat:?}: {error}");
            false
        }
    }
}

fn default_hooks_path() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)?;
    Some(home.join(".aura").join("hooks.toml"))
}

// ============================================================
// Outcome + runner
// ============================================================

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HookOutcome {
    Pass,
    Warn { reason: String },
    Block { reason: String },
}

impl HookOutcome {
    pub fn is_block(&self) -> bool {
        matches!(self, HookOutcome::Block { .. })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HookRun {
    pub event: String,
    pub command: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub outcome: HookOutcome,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub timed_out: bool,
}

const TAIL_LIMIT: usize = 800;

pub async fn dispatch(
    registry: &HookRegistry,
    ctx: &AgentHookContext,
    match_target: Option<&str>,
) -> Vec<HookRun> {
    let candidates: Vec<HookConfig> = registry
        .matches(ctx.kind, match_target)
        .into_iter()
        .cloned()
        .collect();
    let mut out = Vec::with_capacity(candidates.len());
    for hook in &candidates {
        out.push(run_one(hook, ctx).await);
    }
    out
}

async fn run_one(hook: &HookConfig, ctx: &AgentHookContext) -> HookRun {
    let started = std::time::Instant::now();
    let (program, arg_flag) = if cfg!(target_os = "windows") {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };

    let mut cmd = Command::new(program);
    cmd.arg(arg_flag)
        .arg(&hook.command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    // Hooks run with a deliberately minimal, fixed env: env_clear + the standard
    // allowlist + sensitive-key blocking (same security core as the command tool).
    // We intentionally do NOT extend the user-configured command env allowlist to
    // hooks — a hook is project-defined code and should see the smallest env that
    // works, not whatever extra vars commands were granted. env isolation here is a
    // security floor, not a passthrough.
    let _ = crate::tools::execution_isolation::CommandIsolationPolicy::default_for_current_dir()
        .apply_env(&mut cmd);
    cmd.env("AURA_HOOK_EVENT", hook.event.as_str())
        .env("AURA_HOOK_RUN_ID", &ctx.run_id);
    if let Some(sid) = &ctx.session_id {
        cmd.env("AURA_HOOK_SESSION_ID", sid);
    }
    if let Some(tid) = &ctx.task_id {
        cmd.env("AURA_HOOK_TASK_ID", tid);
    }
    for (k, v) in &ctx.extra {
        let env_key = format!("AURA_HOOK_{}", k.to_uppercase());
        if !crate::tools::execution_isolation::is_sensitive_env_key(&env_key) {
            cmd.env(env_key, mask_hook_output(v));
        }
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(error) => {
            return HookRun {
                event: hook.event.as_str().to_string(),
                command: mask_hook_output(&hook.command),
                exit_code: None,
                duration_ms: 0,
                outcome: HookOutcome::Warn {
                    reason: format!("spawn failed: {error}"),
                },
                stdout_tail: String::new(),
                stderr_tail: String::new(),
                timed_out: false,
            };
        }
    };

    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();
    let timeout_dur = Duration::from_millis(hook.timeout_ms);
    let result = timeout(timeout_dur, child.wait()).await;

    let mut stdout_buf = String::new();
    let mut stderr_buf = String::new();
    if let Some(mut h) = stdout_handle {
        let _ = h.read_to_string(&mut stdout_buf).await;
    }
    if let Some(mut h) = stderr_handle {
        let _ = h.read_to_string(&mut stderr_buf).await;
    }
    let stdout_tail = mask_hook_output(&tail(&stdout_buf, TAIL_LIMIT));
    let stderr_tail = mask_hook_output(&tail(&stderr_buf, TAIL_LIMIT));
    let duration_ms = started.elapsed().as_millis() as u64;

    let (exit_code, timed_out, outcome) = match result {
        Ok(Ok(status)) => {
            let code = status.code();
            if status.success() {
                (code, false, HookOutcome::Pass)
            } else {
                let head = stderr_tail.lines().next().unwrap_or("").to_string();
                let reason = if head.is_empty() {
                    format!("hook exited with {code:?}")
                } else {
                    format!("hook exited with {code:?}: {head}")
                };
                let outcome = match hook.on_failure {
                    HookFailureBehavior::Block => HookOutcome::Block { reason },
                    HookFailureBehavior::Warn => HookOutcome::Warn { reason },
                };
                (code, false, outcome)
            }
        }
        Ok(Err(error)) => (
            None,
            false,
            HookOutcome::Warn {
                reason: format!("wait failed: {error}"),
            },
        ),
        Err(_) => {
            let _ = child.kill().await;
            let reason = format!("hook timed out after {}ms", hook.timeout_ms);
            let outcome = match hook.on_failure {
                HookFailureBehavior::Block => HookOutcome::Block { reason },
                HookFailureBehavior::Warn => HookOutcome::Warn { reason },
            };
            (None, true, outcome)
        }
    };

    HookRun {
        event: hook.event.as_str().to_string(),
        command: mask_hook_output(&hook.command),
        exit_code,
        duration_ms,
        outcome,
        stdout_tail,
        stderr_tail,
        timed_out,
    }
}

fn tail(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        text.to_string()
    } else {
        text[text.len() - limit..].to_string()
    }
}

fn mask_hook_output(text: &str) -> String {
    crate::tools::secret_scan::scan(
        text,
        crate::tools::secret_scan::SecretLocation::Log,
        crate::tools::secret_scan::SecretAction::Masked,
    )
    .text
}

// ============================================================
// Process-wide shared registry
// ============================================================

static SHARED: OnceLock<Mutex<HookRegistry>> = OnceLock::new();

fn shared() -> &'static Mutex<HookRegistry> {
    SHARED.get_or_init(|| Mutex::new(HookRegistry::empty()))
}

pub fn install_global(registry: HookRegistry) {
    if let Ok(mut g) = shared().lock() {
        *g = registry;
    }
}

pub async fn dispatch_global(ctx: &AgentHookContext, match_target: Option<&str>) -> Vec<HookRun> {
    let candidates: Vec<HookConfig> = match shared().lock() {
        Ok(g) => g
            .matches(ctx.kind, match_target)
            .into_iter()
            .cloned()
            .collect(),
        Err(_) => return Vec::new(),
    };
    if candidates.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(candidates.len());
    for hook in &candidates {
        out.push(run_one(hook, ctx).await);
    }
    out
}

pub fn reload_global_from_home() {
    install_global(HookRegistry::load_from_home());
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_toml(s: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(s.as_bytes()).unwrap();
        f
    }

    #[test]
    fn load_empty_when_file_missing() {
        let reg = HookRegistry::load_from_path(&PathBuf::from(
            "C:/this/path/does/not/exist/aura_hooks.toml",
        ));
        assert!(reg.all().is_empty());
    }

    #[test]
    fn load_parses_minimal_hook() {
        let f = write_toml(
            r#"
[[hooks]]
event = "after_command"
command = "echo done"
"#,
        );
        let reg = HookRegistry::load_from_path(&f.path().to_path_buf());
        assert_eq!(reg.all().len(), 1);
        assert_eq!(reg.all()[0].event, AgentHookKind::AfterCommand);
        assert_eq!(reg.all()[0].timeout_ms, 10_000);
        assert_eq!(reg.all()[0].on_failure, HookFailureBehavior::Warn);
    }

    #[test]
    fn matches_filters_by_event_kind() {
        let f = write_toml(
            r#"
[[hooks]]
event = "after_command"
command = "a"

[[hooks]]
event = "before_task_done"
command = "b"
"#,
        );
        let reg = HookRegistry::load_from_path(&f.path().to_path_buf());
        assert_eq!(reg.matches(AgentHookKind::AfterCommand, None).len(), 1);
        assert_eq!(reg.matches(AgentHookKind::BeforeTaskDone, None).len(), 1);
        assert_eq!(reg.matches(AgentHookKind::AfterWrite, None).len(), 0);
    }

    #[test]
    fn matcher_regex_filters_target() {
        let f = write_toml(
            r#"
[[hooks]]
event = "after_command"
matcher = "^cargo "
command = "echo cargo-only"

[[hooks]]
event = "after_command"
matcher = "*"
command = "echo always"
"#,
        );
        let reg = HookRegistry::load_from_path(&f.path().to_path_buf());
        let m1 = reg.matches(AgentHookKind::AfterCommand, Some("cargo test"));
        assert_eq!(m1.len(), 2, "cargo + *");
        let m2 = reg.matches(AgentHookKind::AfterCommand, Some("ls -la"));
        assert_eq!(m2.len(), 1, "only *");
    }

    #[tokio::test]
    async fn dispatch_runs_and_returns_pass() {
        let body = if cfg!(target_os = "windows") {
            r#"
[[hooks]]
event = "after_command"
command = "cmd /C exit 0"
"#
        } else {
            r#"
[[hooks]]
event = "after_command"
command = "exit 0"
"#
        };
        let f = write_toml(body);
        let reg = HookRegistry::load_from_path(&f.path().to_path_buf());
        let ctx = AgentHookContext::new(AgentHookKind::AfterCommand, "run-1");
        let runs = dispatch(&reg, &ctx, Some("anything")).await;
        assert_eq!(runs.len(), 1);
        assert!(matches!(runs[0].outcome, HookOutcome::Pass));
    }

    #[tokio::test]
    async fn dispatch_block_on_failure_yields_block() {
        let body = if cfg!(target_os = "windows") {
            r#"
[[hooks]]
event = "before_task_done"
command = "cmd /C exit 1"
on_failure = "block"
"#
        } else {
            r#"
[[hooks]]
event = "before_task_done"
command = "exit 1"
on_failure = "block"
"#
        };
        let f = write_toml(body);
        let reg = HookRegistry::load_from_path(&f.path().to_path_buf());
        let ctx = AgentHookContext::new(AgentHookKind::BeforeTaskDone, "run-1");
        let runs = dispatch(&reg, &ctx, Some("test task")).await;
        assert_eq!(runs.len(), 1);
        assert!(runs[0].outcome.is_block(), "block on_failure must block");
        assert_eq!(runs[0].exit_code, Some(1));
    }

    #[tokio::test]
    async fn dispatch_warn_by_default_on_failure() {
        let body = if cfg!(target_os = "windows") {
            r#"
[[hooks]]
event = "after_command"
command = "cmd /C exit 1"
"#
        } else {
            r#"
[[hooks]]
event = "after_command"
command = "exit 1"
"#
        };
        let f = write_toml(body);
        let reg = HookRegistry::load_from_path(&f.path().to_path_buf());
        let ctx = AgentHookContext::new(AgentHookKind::AfterCommand, "run-1");
        let runs = dispatch(&reg, &ctx, Some("x")).await;
        assert!(matches!(runs[0].outcome, HookOutcome::Warn { .. }));
        assert!(!runs[0].outcome.is_block());
    }

    #[tokio::test]
    async fn hook_output_is_secret_masked() {
        let secret = "sk-ant-AAAAAAAAAAAAAAAAAAAAAAAAA";
        let body = if cfg!(target_os = "windows") {
            format!(
                r#"
[[hooks]]
event = "after_command"
command = "powershell -NoProfile -Command Write-Output '{secret}'"
"#
            )
        } else {
            format!(
                r#"
[[hooks]]
event = "after_command"
command = "printf '%s\n' '{secret}'"
"#
            )
        };
        let f = write_toml(&body);
        let reg = HookRegistry::load_from_path(&f.path().to_path_buf());
        let ctx = AgentHookContext::new(AgentHookKind::AfterCommand, "run-1");

        let runs = dispatch(&reg, &ctx, Some("anything")).await;

        assert_eq!(runs.len(), 1);
        assert!(
            !runs[0].stdout_tail.contains(secret),
            "hook stdout leaked raw secret: {:?}",
            runs[0].stdout_tail
        );
        assert!(runs[0].stdout_tail.contains("[REDACTED:anthropic_api_key]"));
    }
}
