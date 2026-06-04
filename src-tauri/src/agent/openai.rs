use crate::agent::{
    AgentAttachment, AgentError, AgentEvent, ChatCompletionOptions, ChatResponse, LLMClient,
    Message, ModelTokenUsage, Role, ToolSchema,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use tokio::time::{timeout, Duration};
use uuid::Uuid;

#[cfg(not(test))]
const MODEL_WAIT_HEARTBEAT_SECS: u64 = 5;
#[cfg(test)]
const MODEL_WAIT_HEARTBEAT_SECS: u64 = 1;

pub struct OpenAIClient {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    auth_header: Option<String>,
    /// 当 provider 明确不支持视觉（M1.2 静态黑名单，例如 xiaomi-mimo）时设为 `Some(false)`，
    /// 序列化用户消息时会剥掉 image part 并附一条 system note。
    /// 默认 `None` 表示未知，保留现有行为（不剥）。M5 capability matrix 完成后替换。
    vision_supported: Option<bool>,
    /// Patch 5：capability matrix 给出"该模型不支持 tool_calls"时设为 `Some(false)`，
    /// 调 chat_completion 时会丢掉 tools 入参并发系统提示。`None` 表示未知，保留现行。
    tool_calls_supported: Option<bool>,
    /// Capability matrix 上 json_mode 的标志。只有调用方显式要求 JSON 输出时，
    /// 才会据此决定是否往请求里塞 `response_format`。`None` 表示未知。
    /// 不要让这个字段单独把任意 chat 请求都改成 JSON-only —— 那会破坏所有现有用法。
    json_mode_supported: Option<bool>,
}

impl OpenAIClient {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
            model,
            auth_header: None,
            vision_supported: None,
            tool_calls_supported: None,
            json_mode_supported: None,
        }
    }

    pub fn with_tool_call_support(mut self, supported: Option<bool>) -> Self {
        self.tool_calls_supported = supported;
        self
    }

    pub fn with_json_mode_supported(mut self, supported: Option<bool>) -> Self {
        self.json_mode_supported = supported;
        self
    }

    fn maybe_drop_tools(&self, tools: Option<Vec<ToolSchema>>) -> Option<Vec<ToolSchema>> {
        if matches!(self.tool_calls_supported, Some(false)) {
            if let Some(t) = &tools {
                if !t.is_empty() {
                    eprintln!(
                        "[openai-client] dropping {} tool(s); provider/model declared no tool_calls support",
                        t.len()
                    );
                }
            }
            return None;
        }
        tools
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url.trim().trim_end_matches('/').to_string();
        self
    }

    pub fn with_auth_header(mut self, auth_header: Option<String>) -> Self {
        self.auth_header = auth_header;
        self
    }

    pub fn with_vision_support(mut self, supported: Option<bool>) -> Self {
        self.vision_supported = supported;
        self
    }

    fn apply_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.api_key.trim().is_empty() {
            return request;
        }
        match self.auth_header.as_deref() {
            Some("api-key") => request.header("api-key", &self.api_key),
            Some("x-api-key") => request.header("x-api-key", &self.api_key),
            _ => request.header("Authorization", format!("Bearer {}", self.api_key)),
        }
    }

    fn response_format_for(
        &self,
        options: ChatCompletionOptions,
        allow_response_format: bool,
    ) -> Option<OpenAIResponseFormat> {
        (options.wants_json_object()
            && allow_response_format
            && matches!(self.json_mode_supported, Some(true)))
        .then_some(OpenAIResponseFormat {
            format_type: "json_object",
        })
    }

    async fn chat_completion_with_json_permission(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
        options: ChatCompletionOptions,
        allow_response_format: bool,
    ) -> Result<ChatResponse, AgentError> {
        let mut current_tools = self.maybe_drop_tools(tools);
        let mut current_allow_response_format = allow_response_format;
        let mut last_error: Option<AgentError> = None;

        for _ in 0..4 {
            let response_format = self.response_format_for(options, current_allow_response_format);
            let request_messages =
                with_json_mode_guidance(messages.clone(), options, response_format.is_some());
            let openai_messages = to_openai_messages(request_messages, self.vision_supported);
            let openai_tools = openai_tools_from_schemas(current_tools.clone());
            let request = OpenAIRequest {
                model: self.model.clone(),
                messages: openai_messages,
                stream: None,
                stream_options: None,
                tools: openai_tools,
                response_format,
            };

            let response = self
                .apply_auth(
                    self.client
                        .post(format!("{}/chat/completions", self.base_url))
                        .json(&request),
                )
                .send()
                .await
                .map_err(|e| AgentError::Llm(e.to_string()))?;

            let status = response.status();
            if status.is_success() {
                return parse_openai_response(response).await;
            }

            let details = openai_error_details(response).await;
            let tried_response_format = request.response_format.is_some();
            if tried_response_format && is_json_mode_compat_error(status, &details) {
                current_allow_response_format = false;
                last_error = Some(openai_request_error(status, details));
                continue;
            }
            if current_tools
                .as_ref()
                .is_some_and(|items| !items.is_empty())
                && is_tool_compat_error(status, &details)
            {
                current_tools = None;
                last_error = Some(openai_request_error(status, details));
                continue;
            }

            return Err(openai_request_error(status, details));
        }

        Err(last_error.unwrap_or_else(|| {
            AgentError::Llm(
                "OpenAI-compatible request failed after compatibility retries".to_string(),
            )
        }))
    }
}

/// 反序列化时把 `null`、缺失字段和正常数组都当作 `Vec<T>` 处理。
///
/// OpenAI-compatible 上游（典型如 MiMo / 部分 Qwen 网关）在没有工具调用时会返回
/// `{"tool_calls": null}`；裸 `#[serde(default)]` 只能吃"字段缺失"，遇到 `null`
/// 会报 `invalid type: null, expected a sequence`，把整个 run 干崩。
fn null_or_missing_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::<Vec<T>>::deserialize(deserializer).map(Option::unwrap_or_default)
}

// M1.2 静态视觉黑名单已删除（Patch 5）。视觉支持改由 capability matrix 决定，
// 调用方应通过 `agent::capabilities::resolve_capabilities` 拿 `vision` 字段，
// 再调 `OpenAIClient::with_vision_support(Some(cap.vision))`。

fn openai_tools_from_schemas(tools: Option<Vec<ToolSchema>>) -> Option<Vec<OpenAITool>> {
    tools.map(|tools| {
        tools
            .into_iter()
            .map(|t| OpenAITool {
                tool_type: "function".to_string(),
                function: OpenAIFunction {
                    name: t.name,
                    description: t.description,
                    parameters: t.parameters,
                },
            })
            .collect()
    })
}

#[derive(Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<OpenAIStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAITool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<OpenAIResponseFormat>,
}

#[derive(Clone, Serialize)]
struct OpenAIResponseFormat {
    #[serde(rename = "type")]
    format_type: &'static str,
}

