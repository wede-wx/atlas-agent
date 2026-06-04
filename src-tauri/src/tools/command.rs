use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::mpsc::Sender;
use tokio::time::timeout;

use crate::agent::hooks::{self, AgentHookContext, AgentHookKind};
use crate::agent::{AgentError, AgentEvent, ToolResult, ToolSchema};
use crate::storage::LocalDb;
use crate::tools::execution_isolation::{
    CommandIsolationPolicy, CommandIsolationReport, ExecutionIsolationConfig,
};
use crate::tools::{Tool, ToolCapability, ToolExecutionContext, ToolMetadata, ToolSafetyLevel};

const MAX_COMMAND_OUTPUT_CHARS: usize = 12_000;
const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 120;
/// P1-4: bounds for the caller-overridable `timeout_ms`. Floored so a tiny value
/// can't make every command "time out" instantly; capped so a runaway value can't
/// effectively disable the timeout.
const MIN_COMMAND_TIMEOUT_MS: u64 = 1_000;
const MAX_COMMAND_TIMEOUT_MS: u64 = 600_000;
/// When live stream output has no newline yet, flush the buffered prefix to the
/// UI once it grows past this many bytes so streaming does not stall.
const STREAM_EMIT_FLUSH_BYTES: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingCommand {
    pub id: String,
    pub command: String,
    pub cwd: String,
    pub reason: String,
    pub shell: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecutionResult {
    pub command: String,
    pub cwd: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub isolation: CommandIsolationReport,
}

pub struct PrepareCommandTool {
    db: LocalDb,
    isolation: CommandIsolationPolicy,
}

pub struct RunCommandTool {
    isolation: CommandIsolationPolicy,
}

impl PrepareCommandTool {
    pub fn new(db: LocalDb) -> Self {
        Self::new_with_isolation(db, CommandIsolationPolicy::default_for_current_dir())
    }

    pub fn new_with_isolation(db: LocalDb, isolation: CommandIsolationPolicy) -> Self {
        Self { db, isolation }
    }
}

impl RunCommandTool {
    pub fn new(isolation: CommandIsolationPolicy) -> Self {
        Self { isolation }
    }

    pub fn default_isolated() -> Self {
        Self::new(CommandIsolationPolicy::default_for_current_dir())
    }
}

#[async_trait]
impl Tool for PrepareCommandTool {
    fn name(&self) -> &str {
        "prepare_command"
    }

    fn description(&self) -> &str {
        "Prepare a local shell command for user confirmation. It never runs the command."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Prepare a command preview for confirmation mode. Do not use for destructive or system-level commands.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The exact shell command to show to the user"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Short reason shown to the user"
                    }
                },
                "required": ["command"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "准备运行命令".to_string(),
            description_zh: "生成命令预览，必须用户确认后才会运行。".to_string(),
            capability_labels_zh: vec!["系统命令".to_string(), "需确认".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::System],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: false,
            requires_confirmation: true,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| AgentError::Tool("缺少 command 参数。".to_string()))?;
        ensure_command_allowed(command)?;
        let (cwd, _isolation) = self
            .isolation
            .resolve_cwd(args.get("cwd").and_then(|value| value.as_str()))?;
        let reason = args
            .get("reason")
            .and_then(|value| value.as_str())
            .unwrap_or("Agent 准备运行本地命令。");
        let pending = self
            .db
            .prepare_command(
                command.to_string(),
                cwd.to_string_lossy().to_string(),
                reason.to_string(),
                shell_label().to_string(),
            )
            .map_err(|error| AgentError::Tool(error.to_string()))?;

        Ok(ToolResult::warning(
            "命令预览已准备，尚未运行。请在确认卡片里确认或拒绝。",
            serde_json::json!({
                "pendingCommand": PendingCommand {
                    id: pending.id,
                    command: pending.command,
                    cwd: pending.cwd,
                    reason: pending.reason,
                    shell: pending.shell,
                },
                "confirmed": false,
                "playbackState": "not_applicable"
            }),
            vec![
                "向用户说明将要运行的命令和工作目录。".to_string(),
                "提醒用户必须点击确认后才会运行。".to_string(),
            ],
        ))
    }
}

#[async_trait]
impl Tool for RunCommandTool {
    fn name(&self) -> &str {
        "run_command"
    }

