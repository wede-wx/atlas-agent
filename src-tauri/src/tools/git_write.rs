//! Controlled git write tools (P3-1).
//!
//! These tools expose the minimum delivery-oriented Git mutations Aura needs:
//! stage, commit, create branch, and push. They deliberately do not expose
//! reset, clean, rebase, checkout, or arbitrary git flags. Every mutation
//! requires an explicit `confirmed=true` argument, and `git_commit` performs a
//! staged diff + secret scan before it creates the commit.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::agent::{AgentError, ToolResult, ToolResultStatus, ToolSchema};
use crate::tools::fs_scope;
use crate::tools::output_limit::{truncate_middle, MAX_TOOL_OUTPUT_CHARS};
use crate::tools::secret_scan::{self, SecretAction, SecretFinding, SecretLocation};
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};

const GIT_WRITE_TIMEOUT_SECS: u64 = 120;
const MAX_COMMIT_DIFF_SCAN_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug)]
struct GitCommandOutput {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

async fn run_git(args: &[String], cwd: &Path) -> Result<GitCommandOutput, AgentError> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.args(args);
    cmd.current_dir(cwd);
    let output = run_to_completion(cmd, Duration::from_secs(GIT_WRITE_TIMEOUT_SECS), "git").await?;
    Ok(GitCommandOutput {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

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

fn s(args: &[&str]) -> Vec<String> {
    args.iter().map(|value| (*value).to_string()).collect()
}

fn resolve_cwd(args: &Value, extra_roots: &[PathBuf]) -> Result<PathBuf, AgentError> {
    if let Some(cwd) = args.get("cwd").and_then(Value::as_str) {
        return fs_scope::allowed_directory_with_roots(cwd, extra_roots);
    }
    let cwd = std::env::current_dir()
        .map_err(|error| AgentError::Tool(format!("无法读取当前目录: {error}")))?;
    fs_scope::allowed_directory_with_roots(&cwd.to_string_lossy(), extra_roots)
}

async fn repo_root(cwd: &Path) -> Result<String, AgentError> {
    let output = run_git(&s(&["rev-parse", "--show-toplevel"]), cwd).await?;
    if output.exit_code != 0 {
        return Err(AgentError::Tool(format!(
            "当前目录不是可用 Git 仓库（exit={}）：{}",
            output.exit_code,
            output.stderr.trim()
        )));
    }
    Ok(output.stdout.trim().to_string())
}

fn confirmed(args: &Value) -> bool {
    args.get("confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn required_str(args: &Value, key: &str) -> Result<String, AgentError> {
    let value = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AgentError::Tool(format!("缺少 {key} 参数。")))?;
    validate_text_operand(value, key)?;
    Ok(value.to_string())
}

fn required_commit_message(args: &Value) -> Result<String, AgentError> {
    let value = args
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| AgentError::Tool("缺少 message 参数。".to_string()))?;
    if value.trim().is_empty() {
        return Err(AgentError::Tool("message 不能为空。".to_string()));
    }
    if value.contains('\0') {
        return Err(AgentError::Tool(
            "message 不能包含 NUL 控制字符。".to_string(),
        ));
    }
    Ok(value.to_string())
}

fn optional_str(args: &Value, key: &str) -> Result<Option<String>, AgentError> {
    args.get(key)
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| AgentError::Tool(format!("{key} 必须是字符串。")))
                .and_then(|text| {
                    validate_text_operand(text, key)?;
                    Ok(text.to_string())
                })
        })
        .transpose()
}

fn parse_string_array(args: &Value, key: &str) -> Result<Vec<String>, AgentError> {
    let Some(values) = args.get(key) else {
        return Ok(Vec::new());
    };
    let array = values
        .as_array()
        .ok_or_else(|| AgentError::Tool(format!("{key} 必须是字符串数组。")))?;
    array
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| AgentError::Tool(format!("{key} 只能包含字符串。")))
                .and_then(|text| {
                    validate_pathspec(text)?;
                    Ok(text.to_string())
                })
        })
        .collect()
}

fn validate_text_operand(value: &str, label: &str) -> Result<(), AgentError> {
    if value.trim().is_empty() {
        return Err(AgentError::Tool(format!("{label} 不能为空。")));
    }
    if value.contains('\0') || value.contains('\r') || value.contains('\n') {
        return Err(AgentError::Tool(format!("{label} 不能包含控制字符。")));
    }
    if value.starts_with('-') {
        return Err(AgentError::Tool(format!(
            "拒绝以 `-` 开头的 {label} `{value}`，避免注入 Git 选项。"
        )));
    }
    Ok(())
}

