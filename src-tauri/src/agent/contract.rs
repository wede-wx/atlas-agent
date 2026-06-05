use crate::agent::{AgentAttachment, Message, Role, ToolCall, ToolSchema};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolUseDecision {
    NoTools,
    AskUser,
    AutoExpose,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnToolUseDecision {
    pub intent: TaskIntent,
    pub decision: ToolUseDecision,
    pub confidence: f32,
    pub expected_tools: Vec<String>,
    pub user_requested_external_lookup: bool,
    pub user_provided_external_claim: bool,
    pub explicit_no_tools: bool,
    pub freshness_required: bool,
    pub source_required: bool,
    pub reason: String,
}

/// A persisted-in-history confirmation prompt. This is derived from recent
/// conversation history instead of in-memory process state, so it works even
/// when the command layer constructs a fresh Agent for every user turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingToolUseConfirmation {
    pub original_user_input: String,
    pub decision: TurnToolUseDecision,
    pub prompt_text: String,
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
    pub decision: ToolUseDecision,
    pub confidence: f32,
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
    decide_tool_use_for_turn(user_input, has_active_skills, attachments).intent
}

pub fn decide_tool_use_for_turn(
    user_input: &str,
    has_active_skills: bool,
    attachments: &[AgentAttachment],
) -> TurnToolUseDecision {
    let text = user_input.trim();
    let explicit_no_tools = has_explicit_no_tool_instruction(text);
    let user_provided_external_claim = has_user_provided_external_claim(text);
    let user_requested_external_lookup = has_explicit_external_lookup_request(text);
    let freshness_required = mentions_freshness_or_currentness(text);
    let source_required = asks_for_sources(text);

    if has_active_skills {
        let skill_intent = match infer_non_web_intent(text) {
            TaskIntent::Chat => TaskIntent::Planning,
            intent => intent,
        };
        return decision(
            skill_intent,
            if explicit_no_tools {
                ToolUseDecision::NoTools
            } else {
                ToolUseDecision::AutoExpose
            },
            0.90,
            explicit_no_tools,
            user_provided_external_claim,
            user_requested_external_lookup,
            freshness_required,
            source_required,
            if explicit_no_tools {
                "用户显式限制工具/外部访问；即使有技能也不自动暴露工具。"
            } else {
                "已启用技能，当前轮按计划/技能任务处理。"
            },
        );
    }
    if attachments.iter().any(is_image_attachment) {
        return decision(
            TaskIntent::ImageUnderstanding,
            ToolUseDecision::NoTools,
            0.95,
            explicit_no_tools,
            user_provided_external_claim,
            user_requested_external_lookup,
            freshness_required,
            source_required,
            "当前轮包含图片附件；是否能理解图片由模型视觉能力决定，不通过联网工具兜底。",
        );
    }
    if is_simple_greeting(text) {
        return decision(
            TaskIntent::Chat,
            ToolUseDecision::NoTools,
            0.98,
            explicit_no_tools,
            user_provided_external_claim,
            user_requested_external_lookup,
            freshness_required,
            source_required,
            "简单问候，不需要工具。",
        );
    }
    if explicit_no_tools {
        return decision(
            infer_non_web_intent(text),
            ToolUseDecision::NoTools,
            0.94,
            true,
            user_provided_external_claim,
            user_requested_external_lookup,
            freshness_required,
            source_required,
            "用户明确要求不要联网/不要查/只基于已提供内容，因此不暴露工具。",
        );
    }

    if user_requested_external_lookup {
        return decision(
            TaskIntent::WebResearch,
            ToolUseDecision::AutoExpose,
            0.90,
            explicit_no_tools,
            user_provided_external_claim,
            true,
            freshness_required,
            source_required,
            "用户明确要求联网检索、官网确认、核实或获取来源。",
        );
    }

    if is_command_run_request(text) {
        return decision(
            TaskIntent::CommandRun,
            ToolUseDecision::AutoExpose,
            0.88,
            explicit_no_tools,
            user_provided_external_claim,
            user_requested_external_lookup,
            freshness_required,
            source_required,
            "用户要求运行命令、测试或验证，需要命令/验证工具。",
        );
    }

    if is_code_edit_request(text) {
        return decision(
            TaskIntent::CodeEdit,
            ToolUseDecision::AutoExpose,
            0.86,
            explicit_no_tools,
            user_provided_external_claim,
            user_requested_external_lookup,
            freshness_required,
            source_required,
            "用户要求修改、修复、创建或写入代码/文件，需要读写和验证工具。",
        );
    }

    if is_file_read_request(text) {
        return decision(
            TaskIntent::FileRead,
            ToolUseDecision::AutoExpose,
            0.84,
            explicit_no_tools,
            user_provided_external_claim,
            user_requested_external_lookup,
            freshness_required,
            source_required,
            "用户要求读取/查看本地项目或文件，需要只读文件工具。",
        );
    }

    if is_browser_automation_request(text) {
        return decision(
            TaskIntent::BrowserAutomation,
            ToolUseDecision::AutoExpose,
            0.82,
            explicit_no_tools,
            user_provided_external_claim,
            user_requested_external_lookup,
            freshness_required,
            source_required,
            "用户要求浏览器打开、点击、下载或登录等交互，需要浏览器自动化工具。",
        );
    }

    if user_provided_external_claim {
        return decision(
            TaskIntent::Chat,
            ToolUseDecision::NoTools,
            0.84,
            explicit_no_tools,
            true,
            false,
            freshness_required,
            source_required,
            "用户是在提供自己已查到/看到的信息，而不是要求 Atlas 再次联网。",
        );
    }

    if freshness_required || source_required {
        return decision(
            TaskIntent::WebResearch,
            ToolUseDecision::AskUser,
            0.62,
            explicit_no_tools,
            user_provided_external_claim,
            false,
            freshness_required,
            source_required,
            "问题可能需要当前/官方/来源信息，但用户没有明确授权联网；应先询问是否联网核实。",
        );
    }

    if is_planning_request(text) {
        return decision(
            TaskIntent::Planning,
            ToolUseDecision::AutoExpose,
            0.78,
            explicit_no_tools,
            user_provided_external_claim,
            user_requested_external_lookup,
            freshness_required,
            source_required,
            "用户要求计划、拆解或继续任务，需要计划/任务工具。",
        );
    }

    decision(
        TaskIntent::Chat,
        ToolUseDecision::NoTools,
        0.72,
        explicit_no_tools,
        user_provided_external_claim,
        user_requested_external_lookup,
        freshness_required,
        source_required,
        "普通对话，不需要工具。",
    )
}

