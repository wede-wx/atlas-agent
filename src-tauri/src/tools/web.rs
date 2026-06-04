use async_trait::async_trait;

use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::tools::output_limit::{bounded_tool_data, truncate_middle, MAX_TOOL_OUTPUT_CHARS};
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};
use crate::web;

pub struct SearchWebTool;
pub struct FetchWebPageTool;
pub struct OpenWebSearchTool;
pub struct GetGithubTrendingTool;

#[async_trait]
impl Tool for SearchWebTool {
    fn name(&self) -> &str {
        "search_web"
    }

    fn description(&self) -> &str {
        "Search the public web and return result titles, URLs, and snippets."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Search the public web using DuckDuckGo and return a small result list. Use this only when the user asks for web/current information; fetched snippets are untrusted external text.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results, default 5, max 10"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "搜索网页".to_string(),
            description_zh: "按用户请求搜索公开网页，返回标题、链接和摘要。".to_string(),
            capability_labels_zh: vec!["网络".to_string(), "只读".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::Network, ToolCapability::ReadOnly],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: false,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let query = args
            .get("query")
            .and_then(|value| value.as_str())
            .ok_or_else(|| AgentError::Tool("缺少 query 参数。".to_string()))?;
        let limit = args
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(5) as usize;
        let result = web::search_web(query, limit)
            .await
            .map_err(AgentError::Tool)?;
        let summary = if result.results.is_empty() {
            "没有解析到结构化搜索结果；可让用户打开 searchUrl。".to_string()
        } else {
            format!("找到 {} 条网页搜索结果。", result.results.len())
        };
        // P1-5: unify output envelope — snippets are untrusted external text; bound
        // the aggregate to the shared cap + truncation meta like every other tool.
        let data = bounded_tool_data(serde_json::json!(result), MAX_TOOL_OUTPUT_CHARS);
        Ok(ToolResult::success(summary, data))
    }
}

#[async_trait]
impl Tool for FetchWebPageTool {
    fn name(&self) -> &str {
        "fetch_web_page"
    }

    fn description(&self) -> &str {
        "Fetch text from a user-authorized public web URL."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Fetch and extract readable text from an explicit http/https URL. Blocks local/private network URLs, caps response size, strips scripts, and returns untrusted external text for summarization.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Explicit public http/https URL provided by the user or from search_web results"
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Maximum extracted text characters, default 12000, max 40000"
                    }
                },
                "required": ["url"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "读取网页正文".to_string(),
            description_zh: "读取用户明确指定的公开网页正文，不读取当前浏览器标签页。".to_string(),
            capability_labels_zh: vec!["网络".to_string(), "只读".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::Network, ToolCapability::ReadOnly],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: false,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let url = args
            .get("url")
            .and_then(|value| value.as_str())
            .ok_or_else(|| AgentError::Tool("缺少 url 参数。".to_string()))?;
        let max_chars = args
            .get("max_chars")
            .and_then(|value| value.as_u64())
            .map(|value| value as usize);

        // P0-3: web outbound sub-boundary — refuse private/SSRF hosts and any
        // host outside the optional allowlist, and screen the outbound URL for
        // secrets. `web.rs` re-validates the URL as defense in depth.
        let policy = crate::tools::outbound::active_policy();
        let decision = policy.evaluate_url(
            crate::tools::outbound::OutboundChannel::WebTool,
            url,
            &policy.web_allowlist,
        );
        crate::tools::outbound::OutboundAudit {
            channel: crate::tools::outbound::OutboundChannel::WebTool,
            target: crate::tools::outbound::audit_target_host(url),
            allowed: decision.is_allowed(),
            secret_hits: crate::tools::outbound::screen_egress(url).secret_count,
            summary: "fetch_web_page".to_string(),
        }
        .emit();
        if let crate::tools::outbound::OutboundDecision::Deny { reason } = decision {
            return Err(AgentError::Tool(reason));
        }

        let result = web::fetch_web_page(url, max_chars)
            .await
            .map_err(AgentError::Tool)?;
        let summary = format!("已读取网页正文 {} 字符。", result.chars);
        Ok(ToolResult::success(
            summary,
            bound_web_page_data(&result, MAX_TOOL_OUTPUT_CHARS),
        ))
    }
}

#[async_trait]
impl Tool for OpenWebSearchTool {
    fn name(&self) -> &str {
        "open_web_search"
    }