#[derive(Serialize)]
struct OpenAIStreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct OpenAIMessage {
    role: String,
    content: OpenAIMessageContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum OpenAIMessageContent {
    Text(String),
    Parts(Vec<OpenAIContentPart>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OpenAIContentPart {
    Text { text: String },
    ImageUrl { image_url: OpenAIImageUrl },
}

#[derive(Serialize)]
struct OpenAIImageUrl {
    url: String,
}

#[derive(Serialize)]
struct OpenAITool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAIFunction,
}

#[derive(Serialize)]
struct OpenAIFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
    usage: Option<OpenAIUsage>,
}

#[derive(Deserialize)]
struct OpenAIErrorEnvelope {
    error: Option<OpenAIErrorBody>,
}

#[derive(Deserialize)]
struct OpenAIErrorBody {
    message: Option<String>,
    #[serde(rename = "type")]
    error_type: Option<String>,
    code: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct OpenAIChoice {
    message: OpenAIResponseMessage,
    finish_reason: String,
}

#[derive(Deserialize)]
struct OpenAIStreamChunk {
    choices: Vec<OpenAIStreamChoice>,
    usage: Option<OpenAIUsage>,
}

#[derive(Deserialize)]
struct OpenAIStreamChoice {
    delta: OpenAIStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct OpenAIStreamDelta {
    content: Option<String>,
    #[serde(default, deserialize_with = "null_or_missing_vec")]
    tool_calls: Vec<OpenAIStreamToolCall>,
}

#[derive(Deserialize)]
struct OpenAIStreamToolCall {
    index: usize,
    id: Option<String>,
    function: Option<OpenAIStreamFunctionCall>,
}

#[derive(Deserialize)]
struct OpenAIStreamFunctionCall {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct OpenAIResponseMessage {
    content: Option<String>,
    #[serde(default, deserialize_with = "null_or_missing_vec")]
    tool_calls: Vec<OpenAIToolCall>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    total_tokens: Option<i64>,
}

#[derive(Deserialize)]
struct OpenAIToolCall {
    id: String,
    function: OpenAIFunctionCall,
}

#[derive(Deserialize)]
struct OpenAIFunctionCall {
    name: String,
    arguments: String,
}

#[async_trait]
impl LLMClient for OpenAIClient {
    async fn chat_completion(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
    ) -> Result<ChatResponse, AgentError> {
        self.chat_completion_with_json_permission(
            messages,
            tools,
            ChatCompletionOptions::default(),
            true,
        )
        .await
    }

    async fn chat_completion_with_options(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
        options: ChatCompletionOptions,
    ) -> Result<ChatResponse, AgentError> {
        self.chat_completion_with_json_permission(messages, tools, options, true)
            .await
    }

    async fn chat_completion_stream(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
        event_tx: Option<Sender<crate::agent::AgentEvent>>,
    ) -> Result<ChatResponse, AgentError> {
        self.chat_completion_stream_with_options(
            messages,
            tools,
            event_tx,
            ChatCompletionOptions::default(),
        )
        .await
    }

