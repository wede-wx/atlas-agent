//! P1-6: perception primitives — structured user-intent classification and a
//! project snapshot.
//!
//! These let the agent (and later P3-4 Goal / plan triggering) reason over *what
//! the user wants* and *what the project is*, instead of guessing from a long
//! prompt. Two red lines drive the design: intent classification must be real
//! (never a constant), and the project snapshot must never drag dependency or
//! build directories (`node_modules`, `target`, …) into the model context.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// User intent
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntentType {
    Chat,
    Question,
    Task,
    Edit,
    Debug,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserIntent {
    pub intent_type: IntentType,
    /// Does fulfilling this require tool action (write / command / edit) rather
    /// than a purely conversational reply?
    pub needs_action: bool,
    /// Too vague to act on safely — the agent should ask one minimal question
    /// before doing anything.
    pub needs_clarification: bool,
    pub urgency: Urgency,
    /// Why this classification — matched rule tags, for transparency / audit.
    pub signals: Vec<String>,
}

const DEBUG_KW: &[&str] = &[
    "报错",
    "错误",
    "bug",
    "崩溃",
    "失败",
    "不工作",
    "无法运行",
    "fix",
    "修复",
    "调试",
    "debug",
    "异常",
    "panic",
    "报错信息",
    "traceback",
    "stacktrace",
];
const REVIEW_KW: &[&str] = &[
    "审查",
    "评审",
    "review",
    "审一下",
    "复查",
    "检查代码",
    "看下代码",
    "看一下代码",
    "代码审查",
];
const EDIT_KW: &[&str] = &[
    "修改",
    "改一下",
    "改成",
    "编辑",
    "重命名",
    "替换",
    "重构",
    "refactor",
    "rename",
    "调整",
    "改掉",
    "更新一下",
];
const TASK_KW: &[&str] = &[
    "帮我",
    "做一个",
    "做个",
    "创建",
    "新建",
    "实现",
    "写一个",
    "写个",
    "生成",
    "搭一个",
    "部署",
    "安装",
    "配置",
    "新增",
    "build",
    "create",
    "implement",
    "generate",
    "跑一下",
    "运行",
];
const QUESTION_KW: &[&str] = &[
    "吗",
    "什么",
    "怎么",
    "为什么",
    "如何",
    "是不是",
    "可以吗",
    "能不能",
    "what",
    "why",
    "how",
    "which",
    "where",
    "?",
    "？",
];
const CHAT_KW: &[&str] = &[
    "你好",
    "您好",
    "hi",
    "hello",
    "嗨",
    "谢谢",
    "thanks",
    "thank you",
    "辛苦",
    "再见",
    "bye",
    "早上好",
    "晚上好",
    "哈喽",
    "在吗",
];

fn contains_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

/// Rule-based first-pass intent classifier (bilingual zh / en). Deterministic and
/// real — never a constant. A model-based refinement can layer on top later; the
/// rule layer is the safe default that plan-triggering keys off.
pub fn classify_intent(message: &str) -> UserIntent {
    let raw = message.trim();
    let lower = raw.to_lowercase();
    let char_len = raw.chars().count();
    let mut signals: Vec<String> = Vec::new();

    // Most specific action intents first, so "帮我修复报错" reads as debug, not task.
    let intent_type = if contains_any(&lower, DEBUG_KW) {
        signals.push("kw:debug".to_string());
        IntentType::Debug
    } else if contains_any(&lower, REVIEW_KW) {
        signals.push("kw:review".to_string());
        IntentType::Review
    } else if contains_any(&lower, EDIT_KW) {
        signals.push("kw:edit".to_string());
        IntentType::Edit
    } else if contains_any(&lower, TASK_KW) {
        signals.push("kw:task".to_string());
        IntentType::Task
    } else if contains_any(&lower, QUESTION_KW) {
        signals.push("kw:question".to_string());
        IntentType::Question
    } else if contains_any(&lower, CHAT_KW) || char_len <= 6 {
        signals.push("kw:chat".to_string());
        IntentType::Chat
    } else {
        // A non-trivial statement with no action or question marker: respond, don't act.
        signals.push("default:question".to_string());
        IntentType::Question
    };

    let needs_action = matches!(
        intent_type,
        IntentType::Task | IntentType::Edit | IntentType::Debug | IntentType::Review
    );

    let urgency = if contains_any(
        &lower,
        &["紧急", "马上", "立刻", "立即", "尽快", "urgent", "asap"],
    ) || raw.contains("！！")
        || raw.contains("!!")
    {
        signals.push("urgency:high".to_string());
        Urgency::High
    } else if matches!(intent_type, IntentType::Chat) {
        Urgency::Low
    } else {
        Urgency::Normal
    };

    // An action ask too short to carry a concrete target → ask one minimal question
    // instead of guessing (the card's "不确定时降级为问最小问题").
    let needs_clarification = needs_action && char_len <= 4;
    if needs_clarification {
        signals.push("clarify:too_vague".to_string());
    }

    UserIntent {
        intent_type,
        needs_action,
        needs_clarification,
        urgency,
        signals,
    }
}

// ---------------------------------------------------------------------------
// Project snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSnapshot {
    pub root: String,
    pub languages: Vec<String>,
    pub package_managers: Vec<String>,
    pub important_files: Vec<String>,
    pub entry_points: Vec<String>,
    pub test_commands: Vec<String>,
    pub ignored_patterns: Vec<String>,
}