pub fn build_tool_exposure_plan(
    intent: TaskIntent,
    tools_enabled: bool,
    policy_advertises_tools: bool,
    tool_error_budget_exhausted: bool,
    standalone_guidance_mode: bool,
) -> ToolExposurePlan {
    build_tool_exposure_plan_from_decision(
        &TurnToolUseDecision {
            intent,
            decision: ToolUseDecision::AutoExpose,
            confidence: 1.0,
            expected_tools: expected_tools_for_intent(intent),
            user_requested_external_lookup: false,
            user_provided_external_claim: false,
            explicit_no_tools: false,
            freshness_required: false,
            source_required: false,
            reason: "legacy exposure plan".to_string(),
        },
        tools_enabled,
        policy_advertises_tools,
        tool_error_budget_exhausted,
        standalone_guidance_mode,
    )
}

pub fn build_tool_exposure_plan_from_decision(
    decision: &TurnToolUseDecision,
    tools_enabled: bool,
    policy_advertises_tools: bool,
    tool_error_budget_exhausted: bool,
    standalone_guidance_mode: bool,
) -> ToolExposurePlan {
    let wants_tools = matches!(decision.decision, ToolUseDecision::AutoExpose)
        && (!decision.expected_tools.is_empty() || matches!(decision.intent, TaskIntent::Planning));
    let hidden_reason = if !tools_enabled && wants_tools {
        Some("tools_disabled".to_string())
    } else if !policy_advertises_tools && wants_tools {
        Some("policy_hides_tools".to_string())
    } else if tool_error_budget_exhausted && wants_tools {
        Some("tool_error_budget_exhausted".to_string())
    } else if standalone_guidance_mode && wants_tools {
        Some("standalone_guidance_mode".to_string())
    } else if matches!(decision.decision, ToolUseDecision::AskUser) {
        Some("ask_user_before_tools".to_string())
    } else if matches!(decision.decision, ToolUseDecision::NoTools) && decision.explicit_no_tools {
        Some("explicit_no_tools".to_string())
    } else {
        None
    };

    ToolExposurePlan {
        intent: decision.intent,
        decision: decision.decision,
        confidence: decision.confidence,
        advertise_tools: wants_tools
            && tools_enabled
            && policy_advertises_tools
            && !tool_error_budget_exhausted
            && !standalone_guidance_mode,
        expected_tools: decision.expected_tools.clone(),
        hidden_reason,
    }
}