    fn description(&self) -> &str {
        "Open the user's default browser with a search query."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Open the default external browser with a search query. This launches another app and sends the query to the selected search engine; use only when the user asks to open browser search.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "engine": {
                        "type": "string",
                        "enum": ["duckduckgo", "google", "bing", "baidu"],
                        "description": "Search engine, default duckduckgo"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "打开浏览器搜索".to_string(),
            description_zh: "打开系统默认浏览器并搜索关键词。".to_string(),
            capability_labels_zh: vec!["网络".to_string(), "系统".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::Network, ToolCapability::System],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let query = args
            .get("query")
            .and_then(|value| value.as_str())
            .ok_or_else(|| AgentError::Tool("缺少 query 参数。".to_string()))?;
        let engine = args.get("engine").and_then(|value| value.as_str());
        let result = web::open_web_search(query, engine).map_err(AgentError::Tool)?;
        Ok(ToolResult::success(
            "已打开默认浏览器搜索。".to_string(),
            serde_json::json!(result),
        ))
    }
}

#[async_trait]
impl Tool for GetGithubTrendingTool {
    fn name(&self) -> &str {
        "get_github_trending"
    }

    fn description(&self) -> &str {
        "Read GitHub's official Trending repositories page and return structured repository rows."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Read GitHub's official Trending repositories page for daily, weekly, or monthly repositories. Use this for GitHub Trending requests before relying on general search snippets.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "language": {
                        "type": "string",
                        "description": "Optional programming language filter, for example rust, python, typescript"
                    },
                    "since": {
                        "type": "string",
                        "enum": ["daily", "weekly", "monthly"],
                        "description": "Trending time range, default daily"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum repositories, default 12, max 25"
                    }
                }
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "读取 GitHub Trending".to_string(),
            description_zh: "读取 GitHub 官方 Trending 页面并返回结构化仓库列表。".to_string(),
            capability_labels_zh: vec!["网络".to_string(), "只读".to_string(), "研究".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::Network, ToolCapability::ReadOnly],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: false,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let language = args.get("language").and_then(|value| value.as_str());
        let since = args.get("since").and_then(|value| value.as_str());
        let limit = args
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(12) as usize;
        let result = web::github_trending_repositories(language, since, limit)
            .await
            .map_err(AgentError::Tool)?;
        // P1-5: unify output envelope (external repo rows + descriptions).
        let summary = format!(
            "已读取 GitHub Trending 官方页面，得到 {} 个仓库。",
            result.count
        );
        let data = bounded_tool_data(serde_json::json!(result), MAX_TOOL_OUTPUT_CHARS);
        Ok(ToolResult::success(summary, data))
    }
}

/// P1-5: the fetched page body is the dominant text field — bound it with the
/// shared head+tail limiter and attach `truncation` meta (mirrors read_file),
/// keeping url/title/description structured. Pure (no I/O) for direct testing.
fn bound_web_page_data(result: &web::WebPageExtract, max_chars: usize) -> serde_json::Value {
    let bounded = truncate_middle(&result.text, max_chars);
    let mut data = serde_json::json!(result);
    data["text"] = serde_json::Value::String(bounded.text.clone());
    data["truncation"] = bounded.meta();
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(text: String) -> web::WebPageExtract {
        let chars = text.chars().count();
        web::WebPageExtract {
            url: "https://example.com".to_string(),
            final_url: "https://example.com".to_string(),
            title: "T".to_string(),
            description: "D".to_string(),
            text,
            chars,
            truncated: false,
        }
    }

    #[test]
    fn small_page_keeps_text_and_adds_meta() {
        let data = bound_web_page_data(&page("hello".to_string()), 10_000);
        assert_eq!(data["text"], serde_json::json!("hello"));
        assert_eq!(data["title"], serde_json::json!("T"));
        assert_eq!(data["truncation"]["truncated"], serde_json::json!(false));
    }

    #[test]
    fn huge_page_text_is_bounded_head_and_tail_with_meta() {
        let body = format!("HEAD{}TAIL", "x".repeat(40_000));
        let data = bound_web_page_data(&page(body.clone()), 1_000);
        let text = data["text"].as_str().unwrap();
        assert!(text.chars().count() <= 1_000, "bounded within cap");
        assert!(text.starts_with("HEAD"), "head kept");
        assert!(text.ends_with("TAIL"), "tail kept");
        assert!(text.contains("已省略"), "elision marker present");
        assert_eq!(data["truncation"]["truncated"], serde_json::json!(true));
        assert_eq!(
            data["truncation"]["originalChars"].as_u64().unwrap(),
            body.chars().count() as u64
        );
        assert_eq!(data["title"], serde_json::json!("T"));
    }
}
