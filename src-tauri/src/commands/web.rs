use crate::web::{
    self, GithubTrendingResponse, OpenWebSearchResult, WebPageExtract, WebSearchResponse,
};
use crate::{storage::LogActivityEventPayload, AppState};
use serde_json::json;
use tauri::State;

#[tauri::command]
pub async fn search_web(
    query: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<WebSearchResponse, String> {
    let result = web::search_web(&query, limit.unwrap_or(5)).await?;
    log_web_activity(
        &state,
        "网页搜索",
        "执行公开网页搜索",
        json!({
            "provider": result.provider,
            "resultCount": result.results.len()
        }),
    );
    Ok(result)
}

#[tauri::command]
pub async fn open_external_web_search(
    query: String,
    engine: Option<String>,
    state: State<'_, AppState>,
) -> Result<OpenWebSearchResult, String> {
    let result = web::open_web_search(&query, engine.as_deref())?;
    log_web_activity(
        &state,
        "打开浏览器搜索",
        "打开系统默认浏览器执行搜索",
        json!({
            "engine": result.engine,
            "opened": result.opened
        }),
    );
    Ok(result)
}

#[tauri::command]
pub async fn fetch_web_page(
    url: String,
    max_chars: Option<usize>,
    state: State<'_, AppState>,
) -> Result<WebPageExtract, String> {
    let result = web::fetch_web_page(&url, max_chars).await?;
    log_web_activity(
        &state,
        "读取网页正文",
        "读取用户指定的公开网页正文",
        json!({
            "url": result.url,
            "finalUrl": result.final_url,
            "chars": result.chars,
            "truncated": result.truncated
        }),
    );
    Ok(result)
}

#[tauri::command]
pub async fn get_github_trending(
    language: Option<String>,
    since: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<GithubTrendingResponse, String> {
    let result = web::github_trending_repositories(
        language.as_deref(),
        since.as_deref(),
        limit.unwrap_or(12),
    )
    .await?;
    log_web_activity(
        &state,
        "读取 GitHub Trending",
        "读取 GitHub 官方 Trending 页面并提取仓库列表",
        json!({
            "sourceUrl": result.source_url,
            "since": result.since,
            "count": result.count
        }),
    );
    Ok(result)
}

fn log_web_activity(
    state: &State<'_, AppState>,
    title: &str,
    detail: &str,
    metadata: serde_json::Value,
) {
    let _ = state.local_db.log_activity_event(LogActivityEventPayload {
        date: None,
        kind: "system".to_string(),
        title: title.to_string(),
        detail: detail.to_string(),
        metadata,
    });
}