const SNAPSHOT_MAX_ENTRIES: usize = 4_000;
const SNAPSHOT_MAX_DEPTH: usize = 8;

/// Directory names that are dependencies or build artifacts — never walked into,
/// never surfaced to the model (the P1-6 red line: a snapshot must not drag
/// `node_modules` / `target` into context).
pub fn ignored_dir_names() -> &'static [&'static str] {
    &[
        "node_modules",
        "target",
        "dist",
        "build",
        "out",
        "bin",
        "obj",
        ".git",
        ".svn",
        ".hg",
        "__pycache__",
        ".venv",
        "venv",
        ".next",
        ".nuxt",
        ".turbo",
        ".svelte-kit",
        "vendor",
        ".gradle",
        ".idea",
        ".vscode",
        "coverage",
        ".cache",
        ".cargo",
        ".pytest_cache",
    ]
}

/// Scan a project root into a structured snapshot: tech stack, package managers,
/// key files, entry points and how to test — with dependency / build directories
/// excluded. The walk is bounded in breadth and depth so a huge or pathological
/// tree can't stall or blow up.
pub fn scan_project_snapshot(root: &Path) -> ProjectSnapshot {
    let mut languages: BTreeSet<String> = BTreeSet::new();
    let mut package_managers: BTreeSet<String> = BTreeSet::new();
    let mut important_files: Vec<String> = Vec::new();
    let mut entry_points: BTreeSet<String> = BTreeSet::new();
    let mut test_commands: BTreeSet<String> = BTreeSet::new();

    detect_manifests(
        root,
        &mut languages,
        &mut package_managers,
        &mut important_files,
        &mut test_commands,
    );

    let mut visited = 0usize;
    walk_for_languages(
        root,
        root,
        0,
        &mut visited,
        &mut languages,
        &mut entry_points,
    );

    ProjectSnapshot {
        root: root.to_string_lossy().to_string(),
        languages: languages.into_iter().collect(),
        package_managers: package_managers.into_iter().collect(),
        important_files,
        entry_points: entry_points.into_iter().collect(),
        test_commands: test_commands.into_iter().collect(),
        ignored_patterns: ignored_dir_names().iter().map(|s| s.to_string()).collect(),
    }
}

impl ProjectSnapshot {
    /// Concise prompt the agent can read to know the stack and how to build / test,
    /// so it stops guessing. Empty sections are omitted.
    pub fn context_prompt(&self) -> String {
        let mut lines = vec!["## 项目快照（自动感知，仅供参考）".to_string()];
        lines.push(format!("- 根目录: {}", self.root));
        if !self.languages.is_empty() {
            lines.push(format!("- 技术栈: {}", self.languages.join(", ")));
        }
        if !self.package_managers.is_empty() {
            lines.push(format!("- 包管理: {}", self.package_managers.join(", ")));
        }
        if !self.entry_points.is_empty() {
            lines.push(format!("- 入口: {}", self.entry_points.join(", ")));
        }
        if !self.test_commands.is_empty() {
            lines.push(format!("- 构建/测试: {}", self.test_commands.join("; ")));
        }
        if !self.important_files.is_empty() {
            lines.push(format!("- 关键文件: {}", self.important_files.join(", ")));
        }
        lines.push("（依赖与构建产物目录已忽略，未纳入快照。）".to_string());
        lines.join("\n")
    }
}