    fn description(&self) -> &str {
        "Run a local shell command immediately in full access mode. Use only for commands the user clearly requested."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Run a local shell command immediately. Destructive or system-level commands are blocked by Aura safety checks.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The exact shell command to run"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory. Must stay inside Aura's configured command workspace boundary."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "Optional timeout in milliseconds (default 120000, min 1000, max 600000). On timeout the process is killed."
                    }
                },
                "required": ["command"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "运行命令".to_string(),
            description_zh: "在完全访问权限下直接运行用户明确要求的本地命令。".to_string(),
            capability_labels_zh: vec!["系统命令".to_string(), "直接执行".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::System],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        self.execute_with_context(
            args,
            ToolExecutionContext {
                operation_id: "run_command".to_string(),
                event_tx: None,
            },
        )
        .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<ToolResult, AgentError> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| AgentError::Tool("缺少 command 参数。".to_string()))?;
        let cwd = args.get("cwd").and_then(|value| value.as_str());
        let timeout_duration = resolve_command_timeout(&args);
        let operation_id = context.operation_id.clone();
        let mut result = execute_shell_command_streaming_with_policy(
            command,
            cwd,
            timeout_duration,
            context.event_tx,
            context.operation_id,
            &self.isolation,
        )
        .await?;

        // P0-1: redact secrets in command output at the source, so the masked form
        // is what flows into the tool result, the model context, and persistence.
        result.stdout = mask_command_output(&result.stdout);
        result.stderr = mask_command_output(&result.stderr);
        result.command = mask_command_output(&result.command);

        // Patch 17 / #18: AfterCommand hook dispatch (fire and forget).
        // We don't block command execution on hook results — the command already ran.
        // Block-mode hooks here are surfaced via warning in the tool result if any fired.
        let hook_runs = {
            let ctx = AgentHookContext::new(AgentHookKind::AfterCommand, operation_id)
                .with_extra("command", command)
                .with_extra("exit_code", format!("{:?}", result.exit_code))
                .with_extra("timed_out", result.timed_out.to_string());
            hooks::dispatch_global(&ctx, Some(command)).await
        };
        let blocked_by_hook = hook_runs.iter().any(|r| r.outcome.is_block());
        let hook_count = hook_runs.len();

        let data = serde_json::json!({
            "commandResult": result,
            "confirmed": true,
            "playbackState": "not_applicable",
            "hookRuns": hook_runs,
        });
        if result.timed_out {
            return Ok(ToolResult::warning(
                "命令超时，已停止等待输出。",
                data,
                vec!["向用户说明命令可能仍需手动检查。".to_string()],
            ));
        }
        if result.exit_code == Some(0) {
            if blocked_by_hook {
                Ok(ToolResult::warning(
                    format!("命令运行完成，但有 {hook_count} 个 hook 报告 block。"),
                    data,
                    vec!["检查 hookRuns，决定是否回滚或继续。".to_string()],
                ))
            } else {
                Ok(ToolResult::success("命令运行完成。", data))
            }
        } else {
            Ok(ToolResult::warning(
                format!("命令退出码：{}", result.exit_code.unwrap_or(-1)),
                data,
                vec!["查看 stderr/stdout，向用户说明失败原因和下一步。".to_string()],
            ))
        }
    }
}

pub async fn execute_shell_command(
    command: &str,
    cwd: Option<&str>,
    timeout_duration: Duration,
) -> Result<CommandExecutionResult, AgentError> {
    execute_shell_command_streaming(command, cwd, timeout_duration, None, "command".to_string())
        .await
}

pub async fn execute_shell_command_streaming(
    command: &str,
    cwd: Option<&str>,
    timeout_duration: Duration,
    event_tx: Option<Sender<AgentEvent>>,
    operation_id: String,
) -> Result<CommandExecutionResult, AgentError> {
    let isolation = CommandIsolationPolicy::from_config(&ExecutionIsolationConfig::default(), &[]);
    execute_shell_command_streaming_with_policy(
        command,
        cwd,
        timeout_duration,
        event_tx,
        operation_id,
        &isolation,
    )
    .await
}

