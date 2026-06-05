//! Read-only git tools (M4.4 of millimeter plan).
//!
//! Each tool wraps a single git subcommand with a fixed read-only flag set.
//! Writing subcommands (`commit`, `push`, `reset`, `checkout`, `clean`, ...) are
//! intentionally not exposed — agents that need to mutate the working tree must
//! go through `run_command` with explicit user review.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::tools::output_limit::{truncate_middle, MAX_TOOL_OUTPUT_CHARS};
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};

/// P1-4: read-only git subcommands should be fast; a hang (huge repo, network
/// remote, lock contention) must not block the agent loop. Cap the wait, then
/// kill the process.
const GIT_READONLY_TIMEOUT_SECS: u64 = 60;

async fn run_git(args: &[&str], cwd: Option<&str>) -> Result<(String, String, i32), AgentError> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output =
        run_to_completion(cmd, Duration::from_secs(GIT_READONLY_TIMEOUT_SECS), "git").await?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((stdout, stderr, output.status.code().unwrap_or(-1)))
}

/// P1-4: run a prepared command to completion under a hard timeout. `kill_on_drop`
/// guarantees the child is killed whenever we stop awaiting it — on timeout (the
/// future is dropped here, killing the child) or if the surrounding tool future is
/// cancelled. On timeout we return a structured, readable error instead of hanging
/// the agent loop.
async fn run_to_completion(
    mut cmd: tokio::process::Command,
    timeout: Duration,
    label: &str,
) -> Result<std::process::Output, AgentError> {
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| AgentError::Tool(format!("{label} 启动失败: {e}")))?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(result) => result.map_err(|e| AgentError::Tool(format!("{label} 执行失败: {e}"))),
        Err(_) => Err(AgentError::Tool(format!(
            "{label} 超过 {} 秒未完成，已终止。",
            timeout.as_secs()
        ))),
    }
}