    async fn chat_completion_stream_with_options(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
        event_tx: Option<Sender<crate::agent::AgentEvent>>,
        options: ChatCompletionOptions,
    ) -> Result<ChatResponse, AgentError> {
        let Some(event_tx) = event_tx else {
            return self
                .chat_completion_with_options(messages, tools, options)
                .await;
        };
        let tools = self.maybe_drop_tools(tools);
        let message_id = format!("msg_{}", Uuid::new_v4());
        let mut response_started = false;

        let response_format = self.response_format_for(options, true);
        let request_messages =
            with_json_mode_guidance(messages.clone(), options, response_format.is_some());
        let openai_messages = to_openai_messages(request_messages, self.vision_supported);
        let tools_for_fallback = tools.clone();
        let openai_tools = openai_tools_from_schemas(tools);
        let visible_summary = visible_work_summary(
            &openai_messages,
            openai_tools
                .as_ref()
                .is_some_and(|tools: &Vec<OpenAITool>| !tools.is_empty()),
        );

        let request = OpenAIRequest {
            model: self.model.clone(),
            messages: openai_messages,
            stream: Some(true),
            stream_options: Some(OpenAIStreamOptions {
                include_usage: true,
            }),
            tools: openai_tools,
            response_format,
        };

        emit_model_wait(
            &event_tx,
            "正在分析任务",
            Some(model_wait_detail(
                &visible_summary,
                "请求已发送，正在等待模型服务响应。",
            )),
        )
        .await?;

        let request = self
            .apply_auth(
                self.client
                    .post(format!("{}/chat/completions", self.base_url))
                    .json(&request),
            )
            .send();
        let mut request = Box::pin(request);
        let mut request_waited_secs = 0_u64;
        let response = loop {
            match timeout(Duration::from_secs(MODEL_WAIT_HEARTBEAT_SECS), &mut request).await {
                Ok(result) => break result.map_err(|e| AgentError::Llm(e.to_string()))?,
                Err(_) => {
                    request_waited_secs += MODEL_WAIT_HEARTBEAT_SECS;
                    emit_model_wait(
                        &event_tx,
                        "仍在等待模型连接",
                        Some(model_wait_detail(
                            &visible_summary,
                            &format!("已等待 {request_waited_secs} 秒，还没有收到模型服务响应。"),
                        )),
                    )
                    .await?;
                }
            }
        };

        let status = response.status();
        if !status.is_success() {
            let details = openai_error_details(response).await;
            if is_stream_compat_error(status, &details)
                || is_tool_compat_error(status, &details)
                || is_json_mode_compat_error(status, &details)
            {
                let json_mode_failed = is_json_mode_compat_error(status, &details);
                let reason = if json_mode_failed {
                    "json_mode_not_supported"
                } else if is_tool_compat_error(status, &details) {
                    "tool_calling_not_supported"
                } else {
                    "streaming_not_supported"
                };
                event_tx
                    .send(AgentEvent::ResponseFallbackStarted {
                        message_id: message_id.clone(),
                        reason: reason.to_string(),
                    })
                    .await
                    .map_err(|_| AgentError::Cancelled)?;
                emit_model_wait(
                    &event_tx,
                    "切换模型兼容模式",
                    Some(model_wait_detail(
                        &visible_summary,
                        if json_mode_failed {
                            "当前模型服务不支持 JSON mode 参数，已改用普通请求并保留 JSON 输出指令。"
                        } else {
                            "当前模型服务不支持 Aura 的完整流式或工具调用参数，已改用兼容请求继续生成回复。"
                        },
                    )),
                )
                .await?;
                let response = self
                    .chat_completion_with_json_permission(
                        messages,
                        tools_for_fallback,
                        options,
                        !json_mode_failed,
                    )
                    .await
                    .map_err(|error| {
                        if agent_error_is_tool_compat_error(&error) {
                            AgentError::Llm(format!(
                                "{}；兼容重试也失败，请换用支持工具调用的模型，或先关闭需要本地工具的任务。",
                                error
                            ))
                        } else {
                            error
                        }
                    })?;
                if let Some(content) = &response.content {
                    event_tx
                        .send(AgentEvent::ResponseCompleted {
                            message_id,
                            content: content.clone(),
                        })
                        .await
                        .map_err(|_| AgentError::Cancelled)?;
                }
                return Ok(response);
            }
            return Err(openai_streaming_request_error(status, details));
        }

        let mut content = String::new();
        let mut finish_reason = "stop".to_string();
        let mut usage: Option<ModelTokenUsage> = None;
        let mut tool_builders: Vec<StreamToolBuilder> = Vec::new();
        let mut tool_progress_announced = false;
        let mut tool_argument_chars = 0usize;
        let mut next_tool_argument_notice = 4096usize;
        let mut buffer = String::new();
        let mut non_sse_payload = String::new();
        let mut stream = response.bytes_stream();
        let mut stream_waited_secs = 0_u64;
        loop {
            let chunk = match timeout(
                Duration::from_secs(MODEL_WAIT_HEARTBEAT_SECS),
                stream.next(),
            )
            .await
            {
                Ok(Some(chunk)) => {
                    stream_waited_secs = 0;
                    chunk
                }
                Ok(None) => break,
                Err(_) => {
                    stream_waited_secs += MODEL_WAIT_HEARTBEAT_SECS;
                    let detail = if response_started || tool_progress_announced {
                        format!("已等待 {stream_waited_secs} 秒，模型流暂时没有新片段。")
                    } else {
                        format!("已等待 {stream_waited_secs} 秒，还没有收到首个流式片段。")
                    };
                    emit_model_wait(
                        &event_tx,
                        "仍在等待模型输出",
                        Some(model_wait_detail(&visible_summary, &detail)),
                    )
                    .await?;
                    continue;
                }
            };
            let chunk = chunk.map_err(|e| AgentError::Llm(e.to_string()))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();
                if line.is_empty() || line.starts_with(':') {
                    continue;
                }
                let Some(data) = line.strip_prefix("data:").map(str::trim) else {
                    if line.starts_with('{') || !non_sse_payload.is_empty() {
                        non_sse_payload.push_str(&line);
                    }
                    continue;
                };
                if data == "[DONE]" {
                    continue;
                }
                let parsed = match serde_json::from_str::<OpenAIStreamChunk>(data) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        if !response_started && content.is_empty() {
                            event_tx
                                .send(AgentEvent::ResponseFallbackStarted {
                                    message_id: message_id.clone(),
                                    reason: "stream_parse_failed".to_string(),
                                })
                                .await
                                .map_err(|_| AgentError::Cancelled)?;
                            emit_model_wait(
                                &event_tx,
                                "切换模型兼容模式",
                                Some(model_wait_detail(
                                    &visible_summary,
                                    "当前模型服务返回了非标准流式片段，已改用普通请求继续生成回复。",
                                )),
                            )
                            .await?;
                            let response = self
                                .chat_completion_with_options(messages, tools_for_fallback, options)
                                .await?;
                            if let Some(content) = &response.content {
                                event_tx
                                    .send(AgentEvent::ResponseCompleted {
                                        message_id,
                                        content: content.clone(),
                                    })
                                    .await
                                    .map_err(|_| AgentError::Cancelled)?;
                            }
                            return Ok(response);
                        }
                        return Err(AgentError::Llm(format!(
                            "OpenAI stream parse failed: {error}"
                        )));
                    }
                };
                if let Some(parsed_usage) = parsed.usage {
                    usage = openai_usage_to_model_usage(parsed_usage);
                }
                for choice in parsed.choices {
                    if let Some(reason) = choice.finish_reason {
                        finish_reason = reason;
                    }
                    if let Some(delta) = choice.delta.content {
                        if !delta.is_empty() {
                            content.push_str(&delta);
                            if !response_started {
                                event_tx
                                    .send(AgentEvent::ResponseStarted {
                                        message_id: message_id.clone(),
                                    })
                                    .await
                                    .map_err(|_| AgentError::Cancelled)?;
                                response_started = true;
                            }
                            event_tx
                                .send(AgentEvent::ResponseDelta {
                                    message_id: message_id.clone(),
                                    content: delta,
                                })
                                .await
                                .map_err(|_| AgentError::Cancelled)?;
                        }
                    }
                    for tool_call in choice.delta.tool_calls {
                        if !tool_progress_announced {
                            event_tx
                                .send(AgentEvent::OperationPreparing {
                                    label: "正在准备本地操作".to_string(),
                                    detail: Some("模型正在生成工具参数。".to_string()),
                                    tool_name: None,
                                    bytes: None,
                                })
                                .await
                                .map_err(|_| AgentError::Cancelled)?;
                            event_tx
                                .send(AgentEvent::Thinking {
                                    content: "模型正在准备本地操作。".to_string(),
                                })
                                .await
                                .map_err(|_| AgentError::Cancelled)?;
                            tool_progress_announced = true;
                        }
                        ensure_tool_builder(&mut tool_builders, tool_call.index);
                        if let Some(builder) = tool_builders.get_mut(tool_call.index) {
                            if let Some(id) = tool_call.id {
                                builder.id = id;
                            }
                            if let Some(function) = tool_call.function {
                                if let Some(name) = function.name {
                                    builder.name.push_str(&name);
                                }
                                if let Some(arguments) = function.arguments {
                                    tool_argument_chars += arguments.chars().count();
                                    builder.arguments.push_str(&arguments);
                                    if tool_argument_chars >= next_tool_argument_notice {
                                        let tool_name = (!builder.name.trim().is_empty())
                                            .then(|| builder.name.clone());
                                        event_tx
                                            .send(AgentEvent::OperationProgress {
                                                label: "正在生成本地操作内容".to_string(),
                                                detail: Some(format!(
                                                    "已生成约 {} KB 工具参数。",
                                                    tool_argument_chars / 1024
                                                )),
                                                tool_name: tool_name.clone(),
                                                bytes: Some(tool_argument_chars),
                                            })
                                            .await
                                            .map_err(|_| AgentError::Cancelled)?;
                                        event_tx
                                            .send(AgentEvent::Thinking {
                                                content: format!(
                                                    "正在生成本地操作内容，约 {} KB。",
                                                    tool_argument_chars / 1024
                                                ),
                                            })
                                            .await
                                            .map_err(|_| AgentError::Cancelled)?;
                                        next_tool_argument_notice += 8192;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let raw_fallback_payload = format!("{}{}", non_sse_payload, buffer.trim());
        if !response_started
            && content.is_empty()
            && tool_builders
                .iter()
                .all(|builder| builder.name.trim().is_empty())
        {
            if let Ok(response) = parse_openai_response_text(raw_fallback_payload.trim()) {
                if let Some(content) = &response.content {
                    event_tx
                        .send(AgentEvent::ResponseStarted {
                            message_id: message_id.clone(),
                        })
                        .await
                        .map_err(|_| AgentError::Cancelled)?;
                    event_tx
                        .send(AgentEvent::ResponseCompleted {
                            message_id,
                            content: content.clone(),
                        })
                        .await
                        .map_err(|_| AgentError::Cancelled)?;
                }
                return Ok(response);
            }

            event_tx
                .send(AgentEvent::ResponseFallbackStarted {
                    message_id: message_id.clone(),
                    reason: "empty_stream_response".to_string(),
                })
                .await
                .map_err(|_| AgentError::Cancelled)?;
            emit_model_wait(
                &event_tx,
                "切换模型兼容模式",
                Some(model_wait_detail(
                    &visible_summary,
                    "当前模型服务没有返回标准流式内容，已改用普通请求继续生成回复。",
                )),
            )
            .await?;
            let response = self
                .chat_completion_with_options(messages, tools_for_fallback, options)
                .await?;
            if let Some(content) = &response.content {
                event_tx
                    .send(AgentEvent::ResponseCompleted {
                        message_id,
                        content: content.clone(),
                    })
                    .await
                    .map_err(|_| AgentError::Cancelled)?;
            }
            return Ok(response);
        }

        let tool_calls = tool_builders
            .into_iter()
            .filter(|builder| !builder.name.trim().is_empty())
            .enumerate()
            .map(|(index, builder)| crate::agent::ToolCall {
                id: if builder.id.is_empty() {
                    format!("stream_tool_{index}")
                } else {
                    builder.id
                },
                name: builder.name,
                arguments: serde_json::from_str(&builder.arguments)
                    .unwrap_or(serde_json::json!({})),
            })
            .collect::<Vec<_>>();

        if response_started {
            event_tx
                .send(AgentEvent::ResponseCompleted {
                    message_id,
                    content: content.clone(),
                })
                .await
                .map_err(|_| AgentError::Cancelled)?;
        }

        Ok(ChatResponse {
            content: if content.is_empty() {
                None
            } else {
                Some(content)
            },
            tool_calls,
            finish_reason,
            usage,
        })
    }
}

async fn parse_openai_response(response: reqwest::Response) -> Result<ChatResponse, AgentError> {
    let openai_response: OpenAIResponse = response
        .json()
        .await
        .map_err(|e| AgentError::Llm(e.to_string()))?;

    openai_response_to_chat(openai_response)
}

fn parse_openai_response_text(raw: &str) -> Result<ChatResponse, AgentError> {
    if raw.trim().is_empty() {
        return Err(AgentError::Llm("模型没有返回内容。".to_string()));
    }
    let openai_response: OpenAIResponse =
        serde_json::from_str(raw).map_err(|e| AgentError::Llm(e.to_string()))?;
    openai_response_to_chat(openai_response)
}

fn openai_response_to_chat(openai_response: OpenAIResponse) -> Result<ChatResponse, AgentError> {
    let choice = openai_response
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| AgentError::Llm("OpenAI 兼容接口没有返回内容。".to_string()))?;

    let mut tool_calls: Vec<crate::agent::ToolCall> = choice
        .message
        .tool_calls
        .into_iter()
        .map(|tc| crate::agent::ToolCall {
            id: tc.id,
            name: tc.function.name,
            arguments: normalize_tool_arguments(
                serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::json!({})),
            ),
        })
        .collect();
    if tool_calls.is_empty() {
        tool_calls = fallback_text_tool_calls(
            choice.message.content.as_deref(),
            &choice.finish_reason,
        );
    }

    Ok(ChatResponse {
        content: choice.message.content,
        tool_calls,
        finish_reason: choice.finish_reason,
        usage: openai_response.usage.and_then(openai_usage_to_model_usage),
    })
}

fn fallback_text_tool_calls(content: Option<&str>, finish_reason: &str) -> Vec<crate::agent::ToolCall> {
    let finish = finish_reason.trim().to_ascii_lowercase();
    if !matches!(
        finish.as_str(),
        "tool_calls" | "tool_call" | "function_call" | "tool_use"
    ) {
        return Vec::new();
    }
    let Some(content) = content.map(str::trim).filter(|value| value.starts_with('{')) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };

    let mut calls = Vec::new();
    if let Some(items) = value.get("tool_calls").and_then(|value| value.as_array()) {
        for item in items {
            if let Some(call) = fallback_text_tool_call(item, calls.len()) {
                calls.push(call);
            }
        }
    } else if let Some(item) = value.get("tool_call") {
        if let Some(call) = fallback_text_tool_call(item, 0) {
            calls.push(call);
        }
    }
    calls
}

fn fallback_text_tool_call(
    value: &serde_json::Value,
    index: usize,
) -> Option<crate::agent::ToolCall> {
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
        .unwrap_or_else(|| format!("text_tool_call_{index}"));
    let arguments = value
        .get("function")
        .and_then(|function| function.get("arguments"))
        .or_else(|| value.get("arguments"))
        .map(parse_tool_arguments_value)
        .map(normalize_tool_arguments)
        .unwrap_or_else(|| serde_json::json!({}));
    Some(crate::agent::ToolCall {
        id,
        name: name.to_string(),
        arguments,
    })
}

fn parse_tool_arguments_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => serde_json::from_str(text).unwrap_or_else(|_| {
            serde_json::json!({
                "value": text
            })
        }),
        serde_json::Value::Object(_) => value.clone(),
        _ => serde_json::json!({}),
    }
}

fn normalize_tool_arguments(mut value: serde_json::Value) -> serde_json::Value {
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    if object.contains_key("limit") {
        return value;
    }
    for alias in ["numResults", "num_results", "maxResults", "max_results"] {
        if let Some(limit) = object.get(alias).cloned() {
            object.insert("limit".to_string(), limit);
            break;
        }
    }
    value
}

fn is_safe_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 96
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

async fn openai_error_details(response: reqwest::Response) -> String {
    let raw = response.text().await.unwrap_or_default();
    let parsed = serde_json::from_str::<OpenAIErrorEnvelope>(&raw).ok();
    let mut details = parsed
        .and_then(|payload| payload.error)
        .map(|error| {
            let mut message = error
                .message
                .unwrap_or_else(|| "Unknown upstream error".to_string());
            if let Some(error_type) = error.error_type {
                message = format!("{} ({})", message, error_type);
            }
            if let Some(code) = error.code {
                message = format!("{} [code={}]", message, code);
            }
            message
        })
        .unwrap_or(raw);

    if details.trim().is_empty() {
        details = "Unknown upstream error".to_string();
    }
    details
}

fn openai_request_error(status: StatusCode, details: String) -> AgentError {
    AgentError::Llm(format!(
        "OpenAI-compatible request failed with status {}: {}",
        status, details
    ))
}

fn openai_streaming_request_error(status: StatusCode, details: String) -> AgentError {
    AgentError::Llm(format!(
        "OpenAI-compatible streaming request failed with status {}: {}",
        status, details
    ))
}

fn is_stream_compat_error(status: StatusCode, details: &str) -> bool {
    if !status.is_client_error() {
        return false;
    }
    let lower = details.to_ascii_lowercase();
    (lower.contains("stream") || lower.contains("include_usage"))
        && (lower.contains("not support")
            || lower.contains("unsupported")
            || lower.contains("unrecognized")
            || lower.contains("unknown")
            || lower.contains("invalid")
            || lower.contains("unexpected")
            || lower.contains("extra input")
            || lower.contains("additional propert")
            || lower.contains("not permitted")
            || lower.contains("不支持"))
}

fn is_tool_compat_error(status: StatusCode, details: &str) -> bool {
    if !status.is_client_error() {
        return false;
    }
    let lower = details.to_ascii_lowercase();
    (lower.contains("tool")
        || lower.contains("function_call")
        || lower.contains("function calling")
        || lower.contains("函数调用")
        || lower.contains("工具调用"))
        && (lower.contains("not support")
            || lower.contains("unsupported")
            || lower.contains("unrecognized")
            || lower.contains("unknown")
            || lower.contains("invalid")
            || lower.contains("unexpected")
            || lower.contains("extra input")
            || lower.contains("additional propert")
            || lower.contains("not permitted")
            || lower.contains("不支持"))
}

fn is_json_mode_compat_error(status: StatusCode, details: &str) -> bool {
    if !status.is_client_error() {
        return false;
    }
    let lower = details.to_ascii_lowercase();
    (lower.contains("response_format")
        || lower.contains("json mode")
        || lower.contains("json_object")
        || lower.contains("json_schema"))
        && (lower.contains("not support")
            || lower.contains("unsupported")
            || lower.contains("unrecognized")
            || lower.contains("unknown")
            || lower.contains("invalid")
            || lower.contains("unexpected")
            || lower.contains("extra input")
            || lower.contains("additional propert")
            || lower.contains("not permitted")
            || lower.contains("不支持"))
}

fn agent_error_is_tool_compat_error(error: &AgentError) -> bool {
    match error {
        AgentError::Llm(details) => is_tool_compat_error(StatusCode::BAD_REQUEST, details),
        _ => false,
    }
}

fn openai_usage_to_model_usage(usage: OpenAIUsage) -> Option<ModelTokenUsage> {
    let input_tokens = usage.prompt_tokens.unwrap_or(0).max(0);
    let output_tokens = usage.completion_tokens.unwrap_or(0).max(0);
    let total_tokens = usage
        .total_tokens
        .unwrap_or(input_tokens + output_tokens)
        .max(input_tokens + output_tokens);
    (input_tokens > 0 || output_tokens > 0 || total_tokens > 0).then_some(ModelTokenUsage {
        input_tokens,
        output_tokens,
        total_tokens,
    })
}

#[derive(Default)]
struct StreamToolBuilder {
    id: String,
    name: String,
    arguments: String,
}

fn ensure_tool_builder(builders: &mut Vec<StreamToolBuilder>, index: usize) {
    while builders.len() <= index {
        builders.push(StreamToolBuilder::default());
    }
}

fn to_openai_messages(
    messages: Vec<Message>,
    vision_supported: Option<bool>,
) -> Vec<OpenAIMessage> {
    messages
        .into_iter()
        .map(|message| {
            // P0-2: untrusted (tool/external) messages render fenced as data.
            let content = message.model_content();
            OpenAIMessage {
                role: match message.role {
                    Role::System => "system".to_string(),
                    Role::User => "user".to_string(),
                    Role::Assistant => "assistant".to_string(),
                },
                content: openai_content_from_message(
                    content,
                    message.attachments,
                    vision_supported,
                ),
            }
        })
        .collect()
}

fn with_json_mode_guidance(
    mut messages: Vec<Message>,
    options: ChatCompletionOptions,
    provider_response_format_enabled: bool,
) -> Vec<Message> {
    if !options.wants_json_object() {
        return messages;
    }
    let guidance = if provider_response_format_enabled {
        "Structured JSON output requested. Return a valid JSON object only; do not wrap it in Markdown."
    } else {
        "Structured JSON output requested. The selected provider/model is not using provider JSON mode for this request, so return a valid JSON object in plain text only; do not wrap it in Markdown."
    };
    messages.push(Message::plain(Role::System, guidance));
    messages
}

fn is_image_attachment(attachment: &AgentAttachment) -> bool {
    let mime = attachment.mime.to_lowercase();
    attachment.kind.eq_ignore_ascii_case("image") || mime.starts_with("image/")
}

fn openai_content_from_message(
    content: String,
    attachments: Vec<AgentAttachment>,
    vision_supported: Option<bool>,
) -> OpenAIMessageContent {
    // M1.2：当 provider 明确不支持视觉，剥掉所有 image part 并附一条 system note
    // 让用户在前端能看到原因；M5 capability matrix 完成后替换为结构化判定。
    let strip_images = matches!(vision_supported, Some(false));
    let mut stripped_image_names: Vec<String> = Vec::new();

    let mut parts = Vec::new();
    if !content.trim().is_empty() {
        parts.push(OpenAIContentPart::Text { text: content });
    }
    for attachment in attachments {
        if is_image_attachment(&attachment) {
            if strip_images {
                stripped_image_names.push(attachment.name.clone());
                continue;
            }
            if let Some(url) = attachment
                .data_url
                .filter(|value| value.starts_with("data:image/"))
            {
                parts.push(OpenAIContentPart::ImageUrl {
                    image_url: OpenAIImageUrl { url },
                });
            }
        } else if let Some(text) = attachment
            .text_preview
            .filter(|value| !value.trim().is_empty())
        {
            parts.push(OpenAIContentPart::Text {
                text: format!("[附件：{}]\n{}", attachment.name, text),
            });
        }
    }

    if !stripped_image_names.is_empty() {
        eprintln!(
            "[openai-client] stripped {} image attachment(s) because provider declared no vision support",
            stripped_image_names.len()
        );
        let note = format!(
            "[系统提示] 当前模型未声明视觉能力，已剥离 {} 张图片附件：{}。请切换支持视觉的模型再发送图片。",
            stripped_image_names.len(),
            stripped_image_names.join("、")
        );
        parts.push(OpenAIContentPart::Text { text: note });
    }

    if parts.is_empty() {
        OpenAIMessageContent::Text(String::new())
    } else if parts.len() == 1 {
        match parts.pop().expect("one content part") {
            OpenAIContentPart::Text { text } => OpenAIMessageContent::Text(text),
            part => OpenAIMessageContent::Parts(vec![part]),
        }
    } else {
        OpenAIMessageContent::Parts(parts)
    }
}

fn openai_content_text(content: &OpenAIMessageContent) -> String {
    match content {
        OpenAIMessageContent::Text(text) => text.clone(),
        OpenAIMessageContent::Parts(parts) => parts
            .iter()
            .filter_map(|part| match part {
                OpenAIContentPart::Text { text } => Some(text.as_str()),
                OpenAIContentPart::ImageUrl { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn visible_work_summary(messages: &[OpenAIMessage], tools_available: bool) -> String {
    let user_request = messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| compact_status_text(&openai_content_text(&message.content), 140))
        .filter(|content| !content.trim().is_empty())
        .unwrap_or_else(|| "继续当前对话".to_string());

    let action_line = if tools_available {
        "正在判断是否需要读取文件、修改文件或运行本地命令。"
    } else {
        "正在组织直接回复，不会执行本地工具。"
    };
    format!(
        "已收到：{user_request}\n{action_line}\n下一步：模型返回后会显示回复，或显示准备执行的本地操作。"
    )
}

fn model_wait_detail(summary: &str, wait_status: &str) -> String {
    format!("{summary}\n{wait_status}")
}

fn compact_status_text(value: &str, max_chars: usize) -> String {
    let mut normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() > max_chars {
        normalized = normalized.chars().take(max_chars).collect::<String>();
        normalized.push_str("...");
    }
    normalized
}

async fn emit_model_wait(
    event_tx: &Sender<AgentEvent>,
    label: &str,
    detail: Option<String>,
) -> Result<(), AgentError> {
    event_tx
        .send(AgentEvent::Thinking {
            content: detail.clone().unwrap_or_else(|| label.to_string()),
        })
        .await
        .map_err(|_| AgentError::Cancelled)?;
    event_tx
        .send(AgentEvent::OperationProgress {
            label: label.to_string(),
            detail,
            tool_name: None,
            bytes: None,
        })
        .await
        .map_err(|_| AgentError::Cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{
        AgentAttachment, AgentEvent, ChatCompletionOptions, LLMClient, Message, Role,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc;
    use tokio::time::{sleep, timeout, Duration};

    async fn read_http_request(socket: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = timeout(Duration::from_secs(2), socket.read(&mut buffer))
                .await
                .unwrap()
                .unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                let header_text = String::from_utf8_lossy(&request[..header_end]);
                let content_length = header_text
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&request).to_string()
    }

    async fn write_json_response(socket: &mut TcpStream, status: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    }

    fn sample_image_attachment() -> AgentAttachment {
        AgentAttachment {
            id: "img-1".to_string(),
            name: "demo.png".to_string(),
            mime: "image/png".to_string(),
            size: 12,
            kind: "image".to_string(),
            data_url: Some("data:image/png;base64,ZmFrZQ==".to_string()),
            text_preview: None,
            island_package_id: None,
        }
    }

    #[test]
    fn openai_content_includes_image_attachment_part() {
        let content = openai_content_from_message(
            "请看图".to_string(),
            vec![sample_image_attachment()],
            None,
        );

        let OpenAIMessageContent::Parts(parts) = content else {
            panic!("image attachments must serialize as OpenAI content parts");
        };
        assert!(parts
            .iter()
            .any(|part| matches!(part, OpenAIContentPart::Text { text } if text == "请看图")));
        assert!(parts.iter().any(|part| {
            matches!(
                part,
                OpenAIContentPart::ImageUrl { image_url }
                    if image_url.url == "data:image/png;base64,ZmFrZQ=="
            )
        }));
    }

    #[test]
    fn openai_content_allows_image_only_message_without_text_part() {
        let content =
            openai_content_from_message(String::new(), vec![sample_image_attachment()], None);

        let OpenAIMessageContent::Parts(parts) = content else {
            panic!("image-only attachments must serialize as OpenAI content parts");
        };
        assert_eq!(parts.len(), 1);
        assert!(matches!(
            &parts[0],
            OpenAIContentPart::ImageUrl { image_url }
                if image_url.url == "data:image/png;base64,ZmFrZQ=="
        ));
    }

    // M1.1: tool_calls 在 stream / non-stream / null / 缺失字段的反序列化兼容性。
    #[test]
    fn stream_delta_accepts_null_tool_calls() {
        let delta: OpenAIStreamDelta =
            serde_json::from_str(r#"{"content":"ok","tool_calls":null}"#)
                .expect("null tool_calls must not panic");
        assert_eq!(delta.content.as_deref(), Some("ok"));
        assert!(delta.tool_calls.is_empty());
    }

    #[test]
    fn stream_delta_accepts_missing_tool_calls() {
        let delta: OpenAIStreamDelta = serde_json::from_str(r#"{"content":"ok"}"#)
            .expect("missing tool_calls must default to empty vec");
        assert_eq!(delta.content.as_deref(), Some("ok"));
        assert!(delta.tool_calls.is_empty());
    }

    #[test]
    fn response_message_accepts_null_tool_calls() {
        let message: OpenAIResponseMessage =
            serde_json::from_str(r#"{"content":"hi","tool_calls":null}"#)
                .expect("non-streaming null tool_calls must not panic");
        assert_eq!(message.content.as_deref(), Some("hi"));
        assert!(message.tool_calls.is_empty());
    }

    #[test]
    fn response_message_accepts_missing_tool_calls() {
        let message: OpenAIResponseMessage = serde_json::from_str(r#"{"content":"hi"}"#)
            .expect("non-streaming missing tool_calls must default to empty vec");
        assert_eq!(message.content.as_deref(), Some("hi"));
        assert!(message.tool_calls.is_empty());
    }

    #[test]
    fn response_message_accepts_null_content_with_tool_calls() {
        let payload = r#"{
            "content": null,
            "tool_calls": [
                {"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":\"a.txt\"}"}}
            ]
        }"#;
        let message: OpenAIResponseMessage =
            serde_json::from_str(payload).expect("null content + tool_calls must not panic");
        assert!(message.content.is_none());
        assert_eq!(message.tool_calls.len(), 1);
        assert_eq!(message.tool_calls[0].id, "call_1");
        assert_eq!(message.tool_calls[0].function.name, "read_file");
    }

    #[test]
    fn response_message_accepts_null_content_without_tool_calls() {
        // 空消息（content=null, tool_calls 缺失或 null）必须能解析成不报错的 ChatResponse —
        // 让上层决定是否提示用户重试，而不是 serde 阶段 panic。
        let payload = r#"{"content":null}"#;
        let message: OpenAIResponseMessage = serde_json::from_str(payload)
            .expect("null content + missing tool_calls must not panic");
        assert!(message.content.is_none());
        assert!(message.tool_calls.is_empty());
    }

    #[test]
    fn text_json_tool_call_is_parsed_when_finish_reason_requests_tools() {
        let payload = r#"{
            "choices": [{
                "message": {
                    "content": "{\"tool_calls\":[{\"id\":\"call_1\",\"function\":{\"name\":\"search_web\",\"arguments\":\"{\\\"query\\\":\\\"Atlas\\\",\\\"numResults\\\":3}\"}}]}"
                },
                "finish_reason": "tool_calls"
            }]
        }"#;
        let response: OpenAIResponse = serde_json::from_str(payload).unwrap();
        let chat = openai_response_to_chat(response).unwrap();

        assert_eq!(chat.tool_calls.len(), 1);
        assert_eq!(chat.tool_calls[0].id, "call_1");
        assert_eq!(chat.tool_calls[0].name, "search_web");
        assert_eq!(chat.tool_calls[0].arguments["query"], "Atlas");
        assert_eq!(chat.tool_calls[0].arguments["limit"], 3);
    }

    #[test]
    fn text_json_tool_call_is_not_parsed_for_plain_stop_response() {
        let payload = r#"{
            "choices": [{
                "message": {
                    "content": "{\"tool_calls\":[{\"name\":\"read_file\",\"arguments\":{\"path\":\"Cargo.toml\"}}]}"
                },
                "finish_reason": "stop"
            }]
        }"#;
        let response: OpenAIResponse = serde_json::from_str(payload).unwrap();
        let chat = openai_response_to_chat(response).unwrap();

        assert!(chat.tool_calls.is_empty());
        assert!(chat.content.unwrap().contains("tool_calls"));
    }

    #[test]
    fn standard_tool_call_aliases_num_results_to_limit() {
        let payload = r#"{
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [
                        {"id":"call_2","function":{"name":"search_web","arguments":"{\"query\":\"Atlas\",\"num_results\":4}"}}
                    ]
                },
                "finish_reason": "tool_calls"
            }]
        }"#;
        let response: OpenAIResponse = serde_json::from_str(payload).unwrap();
        let chat = openai_response_to_chat(response).unwrap();

        assert_eq!(chat.tool_calls.len(), 1);
        assert_eq!(chat.tool_calls[0].name, "search_web");
        assert_eq!(chat.tool_calls[0].arguments["query"], "Atlas");
        assert_eq!(chat.tool_calls[0].arguments["limit"], 4);
    }

    // M1.2: 当 provider 明确不支持视觉时，剥掉 image part 并附上系统提示。
    #[test]
    fn image_part_is_stripped_when_vision_explicitly_unsupported() {
        let content = openai_content_from_message(
            "请看图".to_string(),
            vec![sample_image_attachment()],
            Some(false),
        );

        let OpenAIMessageContent::Parts(parts) = content else {
            panic!("text + stripped note must serialize as multiple parts");
        };
        assert!(!parts
            .iter()
            .any(|part| matches!(part, OpenAIContentPart::ImageUrl { .. })));
        assert!(parts.iter().any(|part| matches!(
            part,
            OpenAIContentPart::Text { text } if text == "请看图"
        )));
        assert!(parts.iter().any(|part| matches!(
            part,
            OpenAIContentPart::Text { text }
                if text.contains("系统提示")
                    && text.contains("剥离 1 张图片附件")
                    && text.contains("demo.png")
        )));
    }

    #[test]
    fn image_part_is_kept_when_vision_supported_or_unknown() {
        for vision in [None, Some(true)] {
            let content = openai_content_from_message(
                "请看图".to_string(),
                vec![sample_image_attachment()],
                vision,
            );
            let OpenAIMessageContent::Parts(parts) = content else {
                panic!("supported vision must keep image parts");
            };
            assert!(parts
                .iter()
                .any(|part| matches!(part, OpenAIContentPart::ImageUrl { .. })));
            assert!(!parts.iter().any(|part| matches!(
                part,
                OpenAIContentPart::Text { text } if text.contains("系统提示")
            )));
        }
    }

    #[test]
    fn openai_usage_converts_prompt_completion_and_total_tokens() {
        let usage = openai_usage_to_model_usage(OpenAIUsage {
            prompt_tokens: Some(12),
            completion_tokens: Some(7),
            total_tokens: Some(20),
        })
        .unwrap();
        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.output_tokens, 7);
        assert_eq!(usage.total_tokens, 20);
    }

    #[test]
    fn openai_usage_falls_back_to_prompt_plus_completion_total() {
        let usage = openai_usage_to_model_usage(OpenAIUsage {
            prompt_tokens: Some(12),
            completion_tokens: Some(7),
            total_tokens: None,
        })
        .unwrap();
        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.output_tokens, 7);
        assert_eq!(usage.total_tokens, 19);
    }

