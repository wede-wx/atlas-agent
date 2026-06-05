use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 一条验证命令的来源。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationSource {
    /// 任务自身 verify_json 字段。
    Task,
    /// `<project_root>/.atlas/verify.toml`。
    ProjectFile,
    /// `~/.atlas/verify.toml`。
    UserFile,
    /// 仓库脚本推断（package.json scripts / Cargo.toml 等）。
    RepoScripts,
    /// 内置兜底默认值。
    Builtin,
}

/// 一条验证条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationCommand {
    /// 类型：fmt / lint / test / build / smoke / manual。
    pub kind: String,
    /// 实际 shell 命令。
    pub command: String,
    pub source: VerificationSource,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerifyConfig {
    /// kind → command。
    pub commands: BTreeMap<String, String>,
    /// P2-1: opt-in matcher for auto-verify-after-command. A regex; when a
    /// `run_command` matches it, the agent main loop auto-runs the active task's
    /// verify mid-run (not only at done-time). `None` = off (the default), so
    /// there is zero added cost / behavior change unless a project opts in via
    /// `auto_after_command = "<regex>"` in `.atlas/verify.toml`.
    #[serde(default)]
    pub auto_after_command: Option<String>,
}

impl VerifyConfig {
    pub fn empty() -> Self {
        Self {
            commands: BTreeMap::new(),
            auto_after_command: None,
        }
    }

    pub fn parse_toml(contents: &str) -> Result<Self, toml::de::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            commands: BTreeMap<String, String>,
            #[serde(default)]
            auto_after_command: Option<String>,
        }
        let raw: Raw = toml::from_str(contents)?;
        Ok(Self {
            commands: raw.commands,
            // Keep the matcher verbatim (a regex may legitimately contain trailing
            // spaces, e.g. "^cargo "); only drop a blank/whitespace-only value.
            auto_after_command: raw
                .auto_after_command
                .filter(|value| !value.trim().is_empty()),
        })
    }

    pub fn merge(mut self, other: &VerifyConfig) -> Self {
        for (k, v) in &other.commands {
            self.commands.entry(k.clone()).or_insert_with(|| v.clone());
        }
        // Keep self's matcher if set (higher priority), else inherit other's.
        if self.auto_after_command.is_none() {
            self.auto_after_command = other.auto_after_command.clone();
        }
        self
    }
}

/// 内置兜底默认。对应 plan §M3.3 的 [commands] 初始内容。
pub fn builtin_config() -> VerifyConfig {
    let mut commands = BTreeMap::new();
    commands.insert(
        "frontend_typecheck".to_string(),
        "npx tsc --noEmit".to_string(),
    );
    commands.insert("frontend_build".to_string(), "npm run build".to_string());
    commands.insert("frontend_smoke".to_string(), "npm run smoke".to_string());
    commands.insert("full_verify".to_string(), "npm run verify".to_string());
    commands.insert(
        "rust_fmt".to_string(),
        "cargo fmt --manifest-path ./src-tauri/Cargo.toml --check".to_string(),
    );
    commands.insert(
        "rust_check".to_string(),
        "cargo check --manifest-path ./src-tauri/Cargo.toml --target-dir ./output/cargo-check"
            .to_string(),
    );
    commands.insert(
        "rust_lib_test".to_string(),
        "cargo test --manifest-path ./src-tauri/Cargo.toml --lib -- --nocapture".to_string(),
    );
    VerifyConfig {
        commands,
        auto_after_command: None,
    }
}

