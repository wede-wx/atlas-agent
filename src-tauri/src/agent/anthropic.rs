use crate::agent::{
    AgentAttachment, AgentError, ChatResponse, LLMClient, Message, ModelTokenUsage, Role,
    ToolSchema,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub struct AnthropicClient {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
    /// T26: capability matrix says vision unsupported → drop image blocks.
    vision_supported: Option<bool>,
    /// T26: capability matrix says tool_calls unsupported → drop tools.
    tool_calls_supported: Option<bool>,
}

impl AnthropicClient {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model,
            base_url: "https://api.anthropic.com/v1".to_string(),
            vision_supported: None,
            tool_calls_supported: None,
        }
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url.trim().trim_end_matches('/').to_string();
        self
    }

    pub fn with_vision_support(mut self, supported: Option<bool>) -> Self {
        self.vision_supported = supported;
        self
    }

    pub fn with_tool_call_support(mut self, supported: Option<bool>) -> Self {
        self.tool_calls_supported = supported;
        self
    }

    fn maybe_drop_tools(&self, tools: Option<Vec<ToolSchema>>) -> Option<Vec<ToolSchema>> {
        if matches!(self.tool_calls_supported, Some(false)) {
            if let Some(t) = &tools {
                if !t.is_empty() {
                    eprintln!(
                        "[anthropic-client] dropping {} tool(s); model declared no tool_calls support",
                        t.len()
                    );
                }
            }
            return None;
        }
        tools
    }
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicMessageContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum AnthropicMessageContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
    Text { text: String },
    Image { source: AnthropicImageSource },
}