    #[tokio::test]
    async fn non_streaming_request_retries_without_tools_when_provider_rejects_tools() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            let first_request = read_http_request(&mut first).await;
            assert!(first_request.contains("\"tools\""));
            write_json_response(
                &mut first,
                "400 Bad Request",
                r#"{"error":{"message":"tools is not supported"}}"#,
            )
            .await;

            let (mut second, _) = listener.accept().await.unwrap();
            let second_request = read_http_request(&mut second).await;
            assert!(!second_request.contains("\"tools\""));
            write_json_response(
                &mut second,
                "200 OK",
                r#"{"choices":[{"message":{"content":"直接回复"},"finish_reason":"stop"}]}"#,
            )
            .await;
        });

        let client = OpenAIClient::new("test-key".to_string(), "test-model".to_string())
            .with_base_url(format!("http://{addr}"));
        let result = client
            .chat_completion(
                vec![Message::plain(Role::User, "hello")],
                Some(vec![ToolSchema {
                    name: "read_file".to_string(),
                    description: "Read a file".to_string(),
                    parameters: serde_json::json!({"type": "object"}),
                }]),
            )
            .await
            .unwrap();

        server.await.unwrap();
        assert_eq!(result.content.as_deref(), Some("直接回复"));
    }

    #[tokio::test]
    async fn structured_json_request_uses_response_format_when_supported() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            assert!(request.contains("\"response_format\""));
            assert!(request.contains("\"type\":\"json_object\""));
            assert!(request.contains("Structured JSON output requested"));
            write_json_response(
                &mut socket,
                "200 OK",
                r#"{"choices":[{"message":{"content":"{\"ok\":true}"},"finish_reason":"stop"}]}"#,
            )
            .await;
        });

        let client = OpenAIClient::new("test-key".to_string(), "test-model".to_string())
            .with_base_url(format!("http://{addr}"))
            .with_json_mode_supported(Some(true));
        let result = client
            .chat_completion_with_options(
                vec![Message::plain(Role::User, "Return ok as JSON")],
                None,
                ChatCompletionOptions::json_object(),
            )
            .await
            .unwrap();

        server.await.unwrap();
        assert_eq!(result.content.as_deref(), Some("{\"ok\":true}"));
    }

    #[tokio::test]
    async fn structured_json_request_downgrades_when_json_mode_unsupported() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            assert!(!request.contains("\"response_format\""));
            assert!(request.contains("not using provider JSON mode"));
            write_json_response(
                &mut socket,
                "200 OK",
                r#"{"choices":[{"message":{"content":"{\"ok\":true}"},"finish_reason":"stop"}]}"#,
            )
            .await;
        });

        let client = OpenAIClient::new("test-key".to_string(), "test-model".to_string())
            .with_base_url(format!("http://{addr}"))
            .with_json_mode_supported(Some(false));
        let result = client
            .chat_completion_with_options(
                vec![Message::plain(Role::User, "Return ok as JSON")],
                None,
                ChatCompletionOptions::json_object(),
            )
            .await
            .unwrap();

        server.await.unwrap();
        assert_eq!(result.content.as_deref(), Some("{\"ok\":true}"));
    }

    #[tokio::test]
    async fn structured_json_request_retries_without_response_format_on_compat_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            let first_request = read_http_request(&mut first).await;
            assert!(first_request.contains("\"response_format\""));
            write_json_response(
                &mut first,
                "400 Bad Request",
                r#"{"error":{"message":"Unrecognized request argument supplied: response_format"}}"#,
            )
            .await;

            let (mut second, _) = listener.accept().await.unwrap();
            let second_request = read_http_request(&mut second).await;
            assert!(!second_request.contains("\"response_format\""));
            assert!(second_request.contains("not using provider JSON mode"));
            write_json_response(
                &mut second,
                "200 OK",
                r#"{"choices":[{"message":{"content":"{\"ok\":true}"},"finish_reason":"stop"}]}"#,
            )
            .await;
        });

        let client = OpenAIClient::new("test-key".to_string(), "test-model".to_string())
            .with_base_url(format!("http://{addr}"))
            .with_json_mode_supported(Some(true));
        let result = client
            .chat_completion_with_options(
                vec![Message::plain(Role::User, "Return ok as JSON")],
                None,
                ChatCompletionOptions::json_object(),
            )
            .await
            .unwrap();

        server.await.unwrap();
        assert_eq!(result.content.as_deref(), Some("{\"ok\":true}"));
    }

    #[tokio::test]
    async fn plain_request_does_not_enable_json_mode_even_when_supported() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            assert!(!request.contains("\"response_format\""));
            assert!(!request.contains("Structured JSON output requested"));
            write_json_response(
                &mut socket,
                "200 OK",
                r#"{"choices":[{"message":{"content":"普通回复"},"finish_reason":"stop"}]}"#,
            )
            .await;
        });

        let client = OpenAIClient::new("test-key".to_string(), "test-model".to_string())
            .with_base_url(format!("http://{addr}"))
            .with_json_mode_supported(Some(true));
        let result = client
            .chat_completion(vec![Message::plain(Role::User, "hello")], None)
            .await
            .unwrap();

        server.await.unwrap();
        assert_eq!(result.content.as_deref(), Some("普通回复"));
    }

    #[tokio::test]
    async fn streaming_request_falls_back_when_provider_rejects_stream_options() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            let first_request = read_http_request(&mut first).await;
            assert!(first_request.contains("\"stream\":true"));
            assert!(first_request.contains("\"stream_options\""));
            write_json_response(
                &mut first,
                "400 Bad Request",
                r#"{"error":{"message":"Unrecognized request argument supplied: stream_options"}}"#,
            )
            .await;

            let (mut second, _) = listener.accept().await.unwrap();
            let second_request = read_http_request(&mut second).await;
            assert!(!second_request.contains("\"stream\":true"));
            assert!(!second_request.contains("\"stream_options\""));
            write_json_response(
                &mut second,
                "200 OK",
                r#"{"choices":[{"message":{"content":"兼容回复"},"finish_reason":"stop"}]}"#,
            )
            .await;
        });

        let client = OpenAIClient::new("test-key".to_string(), "test-model".to_string())
            .with_base_url(format!("http://{addr}"));
        let (tx, mut rx) = mpsc::channel(16);
        let result = client
            .chat_completion_stream(vec![Message::plain(Role::User, "hello")], None, Some(tx))
            .await
            .unwrap();

        assert_eq!(result.content.as_deref(), Some("兼容回复"));
        let mut saw_fallback = false;
        let mut saw_completed = false;
        while let Ok(Some(event)) = timeout(Duration::from_millis(100), rx.recv()).await {
            match event {
                AgentEvent::ResponseFallbackStarted { reason, .. }
                    if reason == "streaming_not_supported" =>
                {
                    saw_fallback = true;
                }
                AgentEvent::ResponseCompleted { content, .. } if content == "兼容回复" => {
                    saw_completed = true;
                }
                _ => {}
            }
        }
        server.await.unwrap();

        assert!(saw_fallback);
        assert!(saw_completed);
    }

    #[tokio::test]
    async fn streaming_request_accepts_non_sse_json_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            assert!(request.contains("\"stream\":true"));
            write_json_response(
                &mut socket,
                "200 OK",
                r#"{"choices":[{"message":{"content":"非流式 JSON 回复"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}}"#,
            )
            .await;
        });

        let client = OpenAIClient::new("test-key".to_string(), "test-model".to_string())
            .with_base_url(format!("http://{addr}"));
        let (tx, mut rx) = mpsc::channel(16);
        let result = client
            .chat_completion_stream(vec![Message::plain(Role::User, "hello")], None, Some(tx))
            .await
            .unwrap();

        server.await.unwrap();
        assert_eq!(result.content.as_deref(), Some("非流式 JSON 回复"));
        assert_eq!(result.usage.unwrap().total_tokens, 7);

        let mut saw_completed = false;
        while let Ok(Some(event)) = timeout(Duration::from_millis(100), rx.recv()).await {
            if let AgentEvent::ResponseCompleted { content, .. } = event {
                saw_completed |= content == "非流式 JSON 回复";
            }
        }
        assert!(saw_completed);
    }

    #[tokio::test]
    async fn streaming_structured_json_request_uses_response_format_when_supported() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            assert!(request.contains("\"stream\":true"));
            assert!(request.contains("\"response_format\""));
            assert!(request.contains("\"type\":\"json_object\""));
            write_json_response(
                &mut socket,
                "200 OK",
                r#"{"choices":[{"message":{"content":"{\"ok\":true}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}}"#,
            )
            .await;
        });

        let client = OpenAIClient::new("test-key".to_string(), "test-model".to_string())
            .with_base_url(format!("http://{addr}"))
            .with_json_mode_supported(Some(true));
        let (tx, _rx) = mpsc::channel(16);
        let result = client
            .chat_completion_stream_with_options(
                vec![Message::plain(Role::User, "Return ok as JSON")],
                None,
                Some(tx),
                ChatCompletionOptions::json_object(),
            )
            .await
            .unwrap();

        server.await.unwrap();
        assert_eq!(result.content.as_deref(), Some("{\"ok\":true}"));
        assert_eq!(result.usage.unwrap().total_tokens, 7);
    }

    #[tokio::test]
    async fn streaming_request_falls_back_when_stream_chunk_is_malformed() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            let first_request = read_http_request(&mut first).await;
            assert!(first_request.contains("\"stream\":true"));
            first
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: {not-json}\n\n",
                )
                .await
                .unwrap();

            let (mut second, _) = listener.accept().await.unwrap();
            let second_request = read_http_request(&mut second).await;
            assert!(!second_request.contains("\"stream\":true"));
            write_json_response(
                &mut second,
                "200 OK",
                r#"{"choices":[{"message":{"content":"普通请求回复"},"finish_reason":"stop"}]}"#,
            )
            .await;
        });

        let client = OpenAIClient::new("test-key".to_string(), "test-model".to_string())
            .with_base_url(format!("http://{addr}"));
        let (tx, mut rx) = mpsc::channel(16);
        let result = client
            .chat_completion_stream(vec![Message::plain(Role::User, "hello")], None, Some(tx))
            .await
            .unwrap();

        server.await.unwrap();
        assert_eq!(result.content.as_deref(), Some("普通请求回复"));

        let mut saw_parse_fallback = false;
        while let Ok(Some(event)) = timeout(Duration::from_millis(100), rx.recv()).await {
            if let AgentEvent::ResponseFallbackStarted { reason, .. } = event {
                saw_parse_fallback |= reason == "stream_parse_failed";
            }
        }
        assert!(saw_parse_fallback);
    }

    #[tokio::test]
    async fn stream_wait_without_first_chunk_emits_visible_heartbeat() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = timeout(Duration::from_secs(2), socket.read(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if String::from_utf8_lossy(&request).contains("\"stream\":true") {
                    break;
                }
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            sleep(Duration::from_millis(1200)).await;
            socket
                .write_all(
                    r#"data: {"choices":[{"delta":{"content":"好了"},"finish_reason":"stop"}]}

data: [DONE]

"#
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let client = OpenAIClient::new("test-key".to_string(), "test-model".to_string())
            .with_base_url(format!("http://{addr}"));
        let (tx, mut rx) = mpsc::channel(16);
        let result = client
            .chat_completion_stream(vec![Message::plain(Role::User, "hello")], None, Some(tx))
            .await
            .unwrap();

        assert_eq!(result.content.as_deref(), Some("好了"));
        let mut labels = Vec::new();
        let mut saw_response_delta = false;
        while let Ok(Some(event)) = timeout(Duration::from_millis(100), rx.recv()).await {
            match event {
                AgentEvent::OperationProgress { label, detail, .. } => {
                    labels.push((label, detail.unwrap_or_default()));
                }
                AgentEvent::ResponseDelta { content, .. } if content == "好了" => {
                    saw_response_delta = true;
                }
                _ => {}
            }
        }
        server.await.unwrap();

        assert!(labels
            .iter()
            .any(|(label, detail)| label == "正在分析任务" && detail.contains("已收到：hello")));
        assert!(labels.iter().any(|(label, detail)| {
            label == "仍在等待模型输出"
                && detail.contains("正在组织直接回复")
                && detail.contains("还没有收到首个流式片段")
        }));
        assert!(saw_response_delta);
    }

    fn dummy_tool() -> ToolSchema {
        ToolSchema {
            name: "list_files".to_string(),
            description: "demo".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn maybe_drop_tools_strips_when_capability_says_no_tools() {
        let client =
            OpenAIClient::new("k".to_string(), "m".to_string()).with_tool_call_support(Some(false));
        let dropped = client.maybe_drop_tools(Some(vec![dummy_tool()]));
        assert!(
            dropped.is_none(),
            "must drop tools when capability says unsupported"
        );
    }

    #[test]
    fn maybe_drop_tools_keeps_when_capability_says_yes() {
        let client =
            OpenAIClient::new("k".to_string(), "m".to_string()).with_tool_call_support(Some(true));
        let kept = client.maybe_drop_tools(Some(vec![dummy_tool()]));
        assert_eq!(kept.as_ref().map(|v| v.len()), Some(1));
    }

    #[test]
    fn maybe_drop_tools_keeps_when_capability_unknown() {
        let client = OpenAIClient::new("k".to_string(), "m".to_string());
        let kept = client.maybe_drop_tools(Some(vec![dummy_tool()]));
        assert_eq!(kept.as_ref().map(|v| v.len()), Some(1));
    }

    #[test]
    fn with_json_mode_supported_stores_flag() {
        let client = OpenAIClient::new("k".to_string(), "m".to_string())
            .with_json_mode_supported(Some(true));
        assert_eq!(client.json_mode_supported, Some(true));
    }
}