/// 按 plan §M3.3 的优先级解析当前可用的 verify 命令集合。
///
/// 优先级（高→低）：
/// 1. task verify_json （由调用方传入 task_verify）
/// 2. `<project_root>/.atlas/verify.toml`
/// 3. `~/.atlas/verify.toml`
/// 4. 仓库脚本推断（目前未做，留 TODO，由 M3.4 实现）
/// 5. 内置兜底
pub fn load_verify_config(
    project_root: Option<&Path>,
    user_home: Option<&Path>,
    task_verify: Option<&serde_json::Value>,
) -> VerifyConfig {
    let mut merged = VerifyConfig::empty();

    // 1. task verify_json
    if let Some(value) = task_verify {
        if let Some(commands) = task_commands_from_json(value) {
            for (k, v) in commands {
                merged.commands.entry(k).or_insert(v);
            }
        }
    }

    // 2. project-root .atlas/verify.toml
    if let Some(root) = project_root {
        let path = root.join(".atlas").join("verify.toml");
        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = VerifyConfig::parse_toml(&contents) {
                merged = merged.merge(&cfg);
            }
        }
    }

    // 3. user .atlas/verify.toml
    if let Some(home) = user_home {
        let path = home.join(".atlas").join("verify.toml");
        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = VerifyConfig::parse_toml(&contents) {
                merged = merged.merge(&cfg);
            }
        }
    }

    // 5. builtin
    merged.merge(&builtin_config())
}

fn task_commands_from_json(value: &serde_json::Value) -> Option<BTreeMap<String, String>> {
    let array = value.as_array()?;
    let mut commands = BTreeMap::new();
    for entry in array {
        let kind = entry
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("manual")
            .trim()
            .to_string();
        let command = entry
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if !kind.is_empty() && !command.is_empty() {
            commands.insert(kind, command);
        }
    }
    if commands.is_empty() {
        None
    } else {
        Some(commands)
    }
}

/// 返回 `~/.atlas` 目录路径（用于 `~/.atlas/verify.toml`）。
pub fn user_atlas_home() -> Option<PathBuf> {
    if let Some(home) = dirs::home_dir() {
        return Some(home.join(".atlas"));
    }
    None
}

/// Stderr 签名：用于 plan §M3.4 的 "同一 stderr signature 连续两次失败停止"。
/// 取前 200 个有效字符做 hash，避免行号 / 时间戳干扰。
pub fn stderr_signature(stderr: &str) -> String {
    let trimmed = stderr.trim();
    let normalized: String = trimmed
        .chars()
        .filter(|c| !c.is_whitespace() || *c == ' ')
        .take(200)
        .collect();
    if normalized.trim().is_empty() {
        "empty".to_string()
    } else {
        format!("{:x}", md5_like(&normalized))
    }
}