fn validate_pathspec(spec: &str) -> Result<(), AgentError> {
    validate_text_operand(spec, "pathspec")?;
    Ok(())
}

fn validate_branch_name(branch: &str) -> Result<(), AgentError> {
    validate_text_operand(branch, "branch")?;
    if branch.contains(':') || branch.contains(' ') || branch.contains('\t') {
        return Err(AgentError::Tool(
            "分支名不能包含空白字符或 refspec 冒号。".to_string(),
        ));
    }
    Ok(())
}

fn validate_push_remote(remote: &str) -> Result<(), AgentError> {
    validate_text_operand(remote, "remote")?;
    if remote.contains(':') || remote.chars().any(char::is_whitespace) {
        return Err(AgentError::Tool(
            "git_push 只接受已配置的 remote 名称，不接受 URL/refspec。".to_string(),
        ));
    }
    Ok(())
}

fn validate_push_branch(branch: &str) -> Result<(), AgentError> {
    validate_text_operand(branch, "branch")?;
    if branch.contains(':') || branch.contains('*') || branch.chars().any(char::is_whitespace) {
        return Err(AgentError::Tool(
            "git_push 的 branch 不能包含 refspec、通配符或空白字符。".to_string(),
        ));
    }
    Ok(())
}

fn reject_push_danger_fields(args: &Value) -> Result<(), AgentError> {
    for key in [
        "force",
        "forceWithLease",
        "force_with_lease",
        "delete",
        "prune",
        "mirror",
        "tags",
        "flags",
        "args",
        "refspec",
    ] {
        if args.get(key).is_some() {
            return Err(AgentError::Tool(format!(
                "git_push 不接受 `{key}` 参数；force/delete/mirror/tags/refspec 类推送默认拒绝。"
            )));
        }
    }
    Ok(())
}

fn command_preview(args: &[String]) -> String {
    let mut out = String::from("git");
    for arg in args {
        out.push(' ');
        out.push_str(&quote_arg(arg));
    }
    out
}

fn quote_arg(arg: &str) -> String {
    if arg
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | '\\' | '='))
    {
        arg.to_string()
    } else {
        format!("\"{}\"", arg.replace('"', "\\\""))
    }
}

fn output_data(output: &GitCommandOutput) -> Value {
    let stdout = truncate_middle(&output.stdout, MAX_TOOL_OUTPUT_CHARS);
    let stderr = truncate_middle(&output.stderr, MAX_TOOL_OUTPUT_CHARS);
    json!({
        "stdout": stdout.text,
        "stderr": stderr.text,
        "exitCode": output.exit_code,
        "truncated": stdout.truncated || stderr.truncated,
        "truncation": {
            "stdout": stdout.meta(),
            "stderr": stderr.meta()
        }
    })
}

fn error_result(
    summary: impl Into<String>,
    data: Value,
    next_actions: Vec<String>,
    recoverable: bool,
) -> ToolResult {
    ToolResult {
        status: ToolResultStatus::Error,
        summary: summary.into(),
        data,
        next_actions,
        recoverable,
    }
}

fn git_failed_result(action: &str, output: GitCommandOutput, cwd: &Path) -> ToolResult {
    let mut data = output_data(&output);
    data["cwd"] = json!(cwd.to_string_lossy().to_string());
    error_result(
        format!("{action} 失败（exit={}）。", output.exit_code),
        data,
        vec!["查看 stderr，修正 Git 状态或参数后重试。".to_string()],
        true,
    )
}

