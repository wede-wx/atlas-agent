use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::agent::Message;

pub const AGENT_SKILL_STATE_KEY: &str = "agent_skill_states_v1";
pub const MAX_RULE_BYTES: u64 = 64 * 1024;
pub const MAX_SKILL_BYTES: u64 = 128 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub label_zh: String,
    pub description_zh: String,
    pub triggers: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub tags: Vec<String>,
    pub source: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub built_in: bool,
    #[serde(default)]
    pub pending_review: bool,
    #[serde(default)]
    pub source_kind: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub load_error: Option<String>,
    #[serde(default)]
    pub state_key: String,
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub metadata: SkillMetadata,
    pub instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillState {
    pub enabled: bool,
    #[serde(default)]
    pub pending_review: bool,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillLoadIssue {
    pub source: String,
    pub path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuleSource {
    pub kind: String,
    pub label: String,
    pub path: String,
    pub loaded: bool,
    pub bytes: usize,
    pub truncated: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuleContext {
    pub sources: Vec<AgentRuleSource>,
    pub prompt: Option<String>,
    pub project_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentContextStatus {
    pub global_rules: AgentRuleSource,
    pub project_rules: Option<AgentRuleSource>,
    pub project_root: Option<String>,
    pub skills: Vec<SkillMetadata>,
    #[serde(default)]
    pub agents: Vec<AgentProfileMetadata>,
    pub issues: Vec<SkillLoadIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfileMetadata {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub tools: Vec<String>,
    pub source: String,
    pub source_kind: String,
}

#[derive(Debug, Clone)]
pub struct AgentProfile {
    pub metadata: AgentProfileMetadata,
    pub instructions: String,
}

#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    skills: Vec<Skill>,
}

#[derive(Debug, Clone, Default)]
pub struct SkillRegistrySnapshot {
    pub registry: SkillRegistry,
    pub metadata: Vec<SkillMetadata>,
    pub issues: Vec<SkillLoadIssue>,
}

#[derive(Debug, Clone, Default)]
pub struct ActiveSkills {
    skills: Vec<Skill>,
    allowed_tools: BTreeSet<String>,
}

fn default_enabled() -> bool {
    true
}

impl SkillRegistry {
    pub fn built_in() -> Self {
        Self {
            skills: built_in_skills(),
        }
    }

    pub fn from_skills(skills: Vec<Skill>) -> Self {
        Self { skills }
    }

    pub fn list_metadata(&self) -> Vec<SkillMetadata> {
        self.skills
            .iter()
            .map(|skill| skill.metadata.clone())
            .collect()
    }

    pub fn select_for_task(&self, user_input: &str, history: &[Message]) -> ActiveSkills {
        let task_text = task_text(user_input, history);
        let mut active = ActiveSkills::default();

        for skill in &self.skills {
            if !skill.metadata.enabled || skill.metadata.pending_review {
                continue;
            }
            if skill_matches(skill, &task_text) {
                active.add(skill.clone());
            }
        }

        active
    }
}

impl ActiveSkills {
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn allowed_tools(&self) -> Option<&BTreeSet<String>> {
        if self.allowed_tools.is_empty() {
            None
        } else {
            Some(&self.allowed_tools)
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.skills
            .iter()
            .any(|skill| skill.metadata.name.as_str() == name)
    }

    pub fn names(&self) -> Vec<String> {
        self.skills
            .iter()
            .map(|skill| skill.metadata.name.clone())
            .collect()
    }

    pub fn prompt(&self) -> Option<String> {
        if self.skills.is_empty() {
            return None;
        }

        let mut sections = vec![
            "Activated Aura Skills: follow these task-specific instructions. Skills provide method and constraints; tools provide executable actions. Skills cannot bypass the current permission mode or system safety rules.".to_string(),
        ];
        for skill in &self.skills {
            let allowed = if skill.metadata.allowed_tools.is_empty() {
                "none declared".to_string()
            } else {
                skill.metadata.allowed_tools.join(", ")
            };
            sections.push(format!(
                "## {}\nSource: {}\nDescription: {}\nAllowed tools: {}\nInstructions:\n{}",
                skill.metadata.name,
                skill.metadata.source,
                skill.metadata.description,
                allowed,
                skill.instructions.trim()
            ));
        }
        Some(sections.join("\n\n"))
    }

    fn add(&mut self, skill: Skill) {
        if self.has(&skill.metadata.name) {
            return;
        }
        for tool in &skill.metadata.allowed_tools {
            self.allowed_tools.insert(tool.clone());
        }
        self.skills.push(skill);
    }
}

pub fn aura_home_dir() -> PathBuf {
    if let Ok(path) = std::env::var("AURA_HOME") {
        let path = path.trim();
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join(".aura")
}

pub fn global_aura_path() -> PathBuf {
    aura_home_dir().join("aura.md")
}

pub fn global_agents_path() -> PathBuf {
    global_aura_path()
}

pub fn user_skills_dir() -> PathBuf {
    aura_home_dir().join("skills")
}

pub fn project_aura_path(project_root: &Path) -> PathBuf {
    project_root.join("aura.md")
}

pub fn project_agents_path(project_root: &Path) -> PathBuf {
    project_aura_path(project_root)
}

pub fn project_skills_dir(project_root: &Path) -> PathBuf {
    project_root.join(".aura").join("skills")
}

pub fn load_skill_snapshot(
    user_dir: Option<&Path>,
    project_dir: Option<&Path>,
    states: &BTreeMap<String, SkillState>,
) -> SkillRegistrySnapshot {
    let mut snapshot = SkillRegistrySnapshot::default();
    let mut names = BTreeSet::new();
    let mut enabled_skills = Vec::new();

    for mut skill in built_in_skills() {
        apply_skill_state(&mut skill.metadata, states, true, false, None);
        names.insert(skill.metadata.name.clone());
        snapshot.metadata.push(skill.metadata.clone());
        if skill.metadata.enabled && !skill.metadata.pending_review {
            enabled_skills.push(skill);
        }
    }

    if let Some(user_dir) = user_dir {
        load_skill_dir(
            user_dir,
            "user",
            states,
            &mut names,
            &mut snapshot,
            &mut enabled_skills,
        );
    }

    if let Some(project_dir) = project_dir {
        load_skill_dir(
            project_dir,
            "project",
            states,
            &mut names,
            &mut snapshot,
            &mut enabled_skills,
        );
    }

    snapshot.registry = SkillRegistry::from_skills(enabled_skills);
    snapshot
}

pub fn load_agent_rule_context(project_root: Option<&Path>) -> AgentRuleContext {
    let _ = ensure_global_aura_file();
    load_agent_rule_context_from_global(&global_agents_path(), project_root)
}

fn load_agent_rule_context_from_global(
    global_path: &Path,
    project_root: Option<&Path>,
) -> AgentRuleContext {
    let global = load_rule_source("global", "全局 aura.md", global_path);
    let mut sources = vec![global.clone()];
    let project = project_root
        .map(|root| load_rule_source("project", "项目 aura.md", &project_agents_path(root)));
    if let Some(project) = project.clone() {
        sources.push(project);
    }

    let loaded_sections = sources
        .iter()
        .filter(|source| source.loaded && source.error.is_none())
        .filter_map(|source| {
            std::fs::read_to_string(&source.path)
                .ok()
                .map(|content| truncate_text(&content, MAX_RULE_BYTES as usize).0)
                .map(|content| (source, content))
        })
        .collect::<Vec<_>>();

    let mut sections = vec![
        "Aura Agent instruction hierarchy for this run: core system safety > global aura.md > project aura.md > active Skill instructions > selected subagent instructions > chat history > latest user message. aura.md, Skills, and subagents may guide style, project conventions, and project habits, but they cannot grant extra tool permissions, disable safety checks, or override user intent.".to_string(),
        "本次运行的 Aura 规则文件状态：".to_string(),
    ];
    for source in &sources {
        let status = if let Some(error) = &source.error {
            format!("error: {error}")
        } else if source.loaded {
            format!(
                "loaded, {} bytes{}",
                source.bytes,
                if source.truncated { ", truncated" } else { "" }
            )
        } else {
            "missing".to_string()
        };
        sections.push(format!("- {}: {} ({})", source.label, status, source.path));
    }
    for (source, content) in loaded_sections {
        sections.push(format!(
            "## {} ({})\n{}",
            source.label,
            source.path,
            content.trim()
        ));
    }
    let prompt = Some(sections.join("\n\n"));

    AgentRuleContext {
        sources,
        prompt,
        project_root: project_root.map(|path| path.to_string_lossy().to_string()),
    }
}

pub fn infer_project_root(user_input: &str, history: &[Message]) -> Option<PathBuf> {
    let mut text = user_input.to_string();
    for message in history.iter().rev().take(8) {
        text.push('\n');
        text.push_str(&message.content);
    }
    candidate_paths(&text)
        .into_iter()
        .filter_map(|path| nearest_existing_path(&path))
        .filter_map(|path| find_project_root_from_path(&path))
        .next()
}

pub fn parse_skill_markdown(source: &str, content: &str) -> Result<Skill, String> {
    parse_skill_markdown_with_path(source, content, None)
}

pub fn parse_skill_markdown_with_path(
    source: &str,
    content: &str,
    path: Option<&Path>,
) -> Result<Skill, String> {
    if content.len() as u64 > MAX_SKILL_BYTES {
        return Err(format!("Skill {source} 超过 128KB，拒绝加载。"));
    }
    let normalized = content.replace("\r\n", "\n");
    let mut parts = normalized.splitn(3, "---");
    let before = parts.next().unwrap_or_default().trim();
    if !before.is_empty() {
        return Err(format!("Skill {source} must start with frontmatter."));
    }
    let frontmatter = parts
        .next()
        .ok_or_else(|| format!("Skill {source} 缺少 frontmatter。"))?;
    let body = parts
        .next()
        .ok_or_else(|| format!("Skill {source} 缺少正文。"))?;

    let mut metadata = parse_frontmatter(source, frontmatter)?;
    if let Some(path) = path {
        metadata.path = Some(path.to_string_lossy().to_string());
    }
    Ok(Skill {
        metadata,
        instructions: body.trim().to_string(),
    })
}

pub fn parse_skill_states(value: Option<serde_json::Value>) -> BTreeMap<String, SkillState> {
    value
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

pub fn sanitize_skill_name(name: &str) -> Result<String, String> {
    let normalized = name
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if normalized.is_empty() {
        return Err("Skill 名称不能为空。".to_string());
    }
    if normalized.starts_with('.') || normalized.contains("..") || normalized.len() > 80 {
        return Err("Skill 名称不合法。".to_string());
    }
    Ok(normalized)
}

pub fn build_skill_markdown(
    name: &str,
    label_zh: &str,
    description: &str,
    description_zh: &str,
    triggers: &[String],
    allowed_tools: &[String],
    body: &str,
) -> Result<String, String> {
    let name = sanitize_skill_name(name)?;
    if description.trim().is_empty() {
        return Err("Skill 描述不能为空。".to_string());
    }
    if body.trim().is_empty() {
        return Err("Skill 正文不能为空。".to_string());
    }
    Ok(format!(
        "---\nname: {name}\ndescription: {}\nlabel-zh: {}\ndescription-zh: {}\ntriggers: [{}]\nallowed-tools: [{}]\ntags: [user]\n---\n\n{}\n",
        clean_yaml_scalar(description),
        clean_yaml_scalar(if label_zh.trim().is_empty() { &name } else { label_zh }),
        clean_yaml_scalar(if description_zh.trim().is_empty() { description } else { description_zh }),
        triggers
            .iter()
            .map(|item| clean_yaml_scalar(item))
            .collect::<Vec<_>>()
            .join(", "),
        allowed_tools
            .iter()
            .map(|item| clean_yaml_scalar(item))
            .collect::<Vec<_>>()
            .join(", "),
        body.trim()
    ))
}

fn built_in_skills() -> Vec<Skill> {
    Vec::new()
}

pub fn ensure_global_aura_file() -> Result<PathBuf, String> {
    let path = global_aura_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if !path.exists() {
        std::fs::write(&path, "").map_err(|error| error.to_string())?;
    }
    Ok(path)
}

pub fn built_in_agent_profiles() -> Vec<AgentProfile> {
    Vec::new()
}

pub fn list_agent_profile_metadata() -> Vec<AgentProfileMetadata> {
    built_in_agent_profiles()
        .into_iter()
        .map(|agent| agent.metadata)
        .collect()
}

pub fn get_agent_profile(name: &str) -> Option<AgentProfile> {
    let normalized = sanitize_skill_name(name).ok()?;
    built_in_agent_profiles()
        .into_iter()
        .find(|agent| agent.metadata.name == normalized)
}

fn load_skill_dir(
    base: &Path,
    source_kind: &str,
    states: &BTreeMap<String, SkillState>,
    names: &mut BTreeSet<String>,
    snapshot: &mut SkillRegistrySnapshot,
    enabled_skills: &mut Vec<Skill>,
) {
    if !base.exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(base) else {
        snapshot.issues.push(SkillLoadIssue {
            source: source_kind.to_string(),
            path: Some(base.to_string_lossy().to_string()),
            message: "无法读取 Skill 目录。".to_string(),
        });
        return;
    };
    for entry in entries.flatten() {
        let skill_path = entry.path().join("SKILL.md");
        if !skill_path.is_file() {
            continue;
        }
        let source = format!("{}:{}", source_kind, entry.file_name().to_string_lossy());
        let content = match std::fs::read_to_string(&skill_path) {
            Ok(content) => content,
            Err(error) => {
                snapshot.issues.push(SkillLoadIssue {
                    source,
                    path: Some(skill_path.to_string_lossy().to_string()),
                    message: format!("无法读取 Skill：{error}"),
                });
                continue;
            }
        };
        let mut skill = match parse_skill_markdown_with_path(&source, &content, Some(&skill_path)) {
            Ok(skill) => skill,
            Err(error) => {
                snapshot.issues.push(SkillLoadIssue {
                    source,
                    path: Some(skill_path.to_string_lossy().to_string()),
                    message: error,
                });
                continue;
            }
        };
        if names.contains(&skill.metadata.name) {
            snapshot.issues.push(SkillLoadIssue {
                source: source.clone(),
                path: Some(skill_path.to_string_lossy().to_string()),
                message: format!("Skill 名称 {} 已存在，已跳过。", skill.metadata.name),
            });
            continue;
        }
        names.insert(skill.metadata.name.clone());
        apply_skill_state(&mut skill.metadata, states, false, true, Some(source_kind));
        snapshot.metadata.push(skill.metadata.clone());
        if skill.metadata.enabled && !skill.metadata.pending_review {
            enabled_skills.push(skill);
        }
    }
}

fn apply_skill_state(
    metadata: &mut SkillMetadata,
    states: &BTreeMap<String, SkillState>,
    built_in: bool,
    default_pending_review: bool,
    source_kind: Option<&str>,
) {
    metadata.built_in = built_in;
    metadata.source_kind = source_kind.map(str::to_string).unwrap_or_else(|| {
        if built_in {
            "built-in".to_string()
        } else {
            "user".to_string()
        }
    });
    metadata.state_key = skill_state_key(metadata);
    let state = states.get(&metadata.state_key).or_else(|| {
        if metadata.source_kind == "project" {
            None
        } else {
            states.get(&metadata.name)
        }
    });
    metadata.enabled = state.map(|state| state.enabled).unwrap_or(built_in);
    metadata.pending_review = state
        .map(|state| state.pending_review)
        .unwrap_or(default_pending_review && !built_in);
    metadata.origin = state.and_then(|state| state.origin.clone());
}

pub fn skill_state_key(metadata: &SkillMetadata) -> String {
    let source = if metadata.source_kind.trim().is_empty() {
        if metadata.built_in {
            "built-in"
        } else {
            "user"
        }
    } else {
        metadata.source_kind.as_str()
    };
    match source {
        "built-in" => format!("built-in:{}", metadata.name),
        "user" | "project" => {
            let path = metadata
                .path
                .as_deref()
                .map(normalize_state_path)
                .unwrap_or_else(|| source.to_string());
            format!("{source}:{path}:{}", metadata.name)
        }
        other => format!("{other}:{}", metadata.name),
    }
}

fn normalize_state_path(path: &str) -> String {
    let path = PathBuf::from(path);
    path.parent()
        .unwrap_or(path.as_path())
        .canonicalize()
        .unwrap_or_else(|_| path.parent().unwrap_or(path.as_path()).to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn load_rule_source(kind: &str, label: &str, path: &Path) -> AgentRuleSource {
    if !path.exists() {
        return AgentRuleSource {
            kind: kind.to_string(),
            label: label.to_string(),
            path: path.to_string_lossy().to_string(),
            loaded: false,
            bytes: 0,
            truncated: false,
            error: None,
        };
    }
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_RULE_BYTES => AgentRuleSource {
            kind: kind.to_string(),
            label: label.to_string(),
            path: path.to_string_lossy().to_string(),
            loaded: true,
            bytes: metadata.len() as usize,
            truncated: true,
            error: None,
        },
        Ok(metadata) => AgentRuleSource {
            kind: kind.to_string(),
            label: label.to_string(),
            path: path.to_string_lossy().to_string(),
            loaded: true,
            bytes: metadata.len() as usize,
            truncated: false,
            error: None,
        },
        Err(error) => AgentRuleSource {
            kind: kind.to_string(),
            label: label.to_string(),
            path: path.to_string_lossy().to_string(),
            loaded: false,
            bytes: 0,
            truncated: false,
            error: Some(error.to_string()),
        },
    }
}

fn parse_frontmatter(source: &str, frontmatter: &str) -> Result<SkillMetadata, String> {
    let mut name = String::new();
    let mut description = String::new();
    let mut label_zh = String::new();
    let mut description_zh = String::new();
    let mut triggers = Vec::new();
    let mut allowed_tools = Vec::new();
    let mut tags = Vec::new();

    for line in frontmatter.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = clean_scalar(value);
        match key {
            "name" => name = value,
            "description" => description = value,
            "label-zh" | "label_zh" => label_zh = value,
            "description-zh" | "description_zh" => description_zh = value,
            "triggers" => triggers = parse_list(&value),
            "allowed-tools" | "allowed_tools" => allowed_tools = parse_list(&value),
            "tags" => tags = parse_list(&value),
            _ => {}
        }
    }

    if name.trim().is_empty() {
        return Err(format!("Skill {source} 缺少 name。"));
    }
    let name = sanitize_skill_name(&name)?;
    if description.trim().is_empty() {
        return Err(format!("Skill {source} 缺少 description。"));
    }
    if label_zh.trim().is_empty() {
        label_zh = name.clone();
    }
    if description_zh.trim().is_empty() {
        description_zh = description.clone();
    }

    Ok(SkillMetadata {
        name,
        description,
        label_zh,
        description_zh,
        triggers,
        allowed_tools,
        tags,
        source: source.to_string(),
        enabled: true,
        built_in: false,
        pending_review: false,
        source_kind: String::new(),
        path: None,
        origin: None,
        load_error: None,
        state_key: String::new(),
    })
}

fn clean_scalar(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}

fn clean_yaml_scalar(value: &str) -> String {
    let cleaned = value
        .trim()
        .replace(['\r', '\n'], " ")
        .replace('"', "'")
        .replace('[', "(")
        .replace(']', ")");
    format!("\"{cleaned}\"")
}

fn parse_list(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(clean_scalar)
        .filter(|item| !item.is_empty())
        .collect()
}

fn task_text(user_input: &str, history: &[Message]) -> String {
    let mut text = user_input.to_string();
    for message in history.iter().rev().take(4) {
        text.push('\n');
        text.push_str(&message.content);
    }
    text.to_ascii_lowercase()
}

fn skill_matches(skill: &Skill, task_text: &str) -> bool {
    matches_any(task_text, &skill.metadata.triggers)
        || matches_token(task_text, &skill.metadata.name)
        || matches_token(task_text, &skill.metadata.label_zh)
}

fn matches_any(task_text: &str, triggers: &[String]) -> bool {
    triggers.iter().any(|trigger| {
        !trigger.trim().is_empty() && task_text.contains(&trigger.to_ascii_lowercase())
    })
}

fn matches_token(task_text: &str, token: &str) -> bool {
    let token = token.trim().to_ascii_lowercase();
    !token.is_empty() && task_text.contains(&token)
}

fn candidate_paths(text: &str) -> Vec<PathBuf> {
    text.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '"' | '\''
                    | '`'
                    | '，'
                    | '。'
                    | '；'
                    | ';'
                    | '|'
                    | '<'
                    | '>'
                    | '('
                    | ')'
                    | '['
                    | ']'
            )
    })
    .map(|token| token.trim_matches(|ch: char| matches!(ch, ',' | '.' | ':' | '：' | '、')))
    .filter(|token| token.len() > 3)
    .filter(|token| token.contains(":\\") || token.contains(":/") || token.starts_with("\\\\"))
    .map(PathBuf::from)
    .collect()
}

fn nearest_existing_path(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_path_buf());
    }
    let mut current = path.parent()?;
    loop {
        if current.exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

fn find_project_root_from_path(path: &Path) -> Option<PathBuf> {
    let start = if path.is_file() {
        path.parent()?.to_path_buf()
    } else {
        path.to_path_buf()
    };
    for ancestor in start.ancestors() {
        if project_agents_path(ancestor).is_file() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn truncate_text(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_string(), false);
    }
    let mut end = max_bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{unique}"))
    }

    fn sample_skill(name: &str, trigger: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: test skill\ntriggers: [{trigger}]\nallowed-tools: [read_file]\n---\n\nFollow test skill.\n"
        )
    }

    #[test]
    fn built_in_skills_parse() {
        let registry = SkillRegistry::built_in();
        let names = registry
            .list_metadata()
            .into_iter()
            .map(|skill| skill.name)
            .collect::<Vec<_>>();
        assert!(names.is_empty());
    }

    #[test]
    fn explicit_skill_name_selects_skill_without_trigger_match() {
        let dir = unique_temp_dir("aura_skill_explicit_name");
        let skill_dir = dir.join("code-shencha");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            sample_skill("code-shencha", "rare-trigger-token"),
        )
        .unwrap();

        let mut states = BTreeMap::new();
        states.insert(
            "code-shencha".to_string(),
            SkillState {
                enabled: true,
                pending_review: false,
                origin: None,
                updated_at: 0,
            },
        );
        let snapshot = load_skill_snapshot(Some(&dir), None, &states);
        let active = snapshot
            .registry
            .select_for_task("使用 Skill「code-shencha」：检查代码质量", &[]);

        assert!(active.has("code-shencha"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn user_skill_is_disabled_until_enabled() {
        let dir = unique_temp_dir("aura_skill_disabled");
        let skill_dir = dir.join("demo-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            sample_skill("demo-skill", "demo"),
        )
        .unwrap();

        let snapshot = load_skill_snapshot(Some(&dir), None, &BTreeMap::new());
        assert!(snapshot
            .metadata
            .iter()
            .any(|skill| skill.name == "demo-skill" && !skill.enabled && skill.pending_review));
        assert!(snapshot.registry.select_for_task("demo", &[]).is_empty());

        let mut states = BTreeMap::new();
        states.insert(
            "demo-skill".to_string(),
            SkillState {
                enabled: true,
                pending_review: false,
                origin: None,
                updated_at: 0,
            },
        );
        let snapshot = load_skill_snapshot(Some(&dir), None, &states);
        assert!(snapshot
            .registry
            .select_for_task("demo", &[])
            .has("demo-skill"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn project_skill_state_is_scoped_by_project_path() {
        let project_a = unique_temp_dir("aura_project_skill_a");
        let project_b = unique_temp_dir("aura_project_skill_b");
        let skill_a = project_a.join(".aura").join("skills").join("demo-skill");
        let skill_b = project_b.join(".aura").join("skills").join("demo-skill");
        std::fs::create_dir_all(&skill_a).unwrap();
        std::fs::create_dir_all(&skill_b).unwrap();
        std::fs::write(skill_a.join("SKILL.md"), sample_skill("demo-skill", "demo")).unwrap();
        std::fs::write(skill_b.join("SKILL.md"), sample_skill("demo-skill", "demo")).unwrap();

        let snapshot_a = load_skill_snapshot(
            None,
            Some(&project_a.join(".aura").join("skills")),
            &BTreeMap::new(),
        );
        let key_a = snapshot_a
            .metadata
            .iter()
            .find(|skill| skill.name == "demo-skill")
            .unwrap()
            .state_key
            .clone();

        let mut states = BTreeMap::new();
        states.insert(
            "demo-skill".to_string(),
            SkillState {
                enabled: true,
                pending_review: false,
                origin: Some("legacy".to_string()),
                updated_at: 1,
            },
        );
        states.insert(
            key_a,
            SkillState {
                enabled: true,
                pending_review: false,
                origin: Some("project-a".to_string()),
                updated_at: 2,
            },
        );

        let enabled_a =
            load_skill_snapshot(None, Some(&project_a.join(".aura").join("skills")), &states)
                .metadata
                .into_iter()
                .find(|skill| skill.name == "demo-skill")
                .unwrap();
        let enabled_b =
            load_skill_snapshot(None, Some(&project_b.join(".aura").join("skills")), &states)
                .metadata
                .into_iter()
                .find(|skill| skill.name == "demo-skill")
                .unwrap();

        assert!(enabled_a.enabled);
        assert!(!enabled_a.pending_review);
        assert!(!enabled_b.enabled);
        assert!(enabled_b.pending_review);

        let _ = std::fs::remove_dir_all(project_a);
        let _ = std::fs::remove_dir_all(project_b);
    }

    #[test]
    fn duplicate_skill_name_is_skipped() {
        let root = unique_temp_dir("aura_skill_duplicate");
        let user_dir = root.join("user");
        let project_dir = root.join("project");
        let user_skill_dir = user_dir.join("duplicate-skill");
        let project_skill_dir = project_dir.join("duplicate-skill");
        std::fs::create_dir_all(&user_skill_dir).unwrap();
        std::fs::create_dir_all(&project_skill_dir).unwrap();
        std::fs::write(
            user_skill_dir.join("SKILL.md"),
            sample_skill("duplicate-skill", "user-trigger"),
        )
        .unwrap();
        std::fs::write(
            project_skill_dir.join("SKILL.md"),
            sample_skill("duplicate-skill", "project-trigger"),
        )
        .unwrap();

        let snapshot = load_skill_snapshot(Some(&user_dir), Some(&project_dir), &BTreeMap::new());
        assert!(snapshot
            .issues
            .iter()
            .any(|issue| issue.message.contains("已存在")));
        assert_eq!(
            snapshot
                .metadata
                .iter()
                .filter(|skill| skill.name == "duplicate-skill")
                .count(),
            1
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn infers_project_root_only_when_aura_md_exists() {
        let root = unique_temp_dir("aura_project_agents");
        let nested = root.join("src");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("aura.md"), "project rules").unwrap();
        let file = nested.join("main.ts");
        std::fs::write(&file, "").unwrap();

        let message = format!("修改 {}", file.to_string_lossy());
        let inferred = infer_project_root(&message, &[]).unwrap();
        assert_eq!(inferred, root);

        let _ = std::fs::remove_dir_all(inferred);
    }

    #[test]
    fn missing_aura_rules_still_report_source_status() {
        let root = unique_temp_dir("aura_no_rules");
        std::fs::create_dir_all(&root).unwrap();
        let missing_global = root.join("missing-global-aura.md");

        let context = load_agent_rule_context_from_global(&missing_global, None);

        assert!(context.prompt.as_deref().is_some_and(|prompt| prompt
            .contains("missing-global-aura.md")
            && prompt.contains("missing")));
        assert_eq!(context.sources.len(), 1);
        assert!(!context.sources[0].loaded);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn global_aura_md_enters_rule_prompt() {
        let root = unique_temp_dir("aura_global_rule_prompt");
        std::fs::create_dir_all(&root).unwrap();
        let global = root.join("aura.md");
        std::fs::write(&global, "Use global convention B.").unwrap();

        let context = load_agent_rule_context_from_global(&global, None);

        assert!(context
            .prompt
            .as_deref()
            .is_some_and(|prompt| prompt.contains("Use global convention B.")));
        assert!(context
            .sources
            .iter()
            .any(|source| source.kind == "global" && source.loaded));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn project_aura_md_enters_rule_prompt() {
        let root = unique_temp_dir("aura_project_rule_prompt");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("aura.md"), "Use project convention A.").unwrap();

        let context = load_agent_rule_context(Some(&root));

        assert!(context
            .prompt
            .as_deref()
            .is_some_and(|prompt| prompt.contains("Use project convention A.")));
        assert!(context
            .sources
            .iter()
            .any(|source| source.kind == "project" && source.loaded));

        let _ = std::fs::remove_dir_all(root);
    }
}