fn detect_manifests(
    root: &Path,
    languages: &mut BTreeSet<String>,
    package_managers: &mut BTreeSet<String>,
    important_files: &mut Vec<String>,
    test_commands: &mut BTreeSet<String>,
) {
    let exists = |name: &str| root.join(name).exists();
    let record = |name: &str, important_files: &mut Vec<String>| {
        if exists(name) {
            important_files.push(name.to_string());
            true
        } else {
            false
        }
    };

    if record("Cargo.toml", important_files) {
        languages.insert("Rust".to_string());
        package_managers.insert("cargo".to_string());
        test_commands.insert("cargo test".to_string());
    }

    if record("package.json", important_files) {
        let is_ts = exists("tsconfig.json");
        record("tsconfig.json", important_files);
        languages.insert(if is_ts { "TypeScript" } else { "JavaScript" }.to_string());
        let pm = if exists("pnpm-lock.yaml") {
            "pnpm"
        } else if exists("yarn.lock") {
            "yarn"
        } else if exists("bun.lockb") {
            "bun"
        } else {
            "npm"
        };
        package_managers.insert(pm.to_string());
        if package_json_has_test_script(root) {
            test_commands.insert(format!("{pm} test"));
        }
    }

    if exists("pyproject.toml") || exists("requirements.txt") || exists("setup.py") {
        languages.insert("Python".to_string());
        let pm = if exists("poetry.lock") {
            "poetry"
        } else if exists("Pipfile") {
            "pipenv"
        } else {
            "pip"
        };
        package_managers.insert(pm.to_string());
        test_commands.insert("pytest".to_string());
        for f in ["pyproject.toml", "requirements.txt", "setup.py"] {
            record(f, important_files);
        }
    }

    if record("go.mod", important_files) {
        languages.insert("Go".to_string());
        package_managers.insert("go modules".to_string());
        test_commands.insert("go test ./...".to_string());
    }

    if record("pom.xml", important_files) {
        languages.insert("Java".to_string());
        package_managers.insert("maven".to_string());
        test_commands.insert("mvn test".to_string());
    }
    if exists("build.gradle") || exists("build.gradle.kts") {
        languages.insert("Java/Kotlin".to_string());
        package_managers.insert("gradle".to_string());
        test_commands.insert("gradle test".to_string());
        for f in ["build.gradle", "build.gradle.kts"] {
            record(f, important_files);
        }
    }

    if record("Gemfile", important_files) {
        languages.insert("Ruby".to_string());
        package_managers.insert("bundler".to_string());
    }
    if record("composer.json", important_files) {
        languages.insert("PHP".to_string());
        package_managers.insert("composer".to_string());
    }

    for f in ["README.md", "README", "README.rst"] {
        record(f, important_files);
    }
}

fn package_json_has_test_script(root: &Path) -> bool {
    std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .map(|v| {
            v.get("scripts")
                .and_then(|s| s.get("test"))
                .and_then(|t| t.as_str())
                .is_some()
        })
        .unwrap_or(false)
}