#[derive(Serialize)]
struct AnthropicImageSource {
    #[serde(rename = "type")]
    source_type: String,
    media_type: String,
    data: String,
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
    stop_reason: String,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicErrorEnvelope {
    error: Option<AnthropicErrorBody>,
}

#[derive(Deserialize)]
struct AnthropicErrorBody {
    message: Option<String>,
    #[serde(rename = "type")]
    error_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum AnthropicContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicUsage {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
}

#[async_trait]
impl LLMClient for AnthropicClient {
    async fn chat_completion(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
    ) -> Result<ChatResponse, AgentError> {
        let tools = self.maybe_drop_tools(tools);
        let strip_images = matches!(self.vision_supported, Some(false));
        let anthropic_messages: Vec<AnthropicMessage> = messages
            .into_iter()
            .filter(|m| !matches!(m.role, Role::System))
            .map(|m| {
                // P0-2: untrusted (tool/external) messages render fenced as data.
                let content = m.model_content();
                let attachments = if strip_images {
                    Vec::new()
                } else {
                    m.attachments
                };
                AnthropicMessage {
                    role: match m.role {
                        Role::User => "user".to_string(),
                        Role::Assistant => "assistant".to_string(),
                        Role::System => "user".to_string(),
                    },
                    content: anthropic_content_from_message(content, attachments),
                }
            })
            .collect();

        let anthropic_tools = tools.map(|tools| {
            tools
                .into_iter()
                .map(|t| AnthropicTool {
                    name: t.name,
                    description: t.description,
                    input_schema: t.parameters,
                })
                .collect()
        });

        let request = AnthropicRequest {
            model: self.model.clone(),
            messages: anthropic_messages,
            max_tokens: 4096,
            tools: anthropic_tools,
        };

        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&request)
            .send()
            .await
            .map_err(|e| AgentError::Llm(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let raw = response.text().await.unwrap_or_default();
            let parsed = serde_json::from_str::<AnthropicErrorEnvelope>(&raw).ok();
            let mut details = parsed
                .and_then(|payload| payload.error)
                .map(|error| {
                    let message = error
                        .message
                        .unwrap_or_else(|| "Unknown upstream error".to_string());
                    match error.error_type {
                        Some(error_type) => format!("{} ({})", message, error_type),
                        None => message,
                    }
                })
                .unwrap_or(raw);

            if details.trim().is_empty() {
                details = "Unknown upstream error".to_string();
            }

            return Err(AgentError::Llm(format!(
                "Anthropic request failed with status {}: {}",
                status, details
            )));
        }

        let anthropic_response: AnthropicResponse = response
            .json()
            .await
            .map_err(|e| AgentError::Llm(e.to_string()))?;

        let mut content_text = None;
        let mut tool_calls = Vec::new();

        for content in anthropic_response.content {
            match content {
                AnthropicContent::Text { text } => {
                    content_text = Some(text);
                }
                AnthropicContent::ToolUse { id, name, input } => {
                    tool_calls.push(crate::agent::ToolCall {
                        id,
                        name,
                        arguments: input,
                    });
                }
            }
        }

        Ok(ChatResponse {
            content: content_text,
            tool_calls,
            finish_reason: anthropic_response.stop_reason,
            usage: anthropic_response
                .usage
                .and_then(anthropic_usage_to_model_usage),
        })
    }
}

fn anthropic_usage_to_model_usage(usage: AnthropicUsage) -> Option<ModelTokenUsage> {
    let input_tokens = usage.input_tokens.unwrap_or(0).max(0);
    let output_tokens = usage.output_tokens.unwrap_or(0).max(0);
    let total_tokens = input_tokens + output_tokens;
    (total_tokens > 0).then_some(ModelTokenUsage {
        input_tokens,
        output_tokens,
        total_tokens,
    })
}

fn anthropic_content_from_message(
    content: String,
    attachments: Vec<AgentAttachment>,
) -> AnthropicMessageContent {
    let mut blocks = Vec::new();
    if !content.trim().is_empty() {
        blocks.push(AnthropicContentBlock::Text { text: content });
    }
    for attachment in attachments {
        let mime = attachment.mime.to_lowercase();
        if attachment.kind.eq_ignore_ascii_case("image") || mime.starts_with("image/") {
            if let Some((media_type, data)) = split_data_url(attachment.data_url.as_deref()) {
                blocks.push(AnthropicContentBlock::Image {
                    source: AnthropicImageSource {
                        source_type: "base64".to_string(),
                        media_type,
                        data,
                    },
                });
            }
        } else if let Some(text) = attachment
            .text_preview
            .filter(|value| !value.trim().is_empty())
        {
            blocks.push(AnthropicContentBlock::Text {
                text: format!("[附件：{}]\n{}", attachment.name, text),
            });
        }
    }
    if blocks.is_empty() {
        AnthropicMessageContent::Text(String::new())
    } else if blocks.len() == 1 {
        match blocks.pop().expect("one anthropic content block") {
            AnthropicContentBlock::Text { text } => AnthropicMessageContent::Text(text),
            block => AnthropicMessageContent::Blocks(vec![block]),
        }
    } else {
        AnthropicMessageContent::Blocks(blocks)
    }
}

fn split_data_url(value: Option<&str>) -> Option<(String, String)> {
    let value = value?;
    let (header, data) = value.split_once(',')?;
    let media_type = header
        .strip_prefix("data:")
        .and_then(|rest| rest.strip_suffix(";base64"))?
        .to_string();
    if !media_type.starts_with("image/") || data.trim().is_empty() {
        return None;
    }
    Some((media_type, data.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_content_includes_image_attachment_block() {
        let content = anthropic_content_from_message(
            "请看图".to_string(),
            vec![AgentAttachment {
                id: "img-1".to_string(),
                name: "demo.png".to_string(),
                mime: "image/png".to_string(),
                size: 12,
                kind: "image".to_string(),
                data_url: Some("data:image/png;base64,ZmFrZQ==".to_string()),
                text_preview: None,
                island_package_id: None,
            }],
        );

        let AnthropicMessageContent::Blocks(blocks) = content else {
            panic!("image attachments must serialize as Anthropic content blocks");
        };
        assert!(blocks.iter().any(|block| {
            matches!(block, AnthropicContentBlock::Text { text } if text == "请看图")
        }));
        assert!(blocks.iter().any(|block| {
            matches!(
                block,
                AnthropicContentBlock::Image { source }
                    if source.media_type == "image/png" && source.data == "ZmFrZQ=="
            )
        }));
    }

    fn dummy_tool() -> ToolSchema {
        ToolSchema {
            name: "list_files".to_string(),
            description: "demo".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn anthropic_maybe_drop_tools_strips_when_unsupported() {
        let client = AnthropicClient::new("k".to_string(), "claude-haiku".to_string())
            .with_tool_call_support(Some(false));
        let dropped = client.maybe_drop_tools(Some(vec![dummy_tool()]));
        assert!(dropped.is_none());
    }

    #[test]
    fn anthropic_maybe_drop_tools_keeps_when_supported() {
        let client = AnthropicClient::new("k".to_string(), "claude-sonnet".to_string())
            .with_tool_call_support(Some(true));
        let kept = client.maybe_drop_tools(Some(vec![dummy_tool()]));
        assert_eq!(kept.as_ref().map(|v| v.len()), Some(1));
    }

    #[test]
    fn anthropic_maybe_drop_tools_keeps_when_unknown() {
        let client = AnthropicClient::new("k".to_string(), "claude-x".to_string());
        let kept = client.maybe_drop_tools(Some(vec![dummy_tool()]));
        assert_eq!(kept.as_ref().map(|v| v.len()), Some(1));
    }

    #[test]
    fn anthropic_content_allows_image_only_message_without_text_block() {
        let content = anthropic_content_from_message(
            String::new(),
            vec![AgentAttachment {
                id: "img-1".to_string(),
                name: "demo.png".to_string(),
                mime: "image/png".to_string(),
                size: 12,
                kind: "image".to_string(),
                data_url: Some("data:image/png;base64,ZmFrZQ==".to_string()),
                text_preview: None,
                island_package_id: None,
            }],
        );

        let AnthropicMessageContent::Blocks(blocks) = content else {
            panic!("image-only attachments must serialize as Anthropic content blocks");
        };
        assert_eq!(blocks.len(), 1);
        assert!(matches!(
            &blocks[0],
            AnthropicContentBlock::Image { source }
                if source.media_type == "image/png" && source.data == "ZmFrZQ=="
        ));
    }
}