pub async fn execute_shell_command_streaming_with_policy(
    command: &str,
    cwd: Option<&str>,
    timeout_duration: Duration,
    event_tx: Option<Sender<AgentEvent>>,
    operation_id: String,
    isolation: &CommandIsolationPolicy,
) -> Result<CommandExecutionResult, AgentError> {
    ensure_command_allowed(command)?;
    let (cwd, base_isolation_report) = isolation.resolve_cwd(cwd)?;
    let mut child = shell_command(command);
    child.current_dir(&cwd);
    child.stdout(Stdio::piped());
    child.stderr(Stdio::piped());
    let (injected_env, blocked_sensitive_env_count) = isolation.apply_env(&mut child);
    let isolation_report = CommandIsolationReport {
        injected_env,
        blocked_sensitive_env_count,
        ..base_isolation_report
    };
    // P1-4: if this future is dropped before the command finishes — the run is
    // cancelled, or the agent loop moves on — the child must be killed, not left
    // running detached. The timeout branch below also kills explicitly; this covers
    // the cancellation path the explicit kill can't reach.
    child.kill_on_drop(true);

    let mut child = child
        .spawn()
        .map_err(|error| AgentError::Tool(format!("命令启动失败: {error}")))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_handle = stdout.map(|reader| {
        tokio::spawn(read_command_stream(
            reader,
            "stdout",
            event_tx.clone(),
            operation_id.clone(),
        ))
    });
    let stderr_handle = stderr.map(|reader| {
        tokio::spawn(read_command_stream(
            reader,
            "stderr",
            event_tx.clone(),
            operation_id.clone(),
        ))
    });

    let (exit_code, timed_out) = match timeout(timeout_duration, child.wait()).await {
        Ok(result) => {
            let status =
                result.map_err(|error| AgentError::Tool(format!("命令等待失败: {error}")))?;
            (status.code(), false)
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let stdout = collect_stream_output(stdout_handle).await;
            let stderr = collect_stream_output(stderr_handle).await;
            let timeout_message = format!("命令超过 {} 秒未完成。", timeout_duration.as_secs());
            let stderr = if stderr.trim().is_empty() {
                timeout_message
            } else {
                format!("{stderr}\n{timeout_message}")
            };
            return Ok(CommandExecutionResult {
                command: mask_command_output(command),
                cwd: cwd.to_string_lossy().to_string(),
                exit_code: None,
                stdout,
                stderr,
                timed_out: true,
                isolation: isolation_report,
            });
        }
    };
    let stdout = collect_stream_output(stdout_handle).await;
    let stderr = collect_stream_output(stderr_handle).await;

    Ok(CommandExecutionResult {
        command: mask_command_output(command),
        cwd: cwd.to_string_lossy().to_string(),
        exit_code,
        stdout,
        stderr,
        timed_out,
        isolation: isolation_report,
    })
}

async fn read_command_stream<R>(
    mut reader: R,
    stream: &'static str,
    event_tx: Option<Sender<AgentEvent>>,
    operation_id: String,
) -> String
where
    R: AsyncRead + Unpin,
{
    let mut captured = String::new();
    // Carry buffer for the live UI stream so secrets are masked at line
    // granularity before they reach the frontend (P0-1). The full `captured`
    // output is masked again downstream in `execute_with_context`.
    let mut pending = String::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(read) => {
                let chunk = String::from_utf8_lossy(&buffer[..read]).to_string();
                captured.push_str(&chunk);
                if let Some(tx) = &event_tx {
                    pending.push_str(&chunk);
                    if let Some(emit) = drain_maskable_prefix(&mut pending) {
                        let _ = tx
                            .send(AgentEvent::OperationOutput {
                                operation_id: operation_id.clone(),
                                stream: stream.to_string(),
                                content: emit,
                            })
                            .await;
                    }
                }
                if captured.chars().count() > MAX_COMMAND_OUTPUT_CHARS * 2 {
                    captured = clip_output(&captured);
                }
            }
            Err(error) => {
                let text = format!("\n[{stream} 读取失败: {error}]");
                captured.push_str(&text);
                if event_tx.is_some() {
                    pending.push_str(&text);
                }
                break;
            }
        }
    }
    // Flush the buffered tail (masked) so nothing is dropped from the live view.
    if let Some(tx) = &event_tx {
        if !pending.is_empty() {
            let _ = tx
                .send(AgentEvent::OperationOutput {
                    operation_id: operation_id.clone(),
                    stream: stream.to_string(),
                    content: mask_command_output(&pending),
                })
                .await;
        }
    }
    clip_output(&captured)
}

/// Mask any secrets in command output destined for a trust boundary.
fn mask_command_output(text: &str) -> String {
    crate::tools::secret_scan::scan(
        text,
        crate::tools::secret_scan::SecretLocation::CommandOutput,
        crate::tools::secret_scan::SecretAction::Masked,
    )
    .text
}