fn confirmation_required_result(
    summary: impl Into<String>,
    command_args: &[String],
    cwd: &Path,
    root: &str,
    extra: Value,
) -> ToolResult {
    let mut data = json!({
        "confirmed": false,
        "playbackState": "requires_confirmation",
        "commandPreview": command_preview(command_args),
        "cwd": cwd.to_string_lossy().to_string(),
        "repoRoot": root
    });
    if let (Some(target), Some(source)) = (data.as_object_mut(), extra.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
    ToolResult::warning(
        summary,
        data,
        vec!["确认参数和影响范围后，用 confirmed=true 重新调用。".to_string()],
    )
}

fn sensitive_git_metadata(
    name: &str,
    description: &str,
    label_zh: &str,
    description_zh: &str,
    capabilities: Vec<ToolCapability>,
    capability_labels_zh: Vec<String>,
) -> ToolMetadata {
    ToolMetadata {
        name: name.to_string(),
        description: description.to_string(),
        label_zh: label_zh.to_string(),
        description_zh: description_zh.to_string(),
        capability_labels_zh,
        safety_label_zh: "敏感，需确认".to_string(),
        capabilities,
        safety_level: ToolSafetyLevel::Sensitive,
        mutates_state: true,
        requires_confirmation: true,
    }
}

async fn staged_names(cwd: &Path) -> Result<Vec<String>, AgentError> {
    let output = run_git(&s(&["diff", "--cached", "--name-only", "--"]), cwd).await?;
    if output.exit_code != 0 {
        return Err(AgentError::Tool(format!(
            "读取 staged 文件失败（exit={}）：{}",
            output.exit_code,
            output.stderr.trim()
        )));
    }
    Ok(output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

async fn status_porcelain(cwd: &Path) -> Result<String, AgentError> {
    let output = run_git(&s(&["status", "--porcelain=v1", "--branch"]), cwd).await?;
    if output.exit_code != 0 {
        return Err(AgentError::Tool(format!(
            "读取 git status 失败（exit={}）：{}",
            output.exit_code,
            output.stderr.trim()
        )));
    }
    Ok(output.stdout)
}

#[derive(Debug)]
struct CommitPreflight {
    staged_files: Vec<String>,
    staged_diff: String,
    findings: Vec<SecretFinding>,
}

async fn collect_commit_preflight(
    cwd: &Path,
    message: &str,
) -> Result<CommitPreflight, AgentError> {
    let staged_files = staged_names(cwd).await?;
    if staged_files.is_empty() {
        return Ok(CommitPreflight {
            staged_files,
            staged_diff: String::new(),
            findings: Vec::new(),
        });
    }

    let diff_output = run_git(
        &s(&["diff", "--cached", "--no-ext-diff", "--binary", "--"]),
        cwd,
    )
    .await?;
    if diff_output.exit_code != 0 {
        return Err(AgentError::Tool(format!(
            "生成 staged diff 失败（exit={}）：{}",
            diff_output.exit_code,
            diff_output.stderr.trim()
        )));
    }
    if diff_output.stdout.len() > MAX_COMMIT_DIFF_SCAN_BYTES {
        return Err(AgentError::Tool(format!(
            "staged diff 超过 {} 字节，commit 已阻断；请拆分提交或人工扫描后再提交。",
            MAX_COMMIT_DIFF_SCAN_BYTES
        )));
    }

    let mut findings = scan_with_reference(message, "commit_message");
    findings.extend(scan_with_reference(&diff_output.stdout, "staged_diff"));
    Ok(CommitPreflight {
        staged_files,
        staged_diff: diff_output.stdout,
        findings,
    })
}

fn scan_with_reference(input: &str, reference: &str) -> Vec<SecretFinding> {
    let mut report = secret_scan::scan(input, SecretLocation::Commit, SecretAction::Blocked);
    for finding in &mut report.findings {
        finding.reference = Some(reference.to_string());
    }
    report.findings
}

fn secret_block_result(preflight: CommitPreflight, cwd: &Path, root: &str) -> ToolResult {
    let diff_preview = truncate_middle(&preflight.staged_diff, MAX_TOOL_OUTPUT_CHARS);
    error_result(
        format!(
            "commit 前 secret 扫描发现 {} 处疑似密钥，已阻断提交。",
            preflight.findings.len()
        ),
        json!({
            "confirmed": true,
            "playbackState": "blocked",
            "cwd": cwd.to_string_lossy().to_string(),
            "repoRoot": root,
            "stagedFiles": preflight.staged_files,
            "secretFindings": preflight.findings,
            "stagedDiffPreview": diff_preview.text,
            "truncation": diff_preview.meta()
        }),
        vec![
            "从 staged diff 或 commit message 中移除密钥。".to_string(),
            "如果是误报，先把真实密钥替换为测试占位符，再重新 stage/commit。".to_string(),
        ],
        true,
    )
}

// ---------------------------------------------------------------------------

pub struct GitStageTool {
    extra_roots: Vec<PathBuf>,
}

impl GitStageTool {
    pub fn new_with_roots(extra_roots: Vec<PathBuf>) -> Self {
        Self {
            extra_roots: fs_scope::normalize_extra_roots(extra_roots),
        }
    }
}

impl Default for GitStageTool {
    fn default() -> Self {
        Self::new_with_roots(Vec::new())
    }
}

#[async_trait]
impl Tool for GitStageTool {
    fn name(&self) -> &str {
        "git_stage"
    }

    fn description(&self) -> &str {
        "Stage tracked or selected files with `git add`, after explicit confirmation."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description:
                "Stage files in a git repository. Requires confirmed=true before mutation."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "cwd": { "type": "string", "description": "Working directory inside a git repo." },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Specific pathspecs to stage. Cannot start with '-'."
                    },
                    "all": {
                        "type": "boolean",
                        "description": "If true, run git add -A for the repository."
                    },
                    "confirmed": {
                        "type": "boolean",
                        "description": "Must be true after user confirmation to mutate the index."
                    }
                },
                "required": ["confirmed"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        sensitive_git_metadata(
            self.name(),
            self.description(),
            "Git 暂存",
            "确认后把指定文件或全部改动加入 Git index。",
            vec![ToolCapability::Filesystem],
            vec!["Git".to_string(), "写入".to_string(), "需确认".to_string()],
        )
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let cwd = resolve_cwd(&args, &self.extra_roots)?;
        let root = repo_root(&cwd).await?;
        let paths = parse_string_array(&args, "paths")?;
        let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
        if all && !paths.is_empty() {
            return Err(AgentError::Tool(
                "git_stage 不能同时指定 all=true 和 paths。".to_string(),
            ));
        }
        if !all && paths.is_empty() {
            return Err(AgentError::Tool(
                "git_stage 需要 all=true 或至少一个 paths 条目。".to_string(),
            ));
        }

        let mut cli = vec!["add".to_string()];
        if all {
            cli.push("-A".to_string());
        } else {
            cli.push("--".to_string());
            cli.extend(paths.clone());
        }

        if !confirmed(&args) {
            return Ok(confirmation_required_result(
                "git_stage 已生成预览，尚未修改 Git index。",
                &cli,
                &cwd,
                &root,
                json!({ "all": all, "paths": paths }),
            ));
        }

        let output = run_git(&cli, &cwd).await?;
        if output.exit_code != 0 {
            return Ok(git_failed_result("git_stage", output, &cwd));
        }
        let staged = staged_names(&cwd).await?;
        let status = status_porcelain(&cwd).await?;
        let mut data = output_data(&output);
        data["confirmed"] = json!(true);
        data["playbackState"] = json!("executed");
        data["cwd"] = json!(cwd.to_string_lossy().to_string());
        data["repoRoot"] = json!(root);
        data["stagedFiles"] = json!(staged);
        data["statusPorcelain"] = json!(status);
        Ok(ToolResult::success("git_stage 已暂存改动。", data))
    }
}

// ---------------------------------------------------------------------------

pub struct GitCommitTool {
    extra_roots: Vec<PathBuf>,
}

impl GitCommitTool {
    pub fn new_with_roots(extra_roots: Vec<PathBuf>) -> Self {
        Self {
            extra_roots: fs_scope::normalize_extra_roots(extra_roots),
        }
    }
}

impl Default for GitCommitTool {
    fn default() -> Self {
        Self::new_with_roots(Vec::new())
    }
}

#[async_trait]
impl Tool for GitCommitTool {
    fn name(&self) -> &str {
        "git_commit"
    }

    fn description(&self) -> &str {
        "Create a git commit after staged diff and commit-message secret scanning."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Create a git commit. Requires confirmed=true and blocks if staged diff or message contains secrets.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "cwd": { "type": "string", "description": "Working directory inside a git repo." },
                    "message": { "type": "string", "description": "Commit message. Empty messages are rejected." },
                    "confirmed": {
                        "type": "boolean",
                        "description": "Must be true after user confirmation to create the commit."
                    }
                },
                "required": ["message", "confirmed"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        sensitive_git_metadata(
            self.name(),
            self.description(),
            "Git 提交",
            "确认后创建 commit，并在提交前扫描 staged diff 和 commit message。",
            vec![ToolCapability::Filesystem],
            vec![
                "Git".to_string(),
                "写入".to_string(),
                "密钥扫描".to_string(),
                "需确认".to_string(),
            ],
        )
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let cwd = resolve_cwd(&args, &self.extra_roots)?;
        let root = repo_root(&cwd).await?;
        let message = required_commit_message(&args)?;
        let preflight = collect_commit_preflight(&cwd, &message).await?;
        if preflight.staged_files.is_empty() {
            return Ok(error_result(
                "git_commit 已阻断：没有 staged 改动。",
                json!({
                    "confirmed": confirmed(&args),
                    "playbackState": "blocked",
                    "cwd": cwd.to_string_lossy().to_string(),
                    "repoRoot": root
                }),
                vec!["先调用 git_stage 暂存需要提交的文件。".to_string()],
                true,
            ));
        }
        if !preflight.findings.is_empty() {
            return Ok(secret_block_result(preflight, &cwd, &root));
        }

        let cli = vec!["commit".to_string(), "-m".to_string(), message];
        if !confirmed(&args) {
            let diff_preview = truncate_middle(&preflight.staged_diff, MAX_TOOL_OUTPUT_CHARS);
            return Ok(confirmation_required_result(
                "git_commit 已完成提交前预检，尚未创建 commit。",
                &cli,
                &cwd,
                &root,
                json!({
                    "stagedFiles": preflight.staged_files,
                    "secretFindings": [],
                    "stagedDiffPreview": diff_preview.text,
                    "truncation": diff_preview.meta()
                }),
            ));
        }

        let output = run_git(&cli, &cwd).await?;
        if output.exit_code != 0 {
            return Ok(git_failed_result("git_commit", output, &cwd));
        }
        let head = run_git(&s(&["rev-parse", "--short", "HEAD"]), &cwd).await?;
        let commit = if head.exit_code == 0 {
            head.stdout.trim().to_string()
        } else {
            String::new()
        };
        let mut data = output_data(&output);
        data["confirmed"] = json!(true);
        data["playbackState"] = json!("executed");
        data["cwd"] = json!(cwd.to_string_lossy().to_string());
        data["repoRoot"] = json!(root);
        data["commit"] = json!(commit);
        data["stagedFiles"] = json!(preflight.staged_files);
        data["secretFindings"] = json!([]);
        Ok(ToolResult::success("git_commit 已创建提交。", data))
    }
}

// ---------------------------------------------------------------------------

pub struct GitCreateBranchTool {
    extra_roots: Vec<PathBuf>,
}

impl GitCreateBranchTool {
    pub fn new_with_roots(extra_roots: Vec<PathBuf>) -> Self {
        Self {
            extra_roots: fs_scope::normalize_extra_roots(extra_roots),
        }
    }
}

impl Default for GitCreateBranchTool {
    fn default() -> Self {
        Self::new_with_roots(Vec::new())
    }
}

#[async_trait]
impl Tool for GitCreateBranchTool {
    fn name(&self) -> &str {
        "git_create_branch"
    }

    fn description(&self) -> &str {
        "Create a git branch without checking it out, after explicit confirmation."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Create a git branch with `git branch <branch> [startPoint]`. Does not checkout. Requires confirmed=true.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "cwd": { "type": "string", "description": "Working directory inside a git repo." },
                    "branch": { "type": "string", "description": "New branch name." },
                    "startPoint": { "type": "string", "description": "Optional ref to branch from." },
                    "confirmed": {
                        "type": "boolean",
                        "description": "Must be true after user confirmation to create the branch."
                    }
                },
                "required": ["branch", "confirmed"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        sensitive_git_metadata(
            self.name(),
            self.description(),
            "Git 建分支",
            "确认后创建新分支，不自动 checkout。",
            vec![ToolCapability::Filesystem],
            vec!["Git".to_string(), "写入".to_string(), "需确认".to_string()],
        )
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        if args.get("checkout").is_some() {
            return Err(AgentError::Tool(
                "git_create_branch 不支持 checkout 参数；本工具只创建分支，不切换 HEAD。"
                    .to_string(),
            ));
        }
        let cwd = resolve_cwd(&args, &self.extra_roots)?;
        let root = repo_root(&cwd).await?;
        let branch = required_str(&args, "branch")?;
        validate_branch_name(&branch)?;
        let start_point = optional_str(&args, "startPoint")?;

        let check = run_git(&s(&["check-ref-format", "--branch", &branch]), &cwd).await?;
        if check.exit_code != 0 {
            return Err(AgentError::Tool(format!(
                "无效分支名 `{branch}`：{}",
                check.stderr.trim()
            )));
        }

        let mut cli = vec!["branch".to_string(), branch.clone()];
        if let Some(start_point) = start_point.clone() {
            cli.push(start_point);
        }
        if !confirmed(&args) {
            return Ok(confirmation_required_result(
                "git_create_branch 已生成预览，尚未创建分支。",
                &cli,
                &cwd,
                &root,
                json!({ "branch": branch, "startPoint": start_point }),
            ));
        }

        let output = run_git(&cli, &cwd).await?;
        if output.exit_code != 0 {
            return Ok(git_failed_result("git_create_branch", output, &cwd));
        }
        let mut data = output_data(&output);
        data["confirmed"] = json!(true);
        data["playbackState"] = json!("executed");
        data["cwd"] = json!(cwd.to_string_lossy().to_string());
        data["repoRoot"] = json!(root);
        data["branch"] = json!(branch);
        data["startPoint"] = json!(start_point);
        Ok(ToolResult::success("git_create_branch 已创建分支。", data))
    }
}

// ---------------------------------------------------------------------------

pub struct GitPushTool {
    extra_roots: Vec<PathBuf>,
}

impl GitPushTool {
    pub fn new_with_roots(extra_roots: Vec<PathBuf>) -> Self {
        Self {
            extra_roots: fs_scope::normalize_extra_roots(extra_roots),
        }
    }
}

impl Default for GitPushTool {
    fn default() -> Self {
        Self::new_with_roots(Vec::new())
    }
}

#[async_trait]
impl Tool for GitPushTool {
    fn name(&self) -> &str {
        "git_push"
    }

    fn description(&self) -> &str {
        "Push a named branch to a configured remote. Force/refspec pushes are refused."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Push a local branch to a configured remote. Requires confirmed=true. Force/delete/mirror/refspec pushes are not supported.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "cwd": { "type": "string", "description": "Working directory inside a git repo." },
                    "remote": { "type": "string", "description": "Configured remote name, e.g. origin. URLs are rejected." },
                    "branch": { "type": "string", "description": "Branch/ref name to push, e.g. main or HEAD. RefSpecs with ':' are rejected." },
                    "setUpstream": { "type": "boolean", "description": "If true, pass --set-upstream." },
                    "confirmed": {
                        "type": "boolean",
                        "description": "Must be true after user confirmation to contact the remote."
                    }
                },
                "required": ["remote", "branch", "confirmed"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        sensitive_git_metadata(
            self.name(),
            self.description(),
            "Git 推送",
            "确认后把指定分支推送到已配置远端；强制/refspec 推送默认拒绝。",
            vec![ToolCapability::Filesystem, ToolCapability::Network],
            vec![
                "Git".to_string(),
                "网络".to_string(),
                "写入".to_string(),
                "需确认".to_string(),
            ],
        )
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        reject_push_danger_fields(&args)?;
        let cwd = resolve_cwd(&args, &self.extra_roots)?;
        let root = repo_root(&cwd).await?;
        let remote = required_str(&args, "remote")?;
        validate_push_remote(&remote)?;
        let branch = required_str(&args, "branch")?;
        validate_push_branch(&branch)?;
        let set_upstream = args
            .get("setUpstream")
            .or_else(|| args.get("set_upstream"))
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let mut cli = vec!["push".to_string()];
        if set_upstream {
            cli.push("--set-upstream".to_string());
        }
        cli.push(remote.clone());
        cli.push(branch.clone());

        if !confirmed(&args) {
            return Ok(confirmation_required_result(
                "git_push 已生成预览，尚未连接远端。",
                &cli,
                &cwd,
                &root,
                json!({
                    "remote": remote,
                    "branch": branch,
                    "setUpstream": set_upstream,
                    "forceAllowed": false
                }),
            ));
        }

        let output = run_git(&cli, &cwd).await?;
        if output.exit_code != 0 {
            return Ok(git_failed_result("git_push", output, &cwd));
        }
        let mut data = output_data(&output);
        data["confirmed"] = json!(true);
        data["playbackState"] = json!("executed");
        data["cwd"] = json!(cwd.to_string_lossy().to_string());
        data["repoRoot"] = json!(root);
        data["remote"] = json!(remote);
        data["branch"] = json!(branch);
        data["setUpstream"] = json!(set_upstream);
        data["forceAllowed"] = json!(false);
        Ok(ToolResult::success("git_push 已完成推送。", data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::policy::AgentPermissionMode;
    use crate::tools::{PolicyEngine, ToolAccessPolicy};
    use std::process::Command;
    use tempfile::TempDir;

    fn git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .unwrap_or_else(|error| panic!("failed to run git {args:?}: {error}"));
        assert!(
            output.status.success(),
            "git {args:?} failed\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn test_repo() -> TempDir {
        let base = std::env::current_dir().unwrap().join("target");
        std::fs::create_dir_all(&base).unwrap();
        let repo = tempfile::Builder::new()
            .prefix("aura_git_write_")
            .tempdir_in(base)
            .unwrap();
        git(repo.path(), &["init"]);
        git(repo.path(), &["config", "user.name", "Aura Test"]);
        git(
            repo.path(),
            &["config", "user.email", "aura@example.invalid"],
        );
        git(repo.path(), &["config", "core.autocrlf", "false"]);
        std::fs::write(repo.path().join("README.md"), "initial\n").unwrap();
        git(repo.path(), &["add", "README.md"]);
        git(repo.path(), &["commit", "-m", "initial"]);
        git(repo.path(), &["branch", "-M", "main"]);
        repo
    }

    fn cwd(repo: &TempDir) -> String {
        repo.path().to_string_lossy().to_string()
    }

    #[tokio::test]
    async fn git_stage_requires_confirmation_without_mutating_index() {
        let repo = test_repo();
        std::fs::write(repo.path().join("notes.txt"), "draft\n").unwrap();
        let tool = GitStageTool::new_with_roots(vec![repo.path().to_path_buf()]);

        let result = tool
            .execute(json!({
                "cwd": cwd(&repo),
                "paths": ["notes.txt"],
                "confirmed": false
            }))
            .await
            .unwrap();

        assert!(matches!(result.status, ToolResultStatus::Warning));
        assert_eq!(result.data["playbackState"], json!("requires_confirmation"));
        assert!(git(repo.path(), &["diff", "--cached", "--name-only", "--"])
            .trim()
            .is_empty());
    }

    #[tokio::test]
    async fn git_stage_and_commit_create_real_commit_after_preflight() {
        let repo = test_repo();
        std::fs::write(repo.path().join("src.txt"), "hello\n").unwrap();
        let stage = GitStageTool::new_with_roots(vec![repo.path().to_path_buf()]);
        let commit = GitCommitTool::new_with_roots(vec![repo.path().to_path_buf()]);

        let stage_result = stage
            .execute(json!({
                "cwd": cwd(&repo),
                "paths": ["src.txt"],
                "confirmed": true
            }))
            .await
            .unwrap();
        assert!(matches!(stage_result.status, ToolResultStatus::Success));
        assert_eq!(stage_result.data["stagedFiles"], json!(["src.txt"]));

        let commit_result = commit
            .execute(json!({
                "cwd": cwd(&repo),
                "message": "add src file",
                "confirmed": true
            }))
            .await
            .unwrap();
        assert!(matches!(commit_result.status, ToolResultStatus::Success));
        assert_eq!(commit_result.data["stagedFiles"], json!(["src.txt"]));
        assert!(!commit_result.data["commit"].as_str().unwrap().is_empty());
        assert!(git(repo.path(), &["status", "--porcelain"])
            .trim()
            .is_empty());
        assert!(git(repo.path(), &["log", "--oneline", "-1"]).contains("add src file"));
    }

    #[tokio::test]
    async fn git_commit_blocks_secret_in_staged_diff() {
        let repo = test_repo();
        std::fs::write(
            repo.path().join("leak.txt"),
            "AWS_ACCESS_KEY_ID=AKIA1234567890ABCDEF\n",
        )
        .unwrap();
        git(repo.path(), &["add", "leak.txt"]);
        let tool = GitCommitTool::new_with_roots(vec![repo.path().to_path_buf()]);

        let result = tool
            .execute(json!({
                "cwd": cwd(&repo),
                "message": "add leak",
                "confirmed": true
            }))
            .await
            .unwrap();

        assert!(matches!(result.status, ToolResultStatus::Error));
        assert_eq!(result.data["playbackState"], json!("blocked"));
        assert_eq!(
            result.data["secretFindings"][0]["ref"],
            json!("staged_diff")
        );
        assert!(!git(repo.path(), &["log", "--oneline"]).contains("add leak"));
        assert_eq!(
            git(repo.path(), &["diff", "--cached", "--name-only", "--"]).trim(),
            "leak.txt"
        );
    }

    #[tokio::test]
    async fn git_commit_blocks_secret_in_message_before_mutating() {
        let repo = test_repo();
        std::fs::write(repo.path().join("safe.txt"), "safe\n").unwrap();
        git(repo.path(), &["add", "safe.txt"]);
        let tool = GitCommitTool::new_with_roots(vec![repo.path().to_path_buf()]);

        let result = tool
            .execute(json!({
                "cwd": cwd(&repo),
                "message": "rotate sk-proj-1234567890abcdefghijklmnop",
                "confirmed": true
            }))
            .await
            .unwrap();

        assert!(matches!(result.status, ToolResultStatus::Error));
        assert_eq!(
            result.data["secretFindings"][0]["ref"],
            json!("commit_message")
        );
        assert!(!git(repo.path(), &["log", "--oneline"]).contains("rotate"));
    }

    #[tokio::test]
    async fn git_commit_accepts_multiline_or_dash_prefixed_message_values() {
        let repo = test_repo();
        std::fs::write(repo.path().join("dash.txt"), "safe\n").unwrap();
        git(repo.path(), &["add", "dash.txt"]);
        let tool = GitCommitTool::new_with_roots(vec![repo.path().to_path_buf()]);

        let result = tool
            .execute(json!({
                "cwd": cwd(&repo),
                "message": "- scoped subject\n\nbody line",
                "confirmed": true
            }))
            .await
            .unwrap();

        assert!(matches!(result.status, ToolResultStatus::Success));
        assert!(git(repo.path(), &["log", "--format=%B", "-1"]).contains("- scoped subject"));
    }

    #[tokio::test]
    async fn git_create_branch_requires_confirmation_then_creates_branch() {
        let repo = test_repo();
        let tool = GitCreateBranchTool::new_with_roots(vec![repo.path().to_path_buf()]);
        let preview = tool
            .execute(json!({
                "cwd": cwd(&repo),
                "branch": "feature/p3-1",
                "confirmed": false
            }))
            .await
            .unwrap();
        assert!(matches!(preview.status, ToolResultStatus::Warning));
        let missing = Command::new("git")
            .args(["rev-parse", "--verify", "feature/p3-1"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(!missing.status.success());

        let result = tool
            .execute(json!({
                "cwd": cwd(&repo),
                "branch": "feature/p3-1",
                "confirmed": true
            }))
            .await
            .unwrap();
        assert!(matches!(result.status, ToolResultStatus::Success));
        git(repo.path(), &["rev-parse", "--verify", "feature/p3-1"]);
    }

    #[tokio::test]
    async fn git_push_requires_confirmation_and_rejects_force_refspecs() {
        let repo = test_repo();
        let tool = GitPushTool::new_with_roots(vec![repo.path().to_path_buf()]);

        let preview = tool
            .execute(json!({
                "cwd": cwd(&repo),
                "remote": "origin",
                "branch": "HEAD",
                "confirmed": false
            }))
            .await
            .unwrap();
        assert!(matches!(preview.status, ToolResultStatus::Warning));
        assert_eq!(preview.data["forceAllowed"], json!(false));

        assert!(tool
            .execute(json!({
                "cwd": cwd(&repo),
                "remote": "origin",
                "branch": "main",
                "force": true,
                "confirmed": true
            }))
            .await
            .is_err());
        assert!(tool
            .execute(json!({
                "cwd": cwd(&repo),
                "remote": "origin",
                "branch": "main:main",
                "confirmed": true
            }))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn git_push_pushes_branch_to_configured_local_remote() {
        let repo = test_repo();
        let base = std::env::current_dir().unwrap().join("target");
        let bare = tempfile::Builder::new()
            .prefix("aura_git_write_remote_")
            .tempdir_in(base)
            .unwrap();
        git(bare.path(), &["init", "--bare"]);
        git(
            repo.path(),
            &["remote", "add", "origin", &bare.path().to_string_lossy()],
        );
        let tool = GitPushTool::new_with_roots(vec![repo.path().to_path_buf()]);

        let result = tool
            .execute(json!({
                "cwd": cwd(&repo),
                "remote": "origin",
                "branch": "main",
                "setUpstream": true,
                "confirmed": true
            }))
            .await
            .unwrap();

        assert!(matches!(result.status, ToolResultStatus::Success));
        assert_eq!(result.data["forceAllowed"], json!(false));
        git(bare.path(), &["rev-parse", "--verify", "refs/heads/main"]);
    }

    #[test]
    fn git_write_metadata_is_sensitive_confirmed_and_plan_blocked() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(GitStageTool::default()),
            Box::new(GitCommitTool::default()),
            Box::new(GitCreateBranchTool::default()),
            Box::new(GitPushTool::default()),
        ];
        let plan = ToolAccessPolicy::Plan;
        let default = ToolAccessPolicy::Default;
        for tool in tools {
            let metadata = tool.metadata();
            assert_eq!(metadata.safety_level, ToolSafetyLevel::Sensitive);
            assert!(metadata.mutates_state);
            assert!(metadata.requires_confirmation);
            assert!(!plan.allows_metadata(&metadata));
            assert!(default.allows_metadata(&metadata));
            assert!(PolicyEngine::new(AgentPermissionMode::Plan)
                .evaluate_tool_execution(&metadata)
                .reason()
                .is_some());
        }
    }
}
