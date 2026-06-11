use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::agent::AgentError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionIsolationConfig {
    #[serde(default = "default_true")]
    pub command_workspace_boundary: bool,
    #[serde(default)]
    pub allowed_command_roots: Vec<String>,
    #[serde(default = "default_command_env_allowlist")]
    pub command_env_allowlist: Vec<String>,
    /// Step 3：true = fail-closed——OS 沙箱（Landlock/seatbelt）不可用时拒绝
    /// 执行命令；false（默认）= 降级为策略级边界并在审计中披露。
    #[serde(default)]
    pub require_sandbox: bool,
}

impl Default for ExecutionIsolationConfig {
    fn default() -> Self {
        Self {
            command_workspace_boundary: true,
            allowed_command_roots: Vec::new(),
            command_env_allowlist: default_command_env_allowlist(),
            require_sandbox: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandIsolationPolicy {
    workspace_boundary: bool,
    default_cwd: PathBuf,
    allowed_roots: Vec<PathBuf>,
    env_allowlist: Vec<String>,
    require_sandbox: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandIsolationReport {
    pub workspace_boundary: bool,
    pub cwd_within_allowed_roots: bool,
    pub default_cwd: String,
    pub allowed_roots: Vec<String>,
    pub env_policy: String,
    pub injected_env: Vec<String>,
    pub blocked_sensitive_env_count: usize,
    /// Step 3：实际生效的 OS 沙箱后端（landlock/seatbelt/boundary_only）。
    #[serde(default = "default_sandbox_backend")]
    pub sandbox_backend: String,
    /// Step 3：本次命令是否有 OS 级强制。false 必须可见——诚实披露。
    #[serde(default)]
    pub sandbox_enforced: bool,
    #[serde(default)]
    pub sandbox_detail: String,
}

fn default_sandbox_backend() -> String {
    "boundary_only".to_string()
}

impl CommandIsolationPolicy {
    pub fn from_config(config: &ExecutionIsolationConfig, project_roots: &[PathBuf]) -> Self {
        let mut roots = Vec::new();
        for root in project_roots {
            push_normalized_root(&mut roots, root.clone());
        }
        for root in &config.allowed_command_roots {
            push_normalized_root(&mut roots, expanded_path(root));
        }
        if roots.is_empty() {
            if let Ok(cwd) = std::env::current_dir() {
                push_normalized_root(&mut roots, cwd);
            }
        }
        let default_cwd = roots
            .first()
            .cloned()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            workspace_boundary: config.command_workspace_boundary,
            default_cwd,
            allowed_roots: roots,
            env_allowlist: normalized_env_allowlist(&config.command_env_allowlist),
            require_sandbox: config.require_sandbox,
        }
    }

    /// Step 3：把命令边界（allowed_roots）翻译成 OS 沙箱规格——策略级与 OS 级
    /// 用同一份可写根，两层边界永远一致，不会出现"策略允许、沙箱拒绝"的漂移。
    pub fn sandbox_spec(&self) -> crate::tools::sandbox::SandboxSpec {
        crate::tools::sandbox::SandboxSpec {
            writable_roots: self.allowed_roots.clone(),
            require_sandbox: self.require_sandbox,
        }
    }

    pub fn default_for_current_dir() -> Self {
        Self::from_config(&ExecutionIsolationConfig::default(), &[])
    }

    pub fn resolve_cwd(
        &self,
        cwd: Option<&str>,
    ) -> Result<(PathBuf, CommandIsolationReport), AgentError> {
        let raw = cwd
            .map(expanded_path)
            .unwrap_or_else(|| self.default_cwd.clone());
        let absolute = if raw.is_absolute() {
            raw
        } else {
            self.default_cwd.join(raw)
        };
        let real = absolute
            .canonicalize()
            .map_err(|error| AgentError::Tool(format!("无法解析命令工作目录: {error}")))?;
        if !real.is_dir() {
            return Err(AgentError::Tool("命令工作目录不是文件夹。".to_string()));
        }
        if is_sensitive_path(&real) {
            return Err(AgentError::Tool(
                "命令工作目录属于系统、密钥或敏感应用目录。".to_string(),
            ));
        }
        let within_allowed = self
            .allowed_roots
            .iter()
            .any(|root| path_starts_with(&real, root));
        if self.workspace_boundary && !within_allowed {
            return Err(AgentError::Tool(
                "命令工作目录不在允许的项目工作目录边界内。".to_string(),
            ));
        }
        Ok((real, self.report(Vec::new(), 0, within_allowed)))
    }

    pub fn apply_env(&self, command: &mut Command) -> (Vec<String>, usize) {
        command.env_clear();
        let allowed_patterns = self.env_patterns();
        let mut injected = BTreeSet::new();
        let mut blocked_sensitive = 0usize;
        for (key, value) in std::env::vars_os() {
            let key_text = key.to_string_lossy().to_string();
            if !env_key_allowed(&key_text, &allowed_patterns) {
                continue;
            }
            if is_sensitive_env_key(&key_text) {
                blocked_sensitive += 1;
                continue;
            }
            command.env(&key, value);
            injected.insert(key_text);
        }
        command.env("ATLAS_EXECUTION_ISOLATED", "1");
        injected.insert("ATLAS_EXECUTION_ISOLATED".to_string());
        if let Some(root) = self.allowed_roots.first() {
            command.env("ATLAS_COMMAND_WORKSPACE_ROOT", root);
            injected.insert("ATLAS_COMMAND_WORKSPACE_ROOT".to_string());
        }
        (injected.into_iter().collect(), blocked_sensitive)
    }

    pub fn report(
        &self,
        injected_env: Vec<String>,
        blocked_sensitive_env_count: usize,
        cwd_within_allowed_roots: bool,
    ) -> CommandIsolationReport {
        CommandIsolationReport {
            workspace_boundary: self.workspace_boundary,
            cwd_within_allowed_roots,
            default_cwd: self.default_cwd.to_string_lossy().to_string(),
            allowed_roots: self
                .allowed_roots
                .iter()
                .map(|root| root.to_string_lossy().to_string())
                .collect(),
            env_policy: "allowlist".to_string(),
            injected_env,
            blocked_sensitive_env_count,
            // 占位：执行点（command.rs）拿到真实 SandboxApplication 后覆盖。
            sandbox_backend: default_sandbox_backend(),
            sandbox_enforced: false,
            sandbox_detail: String::new(),
        }
    }

    fn env_patterns(&self) -> Vec<String> {
        let mut patterns = normalized_env_allowlist(&default_command_env_allowlist());
        patterns.extend(self.env_allowlist.iter().cloned());
        patterns.sort();
        patterns.dedup();
        patterns
    }
}

pub fn is_sensitive_path(path: &Path) -> bool {
    let text = path.to_string_lossy().to_ascii_lowercase();
    [
        "\\windows\\",
        "\\program files\\",
        "\\program files (x86)\\",
        "\\appdata\\local\\",
        "\\appdata\\roaming\\",
        "\\.ssh\\",
        "\\.gnupg\\",
        "\\.aws\\",
        "/etc/",
        "/usr/bin/",
        "/usr/sbin/",
        "/root/",
        "/.ssh/",
        "/.gnupg/",
        "/.aws/",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

pub fn is_sensitive_env_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.contains("API_KEY")
        || upper.contains("TOKEN")
        || upper.contains("SECRET")
        || upper.contains("PASSWORD")
        || upper.contains("PASSWD")
        || upper.contains("PRIVATE_KEY")
        || upper.contains("CREDENTIAL")
        || upper == "OPENAI_API_KEY"
        || upper == "ANTHROPIC_API_KEY"
}

fn default_true() -> bool {
    true
}

fn default_command_env_allowlist() -> Vec<String> {
    let mut keys = vec![
        "PATH",
        "TEMP",
        "TMP",
        "TMPDIR",
        "HOME",
        "USER",
        "USERNAME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "LANG",
        "LC_*",
        "TERM",
        "SHELL",
        "COMSPEC",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "APPDATA",
        "LOCALAPPDATA",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "NPM_CONFIG_CACHE",
        "PNPM_HOME",
        "YARN_CACHE_FOLDER",
        "CI",
    ];
    keys.sort();
    keys.dedup();
    keys.into_iter().map(str::to_string).collect()
}

fn normalized_env_allowlist(keys: &[String]) -> Vec<String> {
    let mut out = keys
        .iter()
        .map(|key| key.trim().to_ascii_uppercase())
        .filter(|key| !key.is_empty())
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

fn env_key_allowed(key: &str, patterns: &[String]) -> bool {
    let upper = key.to_ascii_uppercase();
    patterns.iter().any(|pattern| {
        pattern
            .strip_suffix('*')
            .map(|prefix| upper.starts_with(prefix))
            .unwrap_or_else(|| &upper == pattern)
    })
}

fn expanded_path(path: &str) -> PathBuf {
    let path = path.trim();
    if (path.starts_with("~/") || path.starts_with("~\\")) && dirs::home_dir().is_some() {
        let home = dirs::home_dir().expect("checked above");
        return home.join(path.trim_start_matches("~/").trim_start_matches("~\\"));
    }
    let raw = PathBuf::from(path);
    if raw.is_absolute() {
        raw
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(raw)
    }
}

fn push_normalized_root(roots: &mut Vec<PathBuf>, root: PathBuf) {
    if let Ok(real) = root.canonicalize() {
        if real.is_dir() && !is_sensitive_path(&real) && !roots.iter().any(|seen| seen == &real) {
            roots.push(real);
        }
    }
}

fn path_starts_with(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let path_text = path.to_string_lossy().to_ascii_lowercase();
        let root_text = root.to_string_lossy().to_ascii_lowercase();
        path_text == root_text
            || path_text.starts_with(&format!(
                "{}{}",
                root_text.trim_end_matches(['\\', '/']),
                std::path::MAIN_SEPARATOR
            ))
    }
    #[cfg(not(windows))]
    {
        path.starts_with(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn policy_defaults_to_current_dir_boundary() {
        let policy = CommandIsolationPolicy::default_for_current_dir();
        assert!(policy.workspace_boundary);
        assert!(!policy.allowed_roots.is_empty());
    }

    #[test]
    fn blocks_sensitive_env_names_even_when_allowlisted() {
        let config = ExecutionIsolationConfig {
            command_workspace_boundary: true,
            allowed_command_roots: Vec::new(),
            command_env_allowlist: vec!["OPENAI_API_KEY".to_string(), "NORMAL_ENV".to_string()],
            require_sandbox: false,
        };
        let policy = CommandIsolationPolicy::from_config(&config, &[]);
        let patterns = policy.env_patterns();
        assert!(env_key_allowed("OPENAI_API_KEY", &patterns));
        assert!(is_sensitive_env_key("OPENAI_API_KEY"));
        assert!(!is_sensitive_env_key("NORMAL_ENV"));
    }

    #[test]
    fn cwd_outside_project_root_is_rejected() {
        let base = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("atlas-isolation-{}", Uuid::new_v4()));
        let allowed = base.join("allowed");
        let outside = base.join("outside");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let policy = CommandIsolationPolicy::from_config(
            &ExecutionIsolationConfig::default(),
            std::slice::from_ref(&allowed),
        );

        let result = policy.resolve_cwd(Some(&outside.to_string_lossy()));

        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn configured_extra_root_is_allowed() {
        let base = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("atlas-isolation-extra-{}", Uuid::new_v4()));
        let project = base.join("project");
        let extra = base.join("extra");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&extra).unwrap();
        let config = ExecutionIsolationConfig {
            command_workspace_boundary: true,
            allowed_command_roots: vec![extra.to_string_lossy().to_string()],
            command_env_allowlist: default_command_env_allowlist(),
            require_sandbox: false,
        };
        let policy = CommandIsolationPolicy::from_config(&config, &[project]);

        let (cwd, report) = policy.resolve_cwd(Some(&extra.to_string_lossy())).unwrap();

        assert_eq!(cwd, extra.canonicalize().unwrap());
        assert!(report.cwd_within_allowed_roots);
        let _ = std::fs::remove_dir_all(base);
    }
}