/// Pull the part of `pending` safe to emit now — everything up to and including
/// the last newline — masked. If no newline has arrived but the buffer is large,
/// flush it whole so the stream does not stall. Returns `None` when only a short
/// partial line is buffered (keep waiting for the rest of the line).
fn drain_maskable_prefix(pending: &mut String) -> Option<String> {
    if let Some(idx) = pending.rfind('\n') {
        let head: String = pending.drain(..=idx).collect();
        Some(mask_command_output(&head))
    } else if pending.len() > STREAM_EMIT_FLUSH_BYTES {
        let head = std::mem::take(pending);
        Some(mask_command_output(&head))
    } else {
        None
    }
}

async fn collect_stream_output(handle: Option<tokio::task::JoinHandle<String>>) -> String {
    match handle {
        Some(handle) => handle.await.unwrap_or_else(|_| String::new()),
        None => String::new(),
    }
}

fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut child = Command::new("powershell");
        child.args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            command,
        ]);
        child
    }
    #[cfg(not(windows))]
    {
        let mut child = Command::new("sh");
        child.args(["-lc", command]);
        child
    }
}

fn shell_label() -> &'static str {
    #[cfg(windows)]
    {
        "powershell"
    }
    #[cfg(not(windows))]
    {
        "sh"
    }
}

/// P1-4: resolve the effective command timeout. Caller may override the default
/// via `timeout_ms`, clamped to a safe band so the value can neither fire
/// instantly nor disable the timeout entirely.
fn resolve_command_timeout(args: &serde_json::Value) -> Duration {
    args.get("timeout_ms")
        .and_then(serde_json::Value::as_u64)
        .map(|ms| Duration::from_millis(ms.clamp(MIN_COMMAND_TIMEOUT_MS, MAX_COMMAND_TIMEOUT_MS)))
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_COMMAND_TIMEOUT_SECS))
}

fn ensure_command_allowed(command: &str) -> Result<(), AgentError> {
    match crate::tools::command_safety::classify_command(command) {
        crate::tools::command_safety::CommandSafety::Denied { reason } => {
            Err(AgentError::Tool(reason))
        }
        _ => Ok(()),
    }
}