pub fn expected_tools_for_intent(intent: TaskIntent) -> Vec<String> {
    let tools: &[&str] = match intent {
        TaskIntent::Chat => &[],
        TaskIntent::WebResearch => &[
            "search_web",
            "fetch_web_page",
            "open_web_search",
            "get_github_trending",
        ],
        TaskIntent::ImageUnderstanding => &[],
        TaskIntent::FileRead => &["read_file", "list_directory", "search_files", "file_info"],
        TaskIntent::CodeEdit => &[
            "read_file",
            "list_directory",
            "search_files",
            "file_info",
            "edit_file",
            "write_file",
            "prepare_file_write",
            "create_directory",
            "run_verify",
            "git_status",
            "git_diff",
        ],
        TaskIntent::CommandRun => &["prepare_command", "run_command", "run_verify"],
        TaskIntent::BrowserAutomation => &["browser_automation", "open_web_search"],
        TaskIntent::Planning => &[
            "create_plan",
            "create_plan_task",
            "update_plan_task",
            "list_plan_tasks",
            "set_active_plan_task",
        ],
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
    if trimmed.starts_with("<tool_call") {
        return true;
    }
    if trimmed.starts_with('{') {
        let head = trimmed.chars().take(256).collect::<String>();
        return head.contains("\"tool_call") || head.contains("\"tool_calls");
    }
    false
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
    while let Some(start) = rest.find("<tool_call") {
        let after_open_start = &rest[start..];
        let Some(open_end) = after_open_start.find('>') else {
            break;
        };
        let after_start = &after_open_start[open_end + 1..];
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

    let function_equals = extract_xml_function_equals(body);
    let name = function_equals
        .or_else(|| extract_xml_tag(body, "name"))
        .or_else(|| extract_xml_tag(body, "tool"))
        .or_else(|| extract_xml_tag(body, "function"))?;
    if !is_safe_tool_name(&name) {
        return None;
    }
    let arguments = if let Some(args_raw) =
        extract_xml_tag(body, "arguments").or_else(|| extract_xml_tag(body, "args"))
    {
        serde_json::from_str::<serde_json::Value>(&args_raw)
            .unwrap_or_else(|_| serde_json::json!({ "query": args_raw }))
    } else {
        parse_xml_parameter_tags(body)
    };
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

fn extract_xml_function_equals(body: &str) -> Option<String> {
    let start = body.find("<function=")? + "<function=".len();
    let end = body[start..].find('>')? + start;
    let name = body[start..end].trim().trim_matches('"').trim_matches('\'');
    (!name.is_empty()).then(|| name.to_string())
}

fn parse_xml_parameter_tags(body: &str) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let mut rest = body;
    while let Some(start) = rest.find("<parameter") {
        let after = &rest[start + "<parameter".len()..];
        let Some(close_rel) = after.find("</parameter>") else {
            break;
        };
        let segment = after[..close_rel].trim();
        if let Some((key, value)) = parse_parameter_segment(segment) {
            map.insert(key, coerce_parameter_value(&value));
        }
        rest = &after[close_rel + "</parameter>".len()..];
    }
    serde_json::Value::Object(map)
}

fn parse_parameter_segment(segment: &str) -> Option<(String, String)> {
    let segment = segment.trim();
    if let Some(rest) = segment.strip_prefix('=') {
        let rest = rest.trim();
        if let Some((key, value)) = rest.split_once('>') {
            return Some((normalize_parameter_key(key), value.trim().to_string()));
        }
        if let Some((key, value)) = rest.split_once('=') {
            return Some((normalize_parameter_key(key), value.trim().to_string()));
        }
        if !rest.is_empty() {
            return Some((normalize_parameter_key(rest), String::new()));
        }
    }

    if let Some((tag_part, value)) = segment.split_once('>') {
        if let Some(name) = extract_parameter_name_attr(tag_part) {
            return Some((normalize_parameter_key(&name), value.trim().to_string()));
        }
    } else if let Some(name) = extract_parameter_name_attr(segment) {
        return Some((normalize_parameter_key(&name), String::new()));
    }
    None
}

fn extract_parameter_name_attr(tag_part: &str) -> Option<String> {
    let name_pos = tag_part.find("name=")?;
    let after = &tag_part[name_pos + "name=".len()..];
    let quote = after.chars().next().unwrap_or(' ');
    if quote == '"' || quote == '\'' {
        let after_quote = &after[1..];
        let end = after_quote.find(quote).unwrap_or(after_quote.len());
        Some(after_quote[..end].to_string())
    } else {
        Some(
            after
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('>')
                .to_string(),
        )
    }
}

fn normalize_parameter_key(key: &str) -> String {
    key.trim().trim_matches('"').trim_matches('\'').to_string()
}

fn coerce_parameter_value(value: &str) -> serde_json::Value {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        serde_json::Value::Bool(true)
    } else if trimmed.eq_ignore_ascii_case("false") {
        serde_json::Value::Bool(false)
    } else if let Ok(number) = trimmed.parse::<i64>() {
        serde_json::Value::Number(number.into())
    } else if let Ok(number) = trimmed.parse::<f64>() {
        serde_json::Number::from_f64(number)
            .map(serde_json::Value::Number)
            .unwrap_or_else(|| serde_json::Value::String(trimmed.to_string()))
    } else {
        serde_json::Value::String(trimmed.to_string())
    }
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
        "web_search" | "search" | "browser_search" | "web.search" => "search_web",
        "fetch_url" | "web_fetch" | "fetch" | "read_url" => "fetch_web_page",
        "open_search" | "open_browser_search" => "open_web_search",
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
            &[
                "numResults",
                "num_results",
                "maxResults",
                "max_results",
                "count",
            ],
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
    for alias in [
        "numResults",
        "num_results",
        "maxResults",
        "max_results",
        "count",
    ] {
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
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
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

fn decision(
    intent: TaskIntent,
    tool_decision: ToolUseDecision,
    confidence: f32,
    explicit_no_tools: bool,
    user_provided_external_claim: bool,
    user_requested_external_lookup: bool,
    freshness_required: bool,
    source_required: bool,
    reason: &str,
) -> TurnToolUseDecision {
    TurnToolUseDecision {
        intent,
        decision: tool_decision,
        confidence,
        expected_tools: expected_tools_for_intent(intent),
        user_requested_external_lookup,
        user_provided_external_claim,
        explicit_no_tools,
        freshness_required,
        source_required,
        reason: reason.to_string(),
    }
}

pub fn pending_tool_use_confirmation_from_history(
    history: &[Message],
) -> Option<PendingToolUseConfirmation> {
    let mut last_non_system_index = None;
    for (index, message) in history.iter().enumerate().rev() {
        if !matches!(&message.role, Role::System) && !message.content.trim().is_empty() {
            last_non_system_index = Some(index);
            break;
        }
    }
    let assistant_index = last_non_system_index?;
    let assistant = &history[assistant_index];
    if !matches!(&assistant.role, Role::Assistant)
        || !is_tool_confirmation_prompt(&assistant.content)
    {
        return None;
    }

    let user = history[..assistant_index].iter().rev().find(|message| {
        matches!(&message.role, Role::User) && !message.content.trim().is_empty()
    })?;
    let mut decision = decide_tool_use_for_turn(&user.content, false, &user.attachments);
    if !matches!(decision.intent, TaskIntent::WebResearch) {
        return None;
    }
    if matches!(decision.decision, ToolUseDecision::NoTools) {
        decision.decision = ToolUseDecision::AskUser;
        decision.expected_tools = expected_tools_for_intent(TaskIntent::WebResearch);
    }
    Some(PendingToolUseConfirmation {
        original_user_input: user.content.clone(),
        decision,
        prompt_text: assistant.content.clone(),
    })
}

pub fn is_tool_use_confirmation_reply(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || has_explicit_no_tool_instruction(trimmed) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let compact = trimmed
        .chars()
        .filter(|ch| !ch.is_whitespace() && !matches!(ch, '。' | '！' | '!' | '.' | ',' | '，'))
        .collect::<String>();
    let short_confirmations = [
        "好",
        "好的",
        "可以",
        "行",
        "嗯",
        "确认",
        "继续",
        "查",
        "查吧",
        "去查",
        "查一下",
        "联网",
        "搜",
        "搜吧",
        "允许",
        "同意",
        "yes",
        "y",
        "ok",
        "okay",
        "go",
        "continue",
        "confirm",
        "do it",
    ];
    if compact.chars().count() <= 12
        && short_confirmations
            .iter()
            .any(|item| compact == *item || lower == *item)
    {
        return true;
    }
    contains_any(
        trimmed,
        &[
            "可以联网",
            "允许联网",
            "那你查",
            "那就查",
            "你查吧",
            "去查吧",
            "继续查",
            "开始查",
            "帮我查",
            "帮我联网",
            "yes, search",
            "please search",
            "go ahead",
            "do it",
        ],
    )
}

pub fn decision_from_pending_tool_confirmation(
    pending: &PendingToolUseConfirmation,
) -> TurnToolUseDecision {
    let mut decision = pending.decision.clone();
    decision.intent = TaskIntent::WebResearch;
    decision.decision = ToolUseDecision::AutoExpose;
    decision.confidence = decision.confidence.max(0.92);
    decision.expected_tools = expected_tools_for_intent(TaskIntent::WebResearch);
    decision.user_requested_external_lookup = true;
    decision.reason = format!(
        "用户已确认允许联网核实；继续处理上一轮待确认问题：{}",
        pending.original_user_input
    );
    decision
}

pub fn confirmed_tool_user_input(
    pending: &PendingToolUseConfirmation,
    confirmation_reply: &str,
) -> String {
    format!(
        "用户已确认允许联网/工具执行。请继续处理上一轮待确认问题：{}\n\n用户确认语：{}",
        pending.original_user_input,
        confirmation_reply.trim()
    )
}

fn is_tool_confirmation_prompt(text: &str) -> bool {
    contains_any(
        text,
        &[
            "需要联网核实",
            "要我现在查吗",
            "是否联网",
            "允许联网",
            "要不要我联网",
            "要不要我查",
            "need to verify online",
            "should i search",
            "do you want me to search",
        ],
    )
}

fn has_explicit_no_tool_instruction(text: &str) -> bool {
    contains_any(
        text,
        &[
            "不要联网",
            "不用联网",
            "不要上网",
            "不用上网",
            "不要搜索",
            "不用搜索",
            "不要查",
            "不用查",
            "不要用工具",
            "不用工具",
            "只根据我发的内容",
            "只基于我提供",
            "根据我提供",
            "根据我发的内容",
            "do not browse",
            "don't browse",
            "do not search",
            "no web",
            "no tools",
            "without tools",
        ],
    )
}

fn has_user_provided_external_claim(text: &str) -> bool {
    contains_any(
        text,
        &[
            "我在网上查到",
            "我在网上看到",
            "我在官网看到",
            "官网说",
            "官方说",
            "网上有人说",
            "以下是我查到",
            "我查到了",
            "我搜到",
            "i found online",
            "i saw online",
            "the website says",
        ],
    ) && !has_explicit_external_lookup_request(text)
}

fn has_explicit_external_lookup_request(text: &str) -> bool {
    if contains_any(
        text,
        &[
            "你上网查",
            "帮我上网查",
            "上网查",
            "联网查",
            "帮我联网",
            "搜一下",
            "搜索一下",
            "帮我搜",
            "去官网确认",
            "官网确认",
            "官方确认",
            "找来源",
            "给出处",
            "公开资料",
            "打开这个网址",
            "读取这个网页",
            "web search",
            "search web",
            "browse",
            "look up",
            "search online",
            "check the official",
            "official source",
        ],
    ) {
        return true;
    }

    let ambiguous_lookup = contains_any(
        text,
        &[
            "查一下",
            "帮我查",
            "查询",
            "查查",
            "核实",
            "验证真假",
            "确认真假",
            "确认一下",
            "verify",
            "fact check",
            "confirm whether",
        ],
    );
    ambiguous_lookup
        && (mentions_web_context(text)
            || mentions_freshness_or_currentness(text)
            || asks_for_sources(text))
}

fn mentions_web_context(text: &str) -> bool {
    contains_any(
        text,
        &[
            "官网",
            "官方",
            "网页",
            "网址",
            "链接",
            "网上",
            "互联网",
            "公开资料",
            "文档",
            "发布说明",
            "GitHub Trending",
            "github trending",
            "url",
            "website",
            "web page",
            "official",
            "docs",
        ],
    )
}

fn mentions_freshness_or_currentness(text: &str) -> bool {
    contains_any(
        text,
        &[
            "最新",
            "现在",
            "今天",
            "最近",
            "官网",
            "官方说明",
            "最新版本",
            "latest",
            "current",
            "today",
            "recent",
            "official",
        ],
    )
}

fn asks_for_sources(text: &str) -> bool {
    contains_any(
        text,
        &["来源", "出处", "引用", "给我链接", "提供链接", "citation"],
    ) || text.to_ascii_lowercase().contains("source url")
        || text.to_ascii_lowercase().contains("sources")
}

fn infer_non_web_intent(text: &str) -> TaskIntent {
    if is_command_run_request(text) {
        TaskIntent::CommandRun
    } else if is_code_edit_request(text) {
        TaskIntent::CodeEdit
    } else if is_file_read_request(text) {
        TaskIntent::FileRead
    } else if is_browser_automation_request(text) {
        TaskIntent::BrowserAutomation
    } else if is_planning_request(text) {
        TaskIntent::Planning
    } else {
        TaskIntent::Chat
    }
}

fn is_command_run_request(text: &str) -> bool {
    contains_any(
        text,
        &[
            "运行",
            "执行",
            "命令",
            "终端",
            "测试",
            "验证",
            "跑一下",
            "run",
            "execute",
            "command",
            "terminal",
            "test",
            "verify",
        ],
    )
}

fn is_code_edit_request(text: &str) -> bool {
    contains_any(
        text,
        &[
            "修改",
            "修复",
            "改成",
            "改为",
            "编辑",
            "写入",
            "创建",
            "删除",
            "代码",
            "bug",
            "补丁",
            "写一个",
            "做一个",
            "html",
            "网页",
            "edit",
            "fix",
            "write",
            "create",
            "delete",
            "code",
            "patch",
        ],
    )
}

fn is_file_read_request(text: &str) -> bool {
    contains_any(
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
    )
}

fn is_browser_automation_request(text: &str) -> bool {
    contains_any(
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
    )
}

fn is_planning_request(text: &str) -> bool {
    contains_any(
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
    )
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
        let decision =
            decide_tool_use_for_turn("你上网查查 mimo 的哪个模型可以识别图片", false, &[]);
        assert_eq!(decision.intent, TaskIntent::WebResearch);
        assert_eq!(decision.decision, ToolUseDecision::AutoExpose);
        assert!(decision
            .expected_tools
            .iter()
            .any(|tool| tool == "search_web"));
    }

    #[test]
    fn user_provided_online_claim_does_not_auto_search() {
        let decision =
            decide_tool_use_for_turn("我在网上查到 MiMo 支持图片识别，你帮我解释下", false, &[]);
        assert_eq!(decision.intent, TaskIntent::Chat);
        assert_eq!(decision.decision, ToolUseDecision::NoTools);
    }

    #[test]
    fn user_asks_to_verify_online_claim_searches() {
        let decision = decide_tool_use_for_turn(
            "我在网上查到 MiMo 支持图片识别，你帮我核实一下真假",
            false,
            &[],
        );
        assert_eq!(decision.intent, TaskIntent::WebResearch);
        assert_eq!(decision.decision, ToolUseDecision::AutoExpose);
    }

    #[test]
    fn freshness_without_lookup_asks_user() {
        let decision = decide_tool_use_for_turn("MiMo 最新模型是什么？", false, &[]);
        assert_eq!(decision.intent, TaskIntent::WebResearch);
        assert_eq!(decision.decision, ToolUseDecision::AskUser);
        let plan = build_tool_exposure_plan_from_decision(&decision, true, true, false, false);
        assert!(!plan.advertise_tools);
        assert_eq!(plan.hidden_reason.as_deref(), Some("ask_user_before_tools"));
    }

    #[test]
    fn no_network_instruction_blocks_search() {
        let decision = decide_tool_use_for_turn("不要联网，根据我发的内容总结", false, &[]);
        assert_eq!(decision.decision, ToolUseDecision::NoTools);
        assert!(decision.explicit_no_tools);
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
    fn normalizer_maps_count_to_limit() {
        let (call, changes) = normalize_tool_call(
            ToolCall {
                id: "1".to_string(),
                name: "web.search".to_string(),
                arguments: serde_json::json!({"query": "mimo", "count": 5}),
            },
            ToolCallSource::TextXml,
        );
        assert_eq!(call.name, "search_web");
        assert_eq!(call.arguments["limit"], 5);
        assert!(changes
            .iter()
            .any(|change| change.field == "count" && change.to == "limit"));
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

    #[test]
    fn xml_like_function_equals_parameter_equals_syntax_is_parsed() {
        let calls = parse_pseudo_tool_calls(
            Some(
                r#"<tool_call>
<function=web_search>
<parameter=query=小米 MiMo 模型 图片识别 多模态 能力 官方 说明</parameter>
<parameter=numResults>5</parameter>
</function>
</tool_call>"#,
            ),
            false,
            ToolCallSource::StreamingText,
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "search_web");
        assert_eq!(
            calls[0].arguments["query"],
            "小米 MiMo 模型 图片识别 多模态 能力 官方 说明"
        );
        assert_eq!(calls[0].arguments["limit"], 5);
    }

    #[test]
    fn xml_like_parameter_tag_body_syntax_is_parsed() {
        let calls = parse_pseudo_tool_calls(
            Some(
                r#"<tool_call type="function">
<function=web_search>
<parameter=query>MiMo 模型 图片识别 多模态 vision</parameter>
<parameter=count>4</parameter>
</function>
</tool_call>"#,
            ),
            false,
            ToolCallSource::StreamingText,
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "search_web");
        assert_eq!(
            calls[0].arguments["query"],
            "MiMo 模型 图片识别 多模态 vision"
        );
        assert_eq!(calls[0].arguments["limit"], 4);
    }

    #[test]
    fn potential_pseudo_tool_detection_does_not_treat_plain_json_as_tool_protocol() {
        assert!(is_potential_pseudo_tool_buffer(
            "<tool_call><function=search_web></function></tool_call>"
        ));
        assert!(is_potential_pseudo_tool_buffer(
            "{\"tool_calls\":[{\"name\":\"search_web\"}]}"
        ));
        assert!(!is_potential_pseudo_tool_buffer(
            "{\n  \"answer\": \"普通 JSON 回复\"\n}"
        ));
        assert!(!is_potential_pseudo_tool_buffer(
            "{\"ok\":true,\"message\":\"done\"}"
        ));
    }

    #[test]
    fn confirmation_reply_resumes_pending_web_research() {
        let history = vec![
            Message::plain(Role::User, "MiMo 最新模型是什么？"),
            Message::plain(Role::Assistant, "这个问题可能需要联网核实，要我现在查吗？"),
        ];
        let pending = pending_tool_use_confirmation_from_history(&history).unwrap();
        assert!(is_tool_use_confirmation_reply("好"));
        let decision = decision_from_pending_tool_confirmation(&pending);
        assert_eq!(decision.intent, TaskIntent::WebResearch);
        assert_eq!(decision.decision, ToolUseDecision::AutoExpose);
        assert!(decision
            .expected_tools
            .iter()
            .any(|tool| tool == "search_web"));
    }

    #[test]
    fn explicit_web_lookup_wins_over_file_terms() {
        let decision =
            decide_tool_use_for_turn("你上网查一下 package.json 里这个依赖的最新版本", false, &[]);
        assert_eq!(decision.intent, TaskIntent::WebResearch);
        assert_eq!(decision.decision, ToolUseDecision::AutoExpose);
    }

    #[test]
    fn expanded_expected_tools_cover_runtime_paths() {
        let web = expected_tools_for_intent(TaskIntent::WebResearch);
        assert!(web.iter().any(|tool| tool == "open_web_search"));
        assert!(web.iter().any(|tool| tool == "get_github_trending"));
        let command = expected_tools_for_intent(TaskIntent::CommandRun);
        assert!(command.iter().any(|tool| tool == "prepare_command"));
        let planning = expected_tools_for_intent(TaskIntent::Planning);
        assert!(planning.iter().any(|tool| tool == "update_plan_task"));
        assert!(planning.iter().any(|tool| tool == "list_plan_tasks"));
        let edit = expected_tools_for_intent(TaskIntent::CodeEdit);
        assert!(edit.iter().any(|tool| tool == "search_files"));
        assert!(edit.iter().any(|tool| tool == "git_diff"));
    }
}