fn walk_for_languages(
    root: &Path,
    dir: &Path,
    depth: usize,
    visited: &mut usize,
    languages: &mut BTreeSet<String>,
    entry_points: &mut BTreeSet<String>,
) {
    if depth > SNAPSHOT_MAX_DEPTH || *visited >= SNAPSHOT_MAX_ENTRIES {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if *visited >= SNAPSHOT_MAX_ENTRIES {
            break;
        }
        *visited += 1;
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            // Skip dependency / build dirs and hidden dirs entirely (red line).
            if name.starts_with('.') || ignored_dir_names().contains(&name.as_str()) {
                continue;
            }
            walk_for_languages(
                root,
                &entry.path(),
                depth + 1,
                visited,
                languages,
                entry_points,
            );
        } else if file_type.is_file() {
            if let Some(lang) = language_for_extension(&name) {
                languages.insert(lang.to_string());
            }
            if is_entry_basename(&name) {
                if let Ok(rel) = entry.path().strip_prefix(root) {
                    entry_points.insert(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
}

fn language_for_extension(file_name: &str) -> Option<&'static str> {
    let ext = file_name.rsplit_once('.').map(|(_, ext)| ext)?;
    Some(match ext.to_ascii_lowercase().as_str() {
        "rs" => "Rust",
        "ts" | "tsx" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "py" => "Python",
        "go" => "Go",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "rb" => "Ruby",
        "php" => "PHP",
        "cs" => "C#",
        "swift" => "Swift",
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "hpp" => "C++",
        "sh" | "bash" => "Shell",
        _ => return None,
    })
}

fn is_entry_basename(file_name: &str) -> bool {
    matches!(
        file_name,
        "main.rs"
            | "lib.rs"
            | "index.ts"
            | "index.tsx"
            | "index.js"
            | "index.jsx"
            | "main.ts"
            | "main.py"
            | "app.py"
            | "__main__.py"
            | "main.go"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_is_chat_without_action() {
        let intent = classify_intent("你好");
        assert_eq!(intent.intent_type, IntentType::Chat);
        assert!(!intent.needs_action, "a greeting must not trigger action");
    }

    #[test]
    fn clear_task_needs_action() {
        let intent = classify_intent("帮我做一个登录页面");
        assert_eq!(intent.intent_type, IntentType::Task);
        assert!(intent.needs_action);
    }

    #[test]
    fn question_does_not_need_action() {
        let intent = classify_intent("这个函数是怎么工作的？");
        assert_eq!(intent.intent_type, IntentType::Question);
        assert!(!intent.needs_action);
    }

    #[test]
    fn debug_edit_review_are_classified() {
        assert_eq!(
            classify_intent("这里报错了帮我修复").intent_type,
            IntentType::Debug
        );
        assert_eq!(
            classify_intent("帮我改一下这个变量名").intent_type,
            IntentType::Edit
        );
        assert_eq!(
            classify_intent("帮我审查一下这段代码").intent_type,
            IntentType::Review
        );
    }

    #[test]
    fn classifier_is_not_constant_and_flags_vague_and_urgent() {
        // Distinct inputs → distinct intents: proves it is not a fixed value.
        assert_ne!(
            classify_intent("你好").intent_type,
            classify_intent("帮我做个网站").intent_type
        );
        // Too-short action ask → ask a minimal question first.
        assert!(classify_intent("改一下").needs_clarification);
        // Urgency is detected.
        assert_eq!(
            classify_intent("紧急！马上修复这个崩溃").urgency,
            Urgency::High
        );
    }

    #[test]
    fn snapshot_detects_stack_and_excludes_deps_and_build() {
        // Build a tiny multi-stack project under the build dir (in scope, gitignored).
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("aura_snapshot_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"").unwrap();
        std::fs::write(dir.join("package.json"), r#"{"scripts":{"test":"vitest"}}"#).unwrap();
        std::fs::write(dir.join("tsconfig.json"), "{}").unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(dir.join("src").join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.join("src").join("index.ts"), "export {};").unwrap();
        // Decoys inside ignored dirs — must NOT influence the snapshot.
        std::fs::write(dir.join("node_modules").join("evil.py"), "x = 1").unwrap();
        std::fs::write(dir.join("target").join("junk.go"), "package main").unwrap();

        let snap = scan_project_snapshot(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(snap.languages.contains(&"Rust".to_string()));
        assert!(snap.languages.contains(&"TypeScript".to_string()));
        // Red line: a .py in node_modules and a .go in target must NOT appear.
        assert!(
            !snap.languages.contains(&"Python".to_string()),
            "node_modules must be excluded from the walk"
        );
        assert!(
            !snap.languages.contains(&"Go".to_string()),
            "target must be excluded from the walk"
        );
        assert!(snap.package_managers.contains(&"cargo".to_string()));
        assert!(snap.package_managers.contains(&"pnpm".to_string()));
        assert!(snap.important_files.contains(&"Cargo.toml".to_string()));
        assert!(snap.important_files.contains(&"package.json".to_string()));
        assert!(snap.test_commands.contains(&"cargo test".to_string()));
        assert!(snap.test_commands.contains(&"pnpm test".to_string()));
        assert!(snap.entry_points.iter().any(|e| e.contains("main.rs")));
        assert!(snap.ignored_patterns.contains(&"node_modules".to_string()));
    }
}