fn resolve_cwd(args: &Value) -> Option<String> {
    args.get("cwd")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn validate_pathspec(spec: &str) -> Result<(), AgentError> {
    if spec.starts_with('-') {
        return Err(AgentError::Tool(format!(
            "拒绝以 `-` 开头的 git 参数 `{spec}`，避免注入子命令选项。"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------

pub struct GitStatusTool;

#[async_trait]
impl Tool for GitStatusTool {
    fn name(&self) -> &str {
        "git_status"
    }

    fn description(&self) -> &str {
        "Show working tree status (porcelain v1 output)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Run `git status --porcelain=v1 --branch` in the given directory."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "cwd": {
                        "type": "string",
                        "description": "Working directory inside a git repo. Defaults to process cwd."
                    }
                }
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "git 状态".to_string(),
            description_zh: "查看工作区改动状态（porcelain v1）。".to_string(),
            capability_labels_zh: vec!["读取".to_string(), "Git".to_string()],
            safety_label_zh: "安全".to_string(),
            capabilities: vec![ToolCapability::Filesystem],
            safety_level: ToolSafetyLevel::Safe,
            mutates_state: false,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let cwd = resolve_cwd(&args);
        let (stdout, stderr, code) =
            run_git(&["status", "--porcelain=v1", "--branch"], cwd.as_deref()).await?;
        let out = truncate_middle(&stdout, MAX_TOOL_OUTPUT_CHARS);
        if code != 0 {
            return Err(AgentError::Tool(format!(
                "git status 失败（exit={code}）：{}",
                stderr.trim()
            )));
        }
        Ok(ToolResult::success(
            "git status 完成".to_string(),
            json!({
                "stdout": out.text,
                "stderr": stderr,
                "exitCode": code,
                "truncated": out.truncated,
                "truncation": out.meta(),
            }),
        ))
    }
}

// ---------------------------------------------------------------------------

pub struct GitDiffTool;

#[async_trait]
impl Tool for GitDiffTool {
    fn name(&self) -> &str {
        "git_diff"
    }

    fn description(&self) -> &str {
        "Show changes between commits, branches, or working tree (read-only)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Run `git diff [--staged] [refs...] [-- paths...]`. Read-only."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "staged": {
                        "type": "boolean",
                        "description": "If true, diff staged changes against HEAD."
                    },
                    "refs": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional commit/branch refs, e.g. [\"main..HEAD\"] or [\"abc123\", \"def456\"]."
                    },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional path filters."
                    },
                    "stat": {
                        "type": "boolean",
                        "description": "If true, return --stat summary instead of full diff."
                    },
                    "cwd": { "type": "string" }
                }
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "git diff".to_string(),
            description_zh: "查看变更内容（只读）。".to_string(),
            capability_labels_zh: vec!["读取".to_string(), "Git".to_string()],
            safety_label_zh: "安全".to_string(),
            capabilities: vec![ToolCapability::Filesystem],
            safety_level: ToolSafetyLevel::Safe,
            mutates_state: false,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let mut cli: Vec<String> = vec!["diff".to_string()];
        if args.get("staged").and_then(Value::as_bool).unwrap_or(false) {
            cli.push("--staged".to_string());
        }
        if args.get("stat").and_then(Value::as_bool).unwrap_or(false) {
            cli.push("--stat".to_string());
        }
        if let Some(refs) = args.get("refs").and_then(Value::as_array) {
            for r in refs {
                if let Some(s) = r.as_str() {
                    validate_pathspec(s)?;
                    cli.push(s.to_string());
                }
            }
        }
        let path_args = args.get("paths").and_then(Value::as_array);
        if let Some(paths) = path_args {
            cli.push("--".to_string());
            for p in paths {
                if let Some(s) = p.as_str() {
                    validate_pathspec(s)?;
                    cli.push(s.to_string());
                }
            }
        }

        let cwd = resolve_cwd(&args);
        let cli_refs: Vec<&str> = cli.iter().map(String::as_str).collect();
        let (stdout, stderr, code) = run_git(&cli_refs, cwd.as_deref()).await?;
        if code != 0 && !stderr.trim().is_empty() {
            return Err(AgentError::Tool(format!(
                "git diff 失败（exit={code}）：{}",
                stderr.trim()
            )));
        }
        let out = truncate_middle(&stdout, MAX_TOOL_OUTPUT_CHARS);
        Ok(ToolResult::success(
            "git diff 完成".to_string(),
            json!({
                "stdout": out.text,
                "stderr": stderr,
                "exitCode": code,
                "truncated": out.truncated,
                "truncation": out.meta(),
            }),
        ))
    }
}

// ---------------------------------------------------------------------------

pub struct GitLogTool;

#[async_trait]
impl Tool for GitLogTool {
    fn name(&self) -> &str {
        "git_log"
    }

    fn description(&self) -> &str {
        "Show commit history (read-only)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Run `git log --oneline -n <limit> [ref] [-- paths...]`. Read-only."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of commits to return (default 20, max 200)."
                    },
                    "ref": {
                        "type": "string",
                        "description": "Optional ref (branch / commit / tag)."
                    },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "cwd": { "type": "string" }
                }
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "git log".to_string(),
            description_zh: "查看提交历史（只读）。".to_string(),
            capability_labels_zh: vec!["读取".to_string(), "Git".to_string()],
            safety_label_zh: "安全".to_string(),
            capabilities: vec![ToolCapability::Filesystem],
            safety_level: ToolSafetyLevel::Safe,
            mutates_state: false,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .min(200) as usize;
        let mut cli: Vec<String> = vec![
            "log".to_string(),
            "--oneline".to_string(),
            format!("-n{limit}"),
        ];
        if let Some(reference) = args.get("ref").and_then(Value::as_str) {
            validate_pathspec(reference)?;
            cli.push(reference.to_string());
        }
        if let Some(paths) = args.get("paths").and_then(Value::as_array) {
            cli.push("--".to_string());
            for p in paths {
                if let Some(s) = p.as_str() {
                    validate_pathspec(s)?;
                    cli.push(s.to_string());
                }
            }
        }
        let cwd = resolve_cwd(&args);
        let cli_refs: Vec<&str> = cli.iter().map(String::as_str).collect();
        let (stdout, stderr, code) = run_git(&cli_refs, cwd.as_deref()).await?;
        if code != 0 {
            return Err(AgentError::Tool(format!(
                "git log 失败（exit={code}）：{}",
                stderr.trim()
            )));
        }
        let out = truncate_middle(&stdout, MAX_TOOL_OUTPUT_CHARS);
        Ok(ToolResult::success(
            format!("git log 完成（最多 {limit} 条）"),
            json!({
                "stdout": out.text,
                "stderr": stderr,
                "exitCode": code,
                "truncated": out.truncated,
                "truncation": out.meta(),
            }),
        ))
    }
}

// ---------------------------------------------------------------------------

pub struct GitShowTool;

#[async_trait]
impl Tool for GitShowTool {
    fn name(&self) -> &str {
        "git_show"
    }

    fn description(&self) -> &str {
        "Show the contents of a specific commit (read-only)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Run `git show <ref>` to inspect a single commit. Read-only.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ref": {
                        "type": "string",
                        "description": "Commit / tag / branch ref to show."
                    },
                    "stat": {
                        "type": "boolean",
                        "description": "If true, only show --stat summary."
                    },
                    "cwd": { "type": "string" }
                },
                "required": ["ref"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "git show".to_string(),
            description_zh: "查看指定提交（只读）。".to_string(),
            capability_labels_zh: vec!["读取".to_string(), "Git".to_string()],
            safety_label_zh: "安全".to_string(),
            capabilities: vec![ToolCapability::Filesystem],
            safety_level: ToolSafetyLevel::Safe,
            mutates_state: false,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let reference = args
            .get("ref")
            .and_then(Value::as_str)
            .ok_or_else(|| AgentError::Tool("缺少 ref 参数。".to_string()))?;
        validate_pathspec(reference)?;
        let mut cli: Vec<String> = vec!["show".to_string()];
        if args.get("stat").and_then(Value::as_bool).unwrap_or(false) {
            cli.push("--stat".to_string());
        }
        cli.push(reference.to_string());
        let cwd = resolve_cwd(&args);
        let cli_refs: Vec<&str> = cli.iter().map(String::as_str).collect();
        let (stdout, stderr, code) = run_git(&cli_refs, cwd.as_deref()).await?;
        if code != 0 {
            return Err(AgentError::Tool(format!(
                "git show 失败（exit={code}）：{}",
                stderr.trim()
            )));
        }
        let out = truncate_middle(&stdout, MAX_TOOL_OUTPUT_CHARS);
        Ok(ToolResult::success(
            format!("git show {reference} 完成"),
            json!({
                "stdout": out.text,
                "stderr": stderr,
                "exitCode": code,
                "truncated": out.truncated,
                "truncation": out.meta(),
                "ref": reference,
            }),
        ))
    }
}

// Suppress unused import warning when feature flags exclude tests.
#[allow(dead_code)]
fn _force_pathbuf_link(_: PathBuf) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_pathspec_rejects_dash_prefix() {
        assert!(validate_pathspec("--upload-pack").is_err());
        assert!(validate_pathspec("-rf").is_err());
        assert!(validate_pathspec("main..HEAD").is_ok());
        assert!(validate_pathspec("src/foo.rs").is_ok());
    }

    fn slow_command() -> tokio::process::Command {
        if cfg!(windows) {
            let mut c = tokio::process::Command::new("powershell");
            c.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
            c
        } else {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", "sleep 30"]);
            c
        }
    }

    #[tokio::test]
    async fn run_to_completion_times_out_and_kills_slow_process() {
        // A 30s sleep under a 300ms timeout must error out promptly — the process is
        // killed (kill_on_drop), the caller is not blocked for the full sleep.
        let start = std::time::Instant::now();
        let result = run_to_completion(slow_command(), Duration::from_millis(300), "test").await;
        assert!(result.is_err(), "slow process must hit the timeout");
        assert!(
            result.unwrap_err().to_string().contains("未完成"),
            "timeout must surface a readable error"
        );
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "must not wait for the full 30s sleep"
        );
    }

    #[tokio::test]
    async fn run_to_completion_returns_output_for_fast_command() {
        let cmd = if cfg!(windows) {
            let mut c = tokio::process::Command::new("powershell");
            c.args([
                "-NoProfile",
                "-Command",
                "Write-Output atlas-git-timeout-ok",
            ]);
            c
        } else {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", "printf atlas-git-timeout-ok"]);
            c
        };
        let output = run_to_completion(cmd, Duration::from_secs(10), "test")
            .await
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("atlas-git-timeout-ok"));
    }
}