fn md5_like(s: &str) -> u64 {
    // 不引入新依赖；用 FxHash 风格的简易 hash 当签名。
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// 在不引入外部命令依赖的最小范围内描述 verification 执行结果。
/// 真实命令执行由 commands/agent.rs 调用 tools::execute_shell_command 完成；
/// 这个结构只负责把结果归一化后写入 run_task_verifications 表。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationOutcome {
    pub task_id: String,
    pub kind: String,
    pub command: String,
    pub exit_code: Option<i64>,
    pub status: String,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

impl VerificationOutcome {
    pub fn signature(&self) -> String {
        stderr_signature(&self.stderr_tail)
    }
}

#[cfg(test)]
mod signature_tests {
    use super::*;

    #[test]
    fn stable_signature_for_same_stderr() {
        let sig1 = stderr_signature("error: cannot find type `Foo`\n  -> src/a.rs:42");
        let sig2 = stderr_signature("error: cannot find type `Foo`\n  -> src/a.rs:42");
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn signature_differs_for_different_errors() {
        let sig1 = stderr_signature("error: foo");
        let sig2 = stderr_signature("error: bar");
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn empty_stderr_signature_is_stable() {
        assert_eq!(stderr_signature(""), "empty");
        assert_eq!(stderr_signature("   "), "empty");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_verify_takes_priority() {
        let task_verify = serde_json::json!([
            { "kind": "rust_check", "command": "echo task" }
        ]);
        let cfg = load_verify_config(None, None, Some(&task_verify));
        assert_eq!(
            cfg.commands.get("rust_check"),
            Some(&"echo task".to_string())
        );
    }

    #[test]
    fn builtin_provides_baseline_when_no_overrides() {
        let cfg = load_verify_config(None, None, None);
        assert!(cfg.commands.contains_key("frontend_build"));
        assert!(cfg.commands.contains_key("rust_check"));
    }

    #[test]
    fn parse_toml_round_trip() {
        let cfg = VerifyConfig::parse_toml(
            r#"[commands]
            frontend_smoke = "pnpm smoke"
            rust_check = "cargo check"
            "#,
        )
        .expect("parse");
        assert_eq!(
            cfg.commands.get("frontend_smoke"),
            Some(&"pnpm smoke".to_string())
        );
        assert_eq!(
            cfg.commands.get("rust_check"),
            Some(&"cargo check".to_string())
        );
    }

    #[test]
    fn auto_after_command_parses_and_defaults_off() {
        // P2-1: matcher is opt-in. Absent => None (no auto-verify after commands).
        let off = VerifyConfig::parse_toml("[commands]\nx = \"echo\"\n").expect("parse");
        assert!(off.auto_after_command.is_none());

        let on = VerifyConfig::parse_toml(
            "auto_after_command = \"^cargo \"\n[commands]\nx = \"echo\"\n",
        )
        .expect("parse");
        assert_eq!(on.auto_after_command.as_deref(), Some("^cargo "));

        // Blank string is treated as off.
        let blank = VerifyConfig::parse_toml("auto_after_command = \"   \"\n").expect("parse");
        assert!(blank.auto_after_command.is_none());
    }

    #[test]
    fn merge_keeps_higher_priority_matcher_else_inherits() {
        // self (higher priority) wins when set; otherwise inherit from other.
        let with = VerifyConfig {
            commands: BTreeMap::new(),
            auto_after_command: Some("^npm ".to_string()),
        };
        let other = VerifyConfig {
            commands: BTreeMap::new(),
            auto_after_command: Some("^cargo ".to_string()),
        };
        assert_eq!(
            with.clone().merge(&other).auto_after_command.as_deref(),
            Some("^npm ")
        );
        assert_eq!(
            VerifyConfig::empty()
                .merge(&other)
                .auto_after_command
                .as_deref(),
            Some("^cargo ")
        );
    }

    /// T10 — real on-disk integration test: write project + user verify.toml
    /// files, then assert load_verify_config picks them up in correct priority order.
    #[test]
    fn integration_load_verify_config_priority() {
        use uuid::Uuid;
        let base = std::env::temp_dir().join(format!("atlas_verify_int_{}", Uuid::new_v4()));
        let project_root = base.join("proj");
        let user_home = base.join("home");
        std::fs::create_dir_all(project_root.join(".atlas")).unwrap();
        std::fs::create_dir_all(user_home.join(".atlas")).unwrap();

        std::fs::write(
            project_root.join(".atlas").join("verify.toml"),
            r#"[commands]
rust_check = "PROJECT cargo check"
project_only = "PROJECT only"
"#,
        )
        .unwrap();
        std::fs::write(
            user_home.join(".atlas").join("verify.toml"),
            r#"[commands]
rust_check = "USER cargo check"
user_only = "USER only"
"#,
        )
        .unwrap();

        let task_verify = serde_json::json!([
            { "kind": "frontend_build", "command": "TASK npm build" }
        ]);

        let cfg = load_verify_config(Some(&project_root), Some(&user_home), Some(&task_verify));

        // task verify wins for the kind it defines
        assert_eq!(
            cfg.commands.get("frontend_build"),
            Some(&"TASK npm build".to_string()),
            "task verify should take priority"
        );
        // project file beats user file for the same kind
        assert_eq!(
            cfg.commands.get("rust_check"),
            Some(&"PROJECT cargo check".to_string()),
            "project .atlas/verify.toml should beat user one"
        );
        // project-only key present
        assert_eq!(
            cfg.commands.get("project_only"),
            Some(&"PROJECT only".to_string())
        );
        // user-only key reachable when project doesn't override
        assert_eq!(
            cfg.commands.get("user_only"),
            Some(&"USER only".to_string())
        );
        // builtin still merged in
        assert!(cfg.commands.contains_key("frontend_smoke"));

        let _ = std::fs::remove_dir_all(base);
    }
}