fn clip_output(value: &str) -> String {
    // P1-5: head + tail via the shared limiter, so a long command's start AND its
    // tail — where errors and exit summaries usually land — both survive
    // truncation instead of keeping only the head.
    crate::tools::output_limit::truncate_middle(value, MAX_COMMAND_OUTPUT_CHARS).text
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn dangerous_commands_are_blocked() {
        assert!(ensure_command_allowed("format C:").is_err());
        assert!(ensure_command_allowed("npm install -g @anthropic-ai/claude-code").is_ok());
    }

    #[tokio::test]
    async fn shell_command_runs_simple_echo() {
        let command = if cfg!(windows) {
            "Write-Output aura-command-test"
        } else {
            "printf aura-command-test"
        };
        let result = execute_shell_command(command, None, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("aura-command-test"));
    }

    #[tokio::test]
    async fn shell_command_streams_output_events() {
        let command = if cfg!(windows) {
            "Write-Output aura-stream-test"
        } else {
            "printf aura-stream-test"
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let result = execute_shell_command_streaming(
            command,
            None,
            Duration::from_secs(10),
            Some(tx),
            "op-stream-test".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("aura-stream-test"));

        let mut saw_output_event = false;
        while let Some(event) = rx.recv().await {
            if let AgentEvent::OperationOutput {
                operation_id,
                stream,
                content,
            } = event
            {
                if operation_id == "op-stream-test"
                    && stream == "stdout"
                    && content.contains("aura-stream-test")
                {
                    saw_output_event = true;
                }
            }
        }
        assert!(saw_output_event);
    }

    #[tokio::test]
    async fn streaming_output_is_masked_before_reaching_ui() {
        // A fake Anthropic-style key on its own line must be redacted in the live
        // OperationOutput events, not only in the final captured result.
        let secret = "sk-ant-AAAAAAAAAAAAAAAAAAAAAAAAA";
        let command = if cfg!(windows) {
            format!("Write-Output '{secret}'")
        } else {
            format!("printf '%s\\n' '{secret}'")
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let result = execute_shell_command_streaming(
            &command,
            None,
            Duration::from_secs(10),
            Some(tx),
            "op-mask-test".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, Some(0));

        let mut streamed = String::new();
        while let Some(event) = rx.recv().await {
            if let AgentEvent::OperationOutput { content, .. } = event {
                streamed.push_str(&content);
            }
        }
        assert!(
            !streamed.contains(secret),
            "raw secret leaked to live stream: {streamed:?}"
        );
        assert!(
            streamed.contains("[REDACTED:anthropic_api_key]"),
            "expected redaction marker in live stream, got: {streamed:?}"
        );
    }

    #[test]
    fn resolve_command_timeout_defaults_and_clamps() {
        assert_eq!(
            resolve_command_timeout(&serde_json::json!({})),
            Duration::from_secs(DEFAULT_COMMAND_TIMEOUT_SECS)
        );
        assert_eq!(
            resolve_command_timeout(&serde_json::json!({ "timeout_ms": 5000 })),
            Duration::from_millis(5000)
        );
        assert_eq!(
            resolve_command_timeout(&serde_json::json!({ "timeout_ms": 1 })),
            Duration::from_millis(MIN_COMMAND_TIMEOUT_MS)
        );
        assert_eq!(
            resolve_command_timeout(&serde_json::json!({ "timeout_ms": 9_999_999 })),
            Duration::from_millis(MAX_COMMAND_TIMEOUT_MS)
        );
    }

    #[test]
    fn command_cwd_outside_workspace_boundary_is_rejected() {
        let base = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("aura-command-boundary-{}", Uuid::new_v4()));
        let project = base.join("project");
        let outside = base.join("outside");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let policy = CommandIsolationPolicy::from_config(
            &ExecutionIsolationConfig::default(),
            std::slice::from_ref(&project),
        );

        let result = policy.resolve_cwd(Some(&outside.to_string_lossy()));

        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn command_env_uses_allowlist_and_blocks_sensitive_env() {
        let base = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("aura-command-env-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("AURA_COMMAND_ALLOWED_TEST", "visible-value");
        std::env::set_var("AURA_COMMAND_SECRET_TOKEN", "secret-value");
        let config = ExecutionIsolationConfig {
            command_workspace_boundary: true,
            allowed_command_roots: Vec::new(),
            command_env_allowlist: vec![
                "AURA_COMMAND_ALLOWED_TEST".to_string(),
                "AURA_COMMAND_SECRET_TOKEN".to_string(),
            ],
        };
        let policy = CommandIsolationPolicy::from_config(&config, std::slice::from_ref(&base));
        let command = if cfg!(windows) {
            "Write-Output ($env:AURA_COMMAND_ALLOWED_TEST + '|' + $env:AURA_COMMAND_SECRET_TOKEN)"
        } else {
            "printf '%s|%s\\n' \"$AURA_COMMAND_ALLOWED_TEST\" \"$AURA_COMMAND_SECRET_TOKEN\""
        };

        let result = execute_shell_command_streaming_with_policy(
            command,
            Some(&base.to_string_lossy()),
            Duration::from_secs(10),
            None,
            "op-env-test".to_string(),
            &policy,
        )
        .await
        .unwrap();

        std::env::remove_var("AURA_COMMAND_ALLOWED_TEST");
        std::env::remove_var("AURA_COMMAND_SECRET_TOKEN");
        let _ = std::fs::remove_dir_all(base);
        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("visible-value|"));
        assert!(
            !result.stdout.contains("secret-value"),
            "sensitive env value leaked into child output: {:?}",
            result.stdout
        );
        assert!(result
            .isolation
            .injected_env
            .iter()
            .any(|key| key == "AURA_COMMAND_ALLOWED_TEST"));
        assert!(!result
            .isolation
            .injected_env
            .iter()
            .any(|key| key == "AURA_COMMAND_SECRET_TOKEN"));
        assert!(result.isolation.blocked_sensitive_env_count >= 1);
    }

    #[tokio::test]
    async fn long_command_times_out_and_does_not_hang() {
        // A 30s sleep under a 400ms timeout must return promptly as timed_out — the
        // child is killed, the agent loop is not blocked for the full sleep.
        let command = if cfg!(windows) {
            "Start-Sleep -Seconds 30"
        } else {
            "sleep 30"
        };
        let start = std::time::Instant::now();
        let result = execute_shell_command(command, None, Duration::from_millis(400))
            .await
            .unwrap();
        assert!(result.timed_out, "long command must report timeout");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "timed-out command must be killed, not awaited for the full sleep"
        );
    }
}
