use crate::agent::{AgentAttachment, ToolCall, ToolSchema};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskIntent {
    Chat,
    WebResearch,
    ImageUnderstanding,
    FileRead,
    CodeEdit,
    CommandRun,
    BrowserAutomation,
    Planning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PseudoToolFormat {
    Json,
    XmlLike,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisionInputFormat {
    None,
    OpenAiImageUrl,
    AnthropicImageBlock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderToolProtocolCaps {
    pub structured_tool_calls: bool,
    pub streaming_tool_calls: bool,
    pub pseudo_tool_call_format: Option<PseudoToolFormat>,
    pub supports_tool_choice: bool,
    pub supports_parallel_tools: bool,
    pub supports_tool_result_role: bool,
    pub supports_json_response_format: bool,
    pub vision_input_format: VisionInputFormat,
}

impl Default for ProviderToolProtocolCaps {
    fn default() -> Self {
        Self {
            structured_tool_calls: false,
            streaming_tool_calls: false,
            pseudo_tool_call_format: None,
            supports_tool_choice: false,
            supports_parallel_tools: false,
            supports_tool_result_role: false,
            supports_json_response_format: false,
            vision_input_format: VisionInputFormat::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExposurePlan {
    pub intent: TaskIntent,
    pub advertise_tools: bool,
    pub expected_tools: Vec<String>,
    pub hidden_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallSource {
    Standard,
    StreamingStandard,
    TextJson,
    TextXml,
    StreamingText,
    Runtime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub source: ToolCallSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolNormalizationChange {
    pub field: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelTurn {
    FinalText(String),
    ToolCalls(Vec<NormalizedToolCall>),
    InvalidToolProtocol { raw: String, reason: String },
    Empty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuntimeDiagnostic {
    ToolVisibilityDecision {
        tools_enabled: bool,
        intent: TaskIntent,
        advertised_tools: Vec<String>,
        hidden_reason: Option<String>,
    },
    ModelToolParseDiagnostic {
        returned_kind: String,
        parsed: bool,
        reason: Option<String>,
    },
    UnknownToolRequested {
        requested: String,
        nearest: Option<String>,
    },
    ToolNormalizationApplied {
        original_name: String,
        normalized_name: String,
        argument_changes: Vec<String>,
    },
}

pub fn classify_task_intent(
    user_input: &str,
    has_active_skills: bool,
    attachments: &[AgentAttachment],
) -> TaskIntent {
    let text = user_input.trim();
    if has_active_skills {
        return TaskIntent::Planning;
    }
    if attachments.iter().any(is_image_attachment) {
        return TaskIntent::ImageUnderstanding;
    }
    if is_simple_greeting(text) {
        return TaskIntent::Chat;
    }
    if contains_any(
        text,
        &[
            "上网查",
            "联网",
            "搜一下",
            "搜索",
            "查一下",
            "查资料",
            "官网",
            "官方说明",
            "最新",
            "当前",
            "网页",
            "公开资料",
            "多模态",
            "图片识别",
            "vision",
            "web",
            "search",
            "browse",
            "latest",
            "current",
        ],
    ) {
        return TaskIntent::WebResearch;
    }
    if contains_any(
        text,
        &[
            "运行", "执行", "命令", "终端", "测试", "验证", "run", "execute", "command",
            "terminal", "test", "verify",
        ],
    ) {
        return TaskIntent::CommandRun;
    }
    if contains_any(
        text,
        &[
            "修改", "修复", "改成", "改为", "编辑", "写入", "创建", "删除", "代码", "bug", "edit",
            "fix", "write", "create", "delete", "code",
        ],
    ) {
        return TaskIntent::CodeEdit;
    }
    if contains_any(
        text,
        &[
            "读取",
            "查看",
            "文件",
            "目录",
            "项目",
            "package.json",
            "read",
            "file",
            "folder",
            "directory",
            "project",
        ],
    ) {
        return TaskIntent::FileRead;
    }
    if contains_any(
        text,
        &[
            "浏览器",
            "打开网页",
            "点击",
            "下载",
            "登录",
            "browser",
            "click",
            "download",
            "login",
        ],
    ) {
        return TaskIntent::BrowserAutomation;
    }
    if contains_any(
        text,
        &[
            "计划",
            "方案",
            "拆解",
            "继续",
            "任务",
            "上下文",
            "探针",
            "plan",
            "todo",
            "task",
            "continue",
            "context",
            "probe",
        ],
    ) {
        return TaskIntent::Planning;
    }
    TaskIntent::Chat
}

pub fn build_tool_exposure_plan(
    intent: TaskIntent,
    tools_enabled: bool,
    policy_advertises_tools: bool,
    tool_error_budget_exhausted: bool,
    standalone_guidance_mode: bool,
) -> ToolExposurePlan {
    let expected_tools = expected_tools_for_intent(intent);
    let wants_tools = !expected_tools.is_empty() || matches!(intent, TaskIntent::Planning);
    let hidden_reason = if !tools_enabled && wants_tools {
        Some("tools_disabled".to_string())
    } else if !policy_advertises_tools && wants_tools {
        Some("policy_hides_tools".to_string())
    } else if tool_error_budget_exhausted && wants_tools {
        Some("tool_error_budget_exhausted".to_string())
    } else if standalone_guidance_mode && wants_tools {
        Some("standalone_guidance_mode".to_string())
    } else {
        None
    };

    ToolExposurePlan {
        intent,
        advertise_tools: wants_tools
            && tools_enabled
            && policy_advertises_tools
            && !tool_error_budget_exhausted
            && !standalone_guidance_mode,
        expected_tools,
        hidden_reason,
    }
}

pub fn expected_tools_for_intent(intent: TaskIntent) -> Vec<String> {
    let tools: &[&str] = match intent {
        TaskIntent::Chat => &[],
        TaskIntent::WebResearch => &["search_web", "fetch_web_page"],
        TaskIntent::ImageUnderstanding => &[],
        TaskIntent::FileRead => &["read_file", "list_directory", "search_files"],
        TaskIntent::CodeEdit => &["read_file", "edit_file", "write_file", "run_verify"],
        TaskIntent::CommandRun => &["run_command", "run_verify"],
        TaskIntent::BrowserAutomation => &["browser_automation"],
        TaskIntent::Planning => &["create_plan", "create_plan_task", "set_active_plan_task"],
    };
    tools.iter().map(|tool| (*tool).to_string()).collect()
}

pub fn normalize_tool_call(
    mut tool_call: ToolCall,
    source: ToolCallSource,
) -> (ToolCall, Vec<ToolNormalizationChange>) {
    let original_name = tool_call.name.clone();
    let normalized_name = normalize_tool_name(&tool_call.name).to_string();
    let mut changes = Vec::new();
    if normalized_name != tool_call.name {
        changes.push(ToolNormalizationChange {
            field: "name".to_string(),
            from: tool_call.name.clone(),
            to: normalized_name.clone(),
        });
        tool_call.name = normalized_name;
    }

    let before_args = tool_call.arguments.clone();
    tool_call.arguments = normalize_tool_arguments_for_name(&tool_call.name, tool_call.arguments);
    if before_args != tool_call.arguments {
        changes.extend(argument_changes(&before_args, &tool_call.arguments));
    }

    let _normalized = NormalizedToolCall {
        id: tool_call.id.clone(),
        name: tool_call.name.clone(),
        arguments: tool_call.arguments.clone(),
        source,
    };
    if original_name != tool_call.name && !changes.iter().any(|change| change.field == "name") {
        changes.push(ToolNormalizationChange {
            field: "name".to_string(),
            from: original_name,
            to: tool_call.name.clone(),
        });
    }
    (tool_call, changes)
}

pub fn chat_response_content_for_tool_turn(
    content: Option<String>,
    tool_calls_empty: bool,
) -> Option<String> {
    if tool_calls_empty {
        content
    } else {
        None
    }
}

pub fn parse_pseudo_tool_calls(
    content: Option<&str>,
    require_tool_finish: bool,
    source: ToolCallSource,
) -> Vec<ToolCall> {
    let Some(raw) = content.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    if raw.starts_with('{') {
        return parse_json_pseudo_tool_calls(raw, require_tool_finish, source);
    }
    parse_xml_like_tool_calls(raw, source)
}

pub fn is_potential_pseudo_tool_buffer(value: &str) -> bool {
    let trimmed = value.trim_start();
    trimmed.starts_with("<tool_call")
        || trimmed.starts_with("{\"tool_call")
        || trimmed.starts_with("{\"tool_calls")
        || trimmed.starts_with("{\n")
}

fn parse_json_pseudo_tool_calls(
    raw: &str,
    require_tool_finish: bool,
    source: ToolCallSource,
) -> Vec<ToolCall> {
    if require_tool_finish && !raw.contains("tool_call") {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let mut calls = Vec::new();
    if let Some(items) = value.get("tool_calls").and_then(|value| value.as_array()) {
        for item in items {
            if let Some(call) = parse_json_pseudo_tool_call(item, calls.len(), source) {
                calls.push(call);
            }
        }
    } else if let Some(item) = value.get("tool_call") {
        if let Some(call) = parse_json_pseudo_tool_call(item, 0, source) {
            calls.push(call);
        }
    }
    calls
}

fn parse_json_pseudo_tool_call(
    value: &serde_json::Value,
    index: usize,
    source: ToolCallSource,
) -> Option<ToolCall> {
    let name = value
        .get("function")
        .and_then(|function| function.get("name"))
        .or_else(|| value.get("name"))
        .and_then(|value| value.as_str())?
        .trim();
    if !is_safe_tool_name(name) {
        return None;
    }
    let id = value
        .get("id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("pseudo_tool_call_{index}"));
    let arguments = value
        .get("function")
        .and_then(|function| function.get("arguments"))
        .or_else(|| value.get("arguments"))
        .map(parse_tool_arguments_value)
        .unwrap_or_else(|| serde_json::json!({}));
    let (call, _) = normalize_tool_call(
        ToolCall {
            id,
            name: name.to_string(),
            arguments,
        },
        source,
    );
    Some(call)
}

fn parse_xml_like_tool_calls(raw: &str, source: ToolCallSource) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    let mut rest = raw;
    while let Some(start) = rest.find("<tool_call>") {
        let after_start = &rest[start + "<tool_call>".len()..];
        let Some(end) = after_start.find("</tool_call>") else {
            break;
        };
        let body = after_start[..end].trim();
        if let Some(call) = parse_xml_like_tool_call_body(body, calls.len(), source) {
            calls.push(call);
        }
        rest = &after_start[end + "</tool_call>".len()..];
    }
    calls
}

fn parse_xml_like_tool_call_body(
    body: &str,
    index: usize,
    source: ToolCallSource,
) -> Option<ToolCall> {
    if body.starts_with('{') {
        return parse_json_pseudo_tool_call(
            &serde_json::from_str::<serde_json::Value>(body).ok()?,
            index,
            source,
        );
    }
    let name = extract_xml_tag(body, "name")
        .or_else(|| extract_xml_tag(body, "tool"))
        .or_else(|| extract_xml_tag(body, "function"))?;
    if !is_safe_tool_name(&name) {
        return None;
    }
    let args_raw = extract_xml_tag(body, "arguments")
        .or_else(|| extract_xml_tag(body, "args"))
        .unwrap_or_else(|| "{}".to_string());
    let arguments = serde_json::from_str::<serde_json::Value>(&args_raw)
        .unwrap_or_else(|_| serde_json::json!({ "query": args_raw }));
    let (call, _) = normalize_tool_call(
        ToolCall {
            id: format!("xml_tool_call_{index}"),
            name,
            arguments,
        },
        source,
    );
    Some(call)
}

fn extract_xml_tag(body: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = body.find(&open)? + open.len();
    let end = body[start..].find(&close)? + start;
    Some(body[start..end].trim().to_string())
}

fn parse_tool_arguments_value(value: &serde_json::Value) -> serde_json::Value {
    if let Some(raw) = value.as_str() {
        serde_json::from_str(raw).unwrap_or_else(|_| serde_json::json!({ "query": raw }))
    } else {
        value.clone()
    }
}

fn normalize_tool_name(name: &str) -> &str {
    match name.trim() {
        "web_search" | "search" | "browser_search" => "search_web",
        "fetch_url" | "web_fetch" => "fetch_web_page",
        other => other,
    }
}

fn normalize_tool_arguments_for_name(
    name: &str,
    mut arguments: serde_json::Value,
) -> serde_json::Value {
    if name == "search_web" {
        normalize_arg_alias(
            &mut arguments,
            &["numResults", "num_results", "maxResults", "max_results"],
            "limit",
        );
    }
    arguments
}

fn normalize_arg_alias(value: &mut serde_json::Value, aliases: &[&str], canonical: &str) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if object.contains_key(canonical) {
        for alias in aliases {
            object.remove(*alias);
        }
        return;
    }
    for alias in aliases {
        if let Some(value) = object.remove(*alias) {
            object.insert(canonical.to_string(), value);
            break;
        }
    }
}

fn argument_changes(
    before: &serde_json::Value,
    after: &serde_json::Value,
) -> Vec<ToolNormalizationChange> {
    let mut changes = Vec::new();
    let Some(before_obj) = before.as_object() else {
        return changes;
    };
    let Some(after_obj) = after.as_object() else {
        return changes;
    };
    for alias in ["numResults", "num_results", "maxResults", "max_results"] {
        if before_obj.contains_key(alias) && after_obj.contains_key("limit") {
            changes.push(ToolNormalizationChange {
                field: alias.to_string(),
                from: alias.to_string(),
                to: "limit".to_string(),
            });
        }
    }
    changes
}

fn is_safe_tool_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name.len() <= 96
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn is_simple_greeting(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let normalized = text.to_ascii_lowercase();
    let simple_greetings = [
        "hi",
        "hello",
        "hey",
        "你好",
        "您好",
        "嗨",
        "在吗",
        "早上好",
        "晚上好",
    ];
    text.chars().count() <= 16
        && simple_greetings
            .iter()
            .any(|greeting| normalized == *greeting || text == *greeting)
}

fn contains_any(content: &str, needles: &[&str]) -> bool {
    let lower = content.to_ascii_lowercase();
    needles
        .iter()
        .any(|needle| content.contains(needle) || lower.contains(&needle.to_ascii_lowercase()))
}

fn is_image_attachment(attachment: &AgentAttachment) -> bool {
    attachment.kind.eq_ignore_ascii_case("image")
        || attachment.mime.to_lowercase().starts_with("image/")
        || attachment
            .data_url
            .as_deref()
            .is_some_and(|value| value.starts_with("data:image/"))
}

pub fn advertised_tool_names(tools: Option<&[ToolSchema]>) -> Vec<String> {
    tools
        .unwrap_or_default()
        .iter()
        .map(|tool| tool.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_research_chinese_is_classified() {
        assert_eq!(
            classify_task_intent("你上网查查 mimo 的哪个模型可以识别图片", false, &[]),
            TaskIntent::WebResearch
        );
    }

    #[test]
    fn greeting_stays_chat() {
        assert_eq!(classify_task_intent("你好", false, &[]), TaskIntent::Chat);
    }

    #[test]
    fn file_read_request_is_classified_and_exposes_file_tools() {
        let intent = classify_task_intent("读取这个项目里的 package.json", false, &[]);
        assert_eq!(intent, TaskIntent::FileRead);
        let plan = build_tool_exposure_plan(intent, true, true, false, false);
        assert!(plan.advertise_tools);
        assert!(plan.expected_tools.iter().any(|tool| tool == "read_file"));
        assert!(plan
            .expected_tools
            .iter()
            .any(|tool| tool == "search_files"));
    }

    #[test]
    fn image_question_does_not_auto_expose_web_tools() {
        let attachment = AgentAttachment {
            id: "img-1".to_string(),
            name: "screen.png".to_string(),
            mime: "image/png".to_string(),
            size: 10,
            kind: "image".to_string(),
            data_url: Some("data:image/png;base64,ZmFrZQ==".to_string()),
            text_preview: None,
            island_package_id: None,
        };
        let intent = classify_task_intent("看看这张图有什么", false, &[attachment]);
        assert_eq!(intent, TaskIntent::ImageUnderstanding);
        let plan = build_tool_exposure_plan(intent, true, true, false, false);
        assert!(!plan.advertise_tools);
        assert!(!plan.expected_tools.iter().any(|tool| tool == "search_web"));
    }

    #[test]
    fn normalizer_maps_web_search_and_num_results() {
        let (call, changes) = normalize_tool_call(
            ToolCall {
                id: "1".to_string(),
                name: "web_search".to_string(),
                arguments: serde_json::json!({"query": "mimo", "numResults": 3}),
            },
            ToolCallSource::TextXml,
        );
        assert_eq!(call.name, "search_web");
        assert_eq!(call.arguments["limit"], 3);
        assert!(!changes.is_empty());
    }

    #[test]
    fn xml_like_tool_call_is_parsed() {
        let calls = parse_pseudo_tool_calls(
            Some(
                r#"<tool_call><name>web_search</name><arguments>{"query":"mimo","numResults":3}</arguments></tool_call>"#,
            ),
            false,
            ToolCallSource::StreamingText,
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "search_web");
        assert_eq!(calls[0].arguments["limit"], 3);
    }
}
