use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use reqwest::header::{CONTENT_TYPE, LOCATION};
use reqwest::{redirect::Policy, Client, Url};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const MAX_WEB_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_TEXT_CHARS: usize = 12_000;
const MAX_TEXT_CHARS: usize = 40_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchItem {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchResponse {
    pub query: String,
    pub provider: String,
    pub search_url: String,
    pub results: Vec<WebSearchItem>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenWebSearchResult {
    pub query: String,
    pub engine: String,
    pub url: String,
    pub opened: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebPageExtract {
    pub url: String,
    pub final_url: String,
    pub title: String,
    pub description: String,
    pub text: String,
    pub chars: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubTrendingRepository {
    pub rank: usize,
    pub owner: String,
    pub name: String,
    pub full_name: String,
    pub url: String,
    pub description: String,
    pub language: Option<String>,
    pub stars: String,
    pub forks: String,
    pub stars_today: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubTrendingResponse {
    pub source_url: String,
    pub since: String,
    pub repositories: Vec<GithubTrendingRepository>,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserHistoryEntry {
    pub browser: String,
    pub profile: String,
    pub title: String,
    pub url: String,
    pub visit_count: i64,
    pub last_visit_time: Option<String>,
    #[serde(skip_serializing)]
    last_visit_chrome_time: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserHistorySearchResult {
    pub query: String,
    pub browser: String,
    pub items: Vec<BrowserHistoryEntry>,
    pub count: usize,
    pub skipped: Vec<String>,
    pub privacy_note: String,
}

#[derive(Debug, Clone)]
struct BrowserHistorySource {
    browser: String,
    profile: String,
    path: PathBuf,
}

pub fn build_search_url(query: &str, engine: Option<&str>) -> Result<(String, String), String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("Search keyword cannot be empty.".to_string());
    }

    let engine = normalize_engine(engine);
    let encoded = percent_encode(trimmed);
    let url = match engine.as_str() {
        "google" => format!("https://www.google.com/search?q={encoded}"),
        "bing" => format!("https://www.bing.com/search?q={encoded}"),
        "baidu" => format!("https://www.baidu.com/s?wd={encoded}"),
        _ => format!("https://duckduckgo.com/?q={encoded}"),
    };
    Ok((engine, url))
}

pub async fn search_web(query: &str, limit: usize) -> Result<WebSearchResponse, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("Search keyword cannot be empty.".to_string());
    }

    let limit = limit.clamp(1, 10);
    let encoded = percent_encode(trimmed);
    let search_url = format!("https://duckduckgo.com/?q={encoded}");
    let html_url = format!("https://duckduckgo.com/html/?q={encoded}");
    let client = web_client()?;
    let response = send_public_get(&client, validate_public_http_url(&html_url)?).await?;
    if !response.status().is_success() {
        return Err(format!(
            "Web search failed with status {}",
            response.status()
        ));
    }

    let html = read_response_limited(response, MAX_WEB_RESPONSE_BYTES).await?;
    let mut results = parse_duckduckgo_results(&html, limit);
    let mut provider = "duckduckgo".to_string();
    if results.is_empty() {
        let lite_url = format!("https://duckduckgo.com/lite/?q={encoded}");
        let lite_response = send_public_get(&client, validate_public_http_url(&lite_url)?).await?;
        if lite_response.status().is_success() {
            let lite_html = read_response_limited(lite_response, MAX_WEB_RESPONSE_BYTES).await?;
            results = parse_duckduckgo_lite_results(&lite_html, limit);
            if !results.is_empty() {
                provider = "duckduckgo_lite".to_string();
            }
        }
    }

    for direct in direct_search_results(trimmed).into_iter().rev() {
        if results.iter().any(|item| item.url == direct.url) {
            continue;
        }
        results.insert(0, direct);
    }
    results.truncate(limit);

    let warning = if results.is_empty() {
        Some("No structured search results were parsed; open searchUrl in the browser.".to_string())
    } else {
        None
    };

    Ok(WebSearchResponse {
        query: trimmed.to_string(),
        provider,
        search_url,
        results,
        warning,
    })
}

fn direct_search_results(query: &str) -> Vec<WebSearchItem> {
    let lower = query.to_ascii_lowercase();
    let mentions_github = lower.contains("github") || lower.contains("git hub");
    let mentions_trending = lower.contains("trending")
        || lower.contains("today")
        || query.contains("今日")
        || query.contains("今天")
        || query.contains("热门")
        || query.contains("最火")
        || query.contains("最热")
        || query.contains("趋势")
        || query.contains("热榜");

    if mentions_github && mentions_trending {
        return vec![WebSearchItem {
            title: "Trending repositories on GitHub today".to_string(),
            url: "https://github.com/trending?since=daily".to_string(),
            snippet: "GitHub official daily Trending repositories page. Fetch this URL to read current repositories.".to_string(),
        }];
    }

    Vec::new()
}

fn normalize_trending_since(since: Option<&str>) -> String {
    match since
        .unwrap_or("daily")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "weekly" => "weekly".to_string(),
        "monthly" => "monthly".to_string(),
        _ => "daily".to_string(),
    }
}

fn github_trending_url(language: Option<&str>, since: &str) -> String {
    let language = language.unwrap_or_default().trim();
    if language.is_empty() {
        return format!("https://github.com/trending?since={since}");
    }
    format!(
        "https://github.com/trending/{}?since={since}",
        percent_encode(language)
    )
}

fn parse_github_trending_repositories(html: &str, limit: usize) -> Vec<GithubTrendingRepository> {
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    let mut rows = Vec::new();

    while rows.len() < limit {
        let Some(article_rel) = lower[cursor..].find("<article") else {
            break;
        };
        let article_start = cursor + article_rel;
        let Some(article_end_rel) = lower[article_start..].find("</article>") else {
            break;
        };
        let article_end = article_start + article_end_rel + "</article>".len();
        let article = &html[article_start..article_end];
        if article.to_ascii_lowercase().contains("box-row") {
            if let Some(repo) = parse_github_trending_article(article, rows.len() + 1) {
                rows.push(repo);
            }
        }
        cursor = article_end;
    }

    rows
}

fn parse_github_trending_article(article: &str, rank: usize) -> Option<GithubTrendingRepository> {
    let lower = article.to_ascii_lowercase();
    let h2_start = lower.find("<h2")?;
    let h2_end = lower[h2_start..]
        .find("</h2>")
        .map(|offset| h2_start + offset)
        .unwrap_or(article.len());
    let h2 = &article[h2_start..h2_end];
    let repo_path = extract_first_href(h2)?;
    let repo_path = repo_path.trim_start_matches('/').trim();
    let mut parts = repo_path.split('/');
    let owner = parts.next()?.trim().to_string();
    let name = parts.next()?.trim().to_string();
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return None;
    }
    let full_name = format!("{owner}/{name}");
    let url = format!("https://github.com/{full_name}");

    let description = extract_first_element_text_after(article, "p", h2_end).unwrap_or_default();
    let language = extract_programming_language(article);
    let stars = extract_link_text_number(article, &format!("/{full_name}/stargazers"));
    let forks = extract_link_text_number(article, &format!("/{full_name}/forks"));
    let stars_today = extract_stars_today(article);

    Some(GithubTrendingRepository {
        rank,
        owner,
        name,
        full_name,
        url,
        description,
        language,
        stars,
        forks,
        stars_today,
    })
}

fn extract_first_href(fragment: &str) -> Option<String> {
    let lower = fragment.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(anchor_rel) = lower[cursor..].find("<a") {
        let start = cursor + anchor_rel;
        let end_rel = lower[start..].find('>')?;
        let end = start + end_rel;
        let tag = &fragment[start..=end];
        if let Some(href) = attr_value(tag, "href") {
            return Some(href);
        }
        cursor = end + 1;
    }
    None
}

fn extract_first_element_text_after(fragment: &str, tag: &str, after: usize) -> Option<String> {
    let lower = fragment.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start_rel = lower[after..].find(&open)?;
    let start = after + start_rel;
    let open_end = lower[start..].find('>').map(|offset| start + offset)?;
    let close_start = lower[open_end..]
        .find(&close)
        .map(|offset| open_end + offset)?;
    Some(html_to_text(&fragment[open_end + 1..close_start]))
}

fn extract_programming_language(article: &str) -> Option<String> {
    let lower = article.to_ascii_lowercase();
    let marker = "itemprop=\"programminglanguage\"";
    let start = lower.find(marker)?;
    let open_end = lower[start..].find('>').map(|offset| start + offset)?;
    let close_start = lower[open_end..]
        .find("</span>")
        .map(|offset| open_end + offset)?;
    let language = html_to_text(&article[open_end + 1..close_start]);
    if language.is_empty() {
        None
    } else {
        Some(language)
    }
}

fn extract_link_text_number(article: &str, href: &str) -> String {
    let lower = article.to_ascii_lowercase();
    let needle = format!("href=\"{}\"", href.to_ascii_lowercase());
    let Some(start) = lower.find(&needle) else {
        return String::new();
    };
    let Some(open_end_rel) = lower[start..].find('>') else {
        return String::new();
    };
    let open_end = start + open_end_rel;
    let Some(close_rel) = lower[open_end..].find("</a>") else {
        return String::new();
    };
    let close = open_end + close_rel;
    numeric_text(&html_to_text(&article[open_end + 1..close]))
}

fn extract_stars_today(article: &str) -> String {
    let lower = article.to_ascii_lowercase();
    let Some(marker) = lower
        .find("stars today")
        .or_else(|| lower.find("star today"))
    else {
        return String::new();
    };
    let prefix_start = marker.saturating_sub(220);
    let prefix = html_to_text(&article[prefix_start..marker]);
    last_numeric_token(&prefix)
}

fn numeric_text(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_ascii_digit() || *ch == ',' || *ch == '.' || *ch == 'k' || *ch == 'K')
        .collect::<String>()
}

fn last_numeric_token(text: &str) -> String {
    text.split_whitespace()
        .rev()
        .map(numeric_text)
        .find(|token| !token.is_empty())
        .unwrap_or_default()
}

pub fn open_web_search(query: &str, engine: Option<&str>) -> Result<OpenWebSearchResult, String> {
    let (engine, url) = build_search_url(query, engine)?;
    open_url_in_default_browser(&url)?;
    Ok(OpenWebSearchResult {
        query: query.trim().to_string(),
        engine,
        url,
        opened: true,
    })
}

pub async fn fetch_web_page(url: &str, max_chars: Option<usize>) -> Result<WebPageExtract, String> {
    let parsed = validate_public_http_url(url)?;
    let client = web_client()?;
    let response = send_public_get(&client, parsed.clone()).await?;
    if !response.status().is_success() {
        return Err(format!(
            "Web page request failed with status {}",
            response.status()
        ));
    }

    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !content_type.is_empty()
        && !content_type.contains("text/")
        && !content_type.contains("html")
        && !content_type.contains("xml")
    {
        return Err(format!("Unsupported web content type: {content_type}"));
    }

    let raw = read_response_limited(response, MAX_WEB_RESPONSE_BYTES).await?;
    let title = extract_title(&raw);
    let description = extract_meta_description(&raw);
    let mut text = if content_type.contains("text/plain") {
        collapse_whitespace(&raw)
    } else {
        html_to_text(&raw)
    };
    let max_chars = max_chars
        .unwrap_or(DEFAULT_TEXT_CHARS)
        .clamp(1_000, MAX_TEXT_CHARS);
    let original_chars = text.chars().count();
    let truncated = original_chars > max_chars;
    if truncated {
        text = text.chars().take(max_chars).collect();
    }
    let chars = text.chars().count();

    Ok(WebPageExtract {
        url: parsed.to_string(),
        final_url,
        title,
        description,
        text,
        chars,
        truncated,
    })
}

pub async fn github_trending_repositories(
    language: Option<&str>,
    since: Option<&str>,
    limit: usize,
) -> Result<GithubTrendingResponse, String> {
    let since = normalize_trending_since(since);
    let limit = limit.clamp(1, 25);
    let source_url = github_trending_url(language, &since);
    let client = web_client()?;
    let response = send_public_get(&client, validate_public_http_url(&source_url)?).await?;
    if !response.status().is_success() {
        return Err(format!(
            "GitHub Trending request failed with status {}",
            response.status()
        ));
    }

    let final_url = response.url().to_string();
    let html = read_response_limited(response, MAX_WEB_RESPONSE_BYTES).await?;
    let mut repositories = parse_github_trending_repositories(&html, limit);
    repositories.truncate(limit);
    let count = repositories.len();
    if count == 0 {
        return Err("GitHub Trending page did not contain parseable repository rows.".to_string());
    }

    Ok(GithubTrendingResponse {
        source_url: final_url,
        since,
        repositories,
        count,
    })
}

pub fn search_browser_history(
    query: &str,
    browser: Option<&str>,
    limit: usize,
) -> Result<BrowserHistorySearchResult, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("Browser history search keyword cannot be empty.".to_string());
    }

    let browser = normalize_browser_filter(browser);
    let sources = discover_browser_history_sources(&browser);
    search_browser_history_sources(trimmed, &browser, limit, &sources)
}

fn search_browser_history_sources(
    query: &str,
    browser: &str,
    limit: usize,
    sources: &[BrowserHistorySource],
) -> Result<BrowserHistorySearchResult, String> {
    let limit = limit.clamp(1, 50);
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    let like = format!("%{}%", escape_like(query));

    for source in sources {
        let temp_path = std::env::temp_dir().join(format!(
            "atlas_history_{}_{}.sqlite",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));

        match fs::copy(&source.path, &temp_path) {
            Ok(_) => {
                match read_history_copy(&temp_path, source, &like, limit) {
                    Ok(mut found) => items.append(&mut found),
                    Err(error) => {
                        skipped.push(format!("{} {}: {}", source.browser, source.profile, error))
                    }
                }
                let _ = fs::remove_file(&temp_path);
            }
            Err(error) => skipped.push(format!(
                "{} {}: failed to copy history database: {}",
                source.browser, source.profile, error
            )),
        }
    }

    items.sort_by_key(|e| std::cmp::Reverse(e.last_visit_chrome_time));
    items.truncate(limit);

    Ok(BrowserHistorySearchResult {
        query: query.to_string(),
        browser: browser.to_string(),
        count: items.len(),
        items,
        skipped,
        privacy_note: "Atlas only searches browser history when this tool or command is explicitly invoked; it does not watch current tabs or read page bodies from history.".to_string(),
    })
}

fn read_history_copy(
    path: &Path,
    source: &BrowserHistorySource,
    like: &str,
    limit: usize,
) -> Result<Vec<BrowserHistoryEntry>, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT url, COALESCE(NULLIF(title, ''), url) AS title, visit_count, last_visit_time
             FROM urls
             WHERE url LIKE ?1 ESCAPE '\\' OR title LIKE ?1 ESCAPE '\\'
             ORDER BY last_visit_time DESC
             LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![like, limit as i64], |row| {
            let chrome_time = row.get::<_, i64>(3).unwrap_or_default();
            Ok(BrowserHistoryEntry {
                browser: source.browser.clone(),
                profile: source.profile.clone(),
                url: row.get(0)?,
                title: row.get(1)?,
                visit_count: row.get::<_, i64>(2).unwrap_or_default(),
                last_visit_time: chrome_time_to_iso(chrome_time),
                last_visit_chrome_time: chrome_time,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|e| e.to_string())?);
    }
    Ok(items)
}

fn discover_browser_history_sources(browser: &str) -> Vec<BrowserHistorySource> {
    let Some(local_data) = dirs::data_local_dir() else {
        return Vec::new();
    };

    let candidates = [
        (
            "chrome",
            local_data.join("Google").join("Chrome").join("User Data"),
        ),
        (
            "edge",
            local_data.join("Microsoft").join("Edge").join("User Data"),
        ),
        (
            "brave",
            local_data
                .join("BraveSoftware")
                .join("Brave-Browser")
                .join("User Data"),
        ),
    ];

    let mut sources = Vec::new();
    for (name, user_data) in candidates {
        if browser != "all" && browser != name {
            continue;
        }
        collect_profile_history_sources(name, &user_data, &mut sources);
    }
    sources
}

fn collect_profile_history_sources(
    browser: &str,
    user_data: &Path,
    sources: &mut Vec<BrowserHistorySource>,
) {
    if !user_data.is_dir() {
        return;
    }

    let Ok(entries) = fs::read_dir(user_data) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let profile = entry.file_name().to_string_lossy().to_string();
        if profile != "Default" && !profile.starts_with("Profile ") {
            continue;
        }
        let path = entry.path().join("History");
        if path.is_file() {
            sources.push(BrowserHistorySource {
                browser: browser.to_string(),
                profile,
                path,
            });
        }
    }
}

fn open_url_in_default_browser(url: &str) -> Result<(), String> {
    let parsed = validate_public_http_url(url)?;
    let url = parsed.as_str();

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut cmd = Command::new("rundll32.exe");
        cmd.args(["url.dll,FileProtocolHandler", url]);
        cmd
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut cmd = Command::new("open");
        cmd.arg(url);
        cmd
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(url);
        cmd
    };

    command
        .spawn()
        .map_err(|e| format!("Failed to open default browser: {e}"))?;
    Ok(())
}

fn web_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(Policy::none())
        .user_agent("Atlas/0.1 web tools")
        .build()
        .map_err(|e| e.to_string())
}

async fn send_public_get(client: &Client, initial_url: Url) -> Result<reqwest::Response, String> {
    let mut current = initial_url;
    for _ in 0..=5 {
        let response = client
            .get(current.clone())
            .send()
            .await
            .map_err(|e| format!("Web request failed: {e}"))?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| "Redirect response is missing Location header.".to_string())?;
            current = validate_redirect_target(&current, location)?;
            continue;
        }
        return Ok(response);
    }
    Err("Too many web redirects.".to_string())
}

fn validate_redirect_target(current: &Url, location: &str) -> Result<Url, String> {
    let next = current
        .join(location)
        .map_err(|e| format!("Invalid redirect URL: {e}"))?;
    validate_public_http_url(next.as_str())
}

async fn read_response_limited(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<String, String> {
    let mut data = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if data.len() + chunk.len() > max_bytes {
            let remaining = max_bytes.saturating_sub(data.len());
            data.extend_from_slice(&chunk[..remaining]);
            break;
        }
        data.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&data).into_owned())
}

fn validate_public_http_url(raw: &str) -> Result<Url, String> {
    let parsed = Url::parse(raw.trim()).map_err(|e| format!("Invalid URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only http and https URLs are allowed.".to_string());
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| "URL must include a host.".to_string())?
        .to_ascii_lowercase();
    let allow_local_smoke_fetch = allow_local_web_smoke_fetch();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        if allow_local_smoke_fetch {
            return Ok(parsed);
        }
        return Err("Local browser/web URLs are blocked for this tool.".to_string());
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if allow_local_smoke_fetch && is_loopback_ip(ip) {
            return Ok(parsed);
        }
        if is_private_ip(ip) {
            return Err("Private or loopback network URLs are blocked for this tool.".to_string());
        }
    }

    Ok(parsed)
}

fn allow_local_web_smoke_fetch() -> bool {
    std::env::var("ATLAS_SMOKE_ALLOW_LOCAL_WEB_FETCH")
        .ok()
        .as_deref()
        == Some("1")
        && std::env::var("ATLAS_SMOKE_RUN_ID")
            .ok()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        && std::env::var("ATLAS_HOME")
            .ok()
            .map(|value| value.to_ascii_lowercase().contains("tauri-smoke"))
            .unwrap_or(false)
}

fn is_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback(),
    }
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip == Ipv4Addr::UNSPECIFIED
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || is_unique_local_v6(ip)
                || is_unicast_link_local_v6(ip)
        }
    }
}

fn is_unique_local_v6(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

fn is_unicast_link_local_v6(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

fn normalize_engine(engine: Option<&str>) -> String {
    match engine
        .unwrap_or("duckduckgo")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "google" => "google".to_string(),
        "bing" => "bing".to_string(),
        "baidu" => "baidu".to_string(),
        _ => "duckduckgo".to_string(),
    }
}

fn normalize_browser_filter(browser: Option<&str>) -> String {
    match browser
        .unwrap_or("all")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "chrome" => "chrome".to_string(),
        "edge" => "edge".to_string(),
        "brave" => "brave".to_string(),
        _ => "all".to_string(),
    }
}

fn parse_duckduckgo_results(html: &str, limit: usize) -> Vec<WebSearchItem> {
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    let mut results = Vec::new();

    while results.len() < limit {
        let Some(marker_rel) = lower[cursor..].find("result__a") else {
            break;
        };
        let marker = cursor + marker_rel;
        let tag_start = lower[..marker].rfind("<a").unwrap_or(marker);
        let Some(tag_end_rel) = lower[marker..].find('>') else {
            break;
        };
        let tag_end = marker + tag_end_rel;
        let Some(close_rel) = lower[tag_end..].find("</a>") else {
            break;
        };
        let close = tag_end + close_rel;
        let tag = &html[tag_start..=tag_end];
        let title = html_to_text(&html[tag_end + 1..close]);
        let href = attr_value(tag, "href").unwrap_or_default();
        let url = normalize_result_url(&href);

        if !title.is_empty() && !url.is_empty() {
            let snippet = find_result_snippet(html, &lower, close);
            results.push(WebSearchItem {
                title,
                url,
                snippet,
            });
        }
        cursor = close + 4;
    }

    results
}

fn find_result_snippet(html: &str, lower: &str, start: usize) -> String {
    let next_result = lower[start..]
        .find("result__a")
        .map(|offset| start + offset)
        .unwrap_or(html.len());
    let area = &lower[start..next_result];
    let Some(marker_rel) = area.find("result__snippet") else {
        return String::new();
    };
    let marker = start + marker_rel;
    let Some(tag_end_rel) = lower[marker..].find('>') else {
        return String::new();
    };
    let tag_end = marker + tag_end_rel;
    let Some(close_rel) = lower[tag_end..].find("</") else {
        return String::new();
    };
    let close = tag_end + close_rel;
    html_to_text(&html[tag_end + 1..close])
}

fn parse_duckduckgo_lite_results(html: &str, limit: usize) -> Vec<WebSearchItem> {
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    let mut results = Vec::new();

    while results.len() < limit {
        let Some(marker_rel) = lower[cursor..].find("result-link") else {
            break;
        };
        let marker = cursor + marker_rel;
        let tag_start = lower[..marker].rfind("<a").unwrap_or(marker);
        let Some(tag_end_rel) = lower[marker..].find('>') else {
            break;
        };
        let tag_end = marker + tag_end_rel;
        let Some(close_rel) = lower[tag_end..].find("</a>") else {
            break;
        };
        let close = tag_end + close_rel;
        let tag = &html[tag_start..=tag_end];
        let title = html_to_text(&html[tag_end + 1..close]);
        let href = attr_value(tag, "href").unwrap_or_default();
        let url = normalize_result_url(&href);

        if !title.is_empty() && !url.is_empty() {
            let snippet = find_lite_result_snippet(html, &lower, close);
            results.push(WebSearchItem {
                title,
                url,
                snippet,
            });
        }
        cursor = close + 4;
    }

    results
}

fn find_lite_result_snippet(html: &str, lower: &str, start: usize) -> String {
    let next_result = lower[start..]
        .find("result-link")
        .map(|offset| start + offset)
        .unwrap_or(html.len());
    let area = &lower[start..next_result];
    let Some(marker_rel) = area.find("result-snippet") else {
        return String::new();
    };
    let marker = start + marker_rel;
    let Some(tag_end_rel) = lower[marker..].find('>') else {
        return String::new();
    };
    let tag_end = marker + tag_end_rel;
    let Some(close_rel) = lower[tag_end..].find("</td>") else {
        return String::new();
    };
    let close = tag_end + close_rel;
    html_to_text(&html[tag_end + 1..close])
}

fn normalize_result_url(href: &str) -> String {
    let href = decode_html_entities(href.trim());
    let candidate = if href.starts_with("//") {
        format!("https:{href}")
    } else if href.starts_with('/') {
        format!("https://duckduckgo.com{href}")
    } else {
        href
    };

    if let Ok(url) = Url::parse(&candidate) {
        if url
            .host_str()
            .map(|host| host.ends_with("duckduckgo.com"))
            .unwrap_or(false)
        {
            if let Some((_, value)) = url.query_pairs().find(|(key, _)| key == "uddg") {
                return value.into_owned();
            }
        }
        if matches!(url.scheme(), "http" | "https") {
            return url.to_string();
        }
    }
    String::new()
}

fn extract_title(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("<title") else {
        return String::new();
    };
    let Some(open_end_rel) = lower[start..].find('>') else {
        return String::new();
    };
    let open_end = start + open_end_rel;
    let Some(close_rel) = lower[open_end..].find("</title>") else {
        return String::new();
    };
    let close = open_end + close_rel;
    html_to_text(&html[open_end + 1..close])
}

fn extract_meta_description(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(start_rel) = lower[cursor..].find("<meta") {
        let start = cursor + start_rel;
        let Some(end_rel) = lower[start..].find('>') else {
            break;
        };
        let end = start + end_rel;
        let tag = &html[start..=end];
        let tag_lower = &lower[start..=end];
        if (tag_lower.contains("name=\"description\"")
            || tag_lower.contains("name='description'")
            || tag_lower.contains("property=\"og:description\"")
            || tag_lower.contains("property='og:description'"))
            && attr_value(tag, "content").is_some()
        {
            return attr_value(tag, "content")
                .map(|value| collapse_whitespace(&decode_html_entities(&value)))
                .unwrap_or_default();
        }
        cursor = end + 1;
    }
    String::new()
}

fn attr_value(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    for quote in ['"', '\''] {
        let pattern = format!("{attr}={quote}");
        if let Some(start) = lower.find(&pattern) {
            let value_start = start + pattern.len();
            if let Some(end_rel) = tag[value_start..].find(quote) {
                return Some(decode_html_entities(
                    &tag[value_start..value_start + end_rel],
                ));
            }
        }
    }
    None
}

fn html_to_text(html: &str) -> String {
    let without_blocks =
        remove_html_block_tags(html, &["head", "script", "style", "noscript", "svg"]);
    let without_comments = remove_html_comments(&without_blocks);
    let mut out = String::new();
    let mut in_tag = false;
    for ch in without_comments.chars() {
        match ch {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    collapse_whitespace(&decode_html_entities(&out))
}

fn remove_html_block_tags(input: &str, tags: &[&str]) -> String {
    let mut output = input.to_string();
    for tag in tags {
        loop {
            let lower = output.to_ascii_lowercase();
            let open = format!("<{tag}");
            let close = format!("</{tag}>");
            let Some(start) = lower.find(&open) else {
                break;
            };
            let Some(end_rel) = lower[start..].find(&close) else {
                output.replace_range(start..output.len(), " ");
                break;
            };
            let end = start + end_rel + close.len();
            output.replace_range(start..end, " ");
        }
    }
    output
}

fn remove_html_comments(input: &str) -> String {
    let mut output = input.to_string();
    while let Some(start) = output.find("<!--") {
        let Some(end_rel) = output[start..].find("-->") else {
            output.replace_range(start..output.len(), " ");
            break;
        };
        output.replace_range(start..start + end_rel + 3, " ");
    }
    output
}

fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_html_entities(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

fn percent_encode(input: &str) -> String {
    let mut out = String::new();
    for byte in input.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn chrome_time_to_iso(chrome_time: i64) -> Option<String> {
    if chrome_time <= 0 {
        return None;
    }
    let unix_seconds = chrome_time / 1_000_000 - 11_644_473_600;
    DateTime::<Utc>::from_timestamp(unix_seconds, 0).map(|dt| dt.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn build_search_url_encodes_query() {
        let (engine, url) = build_search_url("atlas browser 搜索", Some("bing")).unwrap();
        assert_eq!(engine, "bing");
        assert!(url.contains("atlas%20browser%20%E6%90%9C%E7%B4%A2"));
    }

    #[test]
    fn local_urls_are_blocked() {
        assert!(validate_public_http_url("http://localhost:5173").is_err());
        assert!(validate_public_http_url("http://127.0.0.1:5173").is_err());
        assert!(validate_public_http_url("file:///C:/secret.txt").is_err());
    }

    #[test]
    fn redirect_targets_are_revalidated() {
        let current = Url::parse("https://example.com/start").unwrap();
        assert!(validate_redirect_target(&current, "https://example.org/next").is_ok());
        assert!(validate_redirect_target(&current, "http://127.0.0.1:5173/private").is_err());
        assert!(validate_redirect_target(&current, "http://192.168.1.2/private").is_err());
    }

    #[test]
    fn html_extraction_ignores_scripts_and_reads_title() {
        let html = r#"
          <html><head><title>Atlas &amp; Web</title><meta name="description" content="Local first"></head>
          <body><script>hidden()</script><main><h1>Hello</h1><p>Visible text.</p></main></body></html>
        "#;
        assert_eq!(extract_title(html), "Atlas & Web");
        assert_eq!(extract_meta_description(html), "Local first");
        assert_eq!(html_to_text(html), "Hello Visible text.");
    }

    #[test]
    fn parses_duckduckgo_result_fixture() {
        let html = r#"
          <div class="result">
            <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fatlas&amp;rut=abc">Atlas Result</a>
            <a class="result__snippet">A useful snippet &amp; context.</a>
          </div>
        "#;
        let results = parse_duckduckgo_results(html, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Atlas Result");
        assert_eq!(results[0].url, "https://example.com/atlas");
        assert_eq!(results[0].snippet, "A useful snippet & context.");
    }

    #[test]
    fn parses_duckduckgo_lite_result_fixture() {
        let html = r#"
          <tr><td><a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fatlas-lite&amp;rut=abc" class='result-link'>Atlas Lite Result</a></td></tr>
          <tr><td class='result-snippet'>Lite snippet &amp; context.</td></tr>
        "#;
        let results = parse_duckduckgo_lite_results(html, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Atlas Lite Result");
        assert_eq!(results[0].url, "https://example.com/atlas-lite");
        assert_eq!(results[0].snippet, "Lite snippet & context.");
    }

    #[test]
    fn adds_direct_github_trending_result_for_daily_hot_project_queries() {
        let results = direct_search_results("今天 GitHub 最火的项目");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://github.com/trending?since=daily");
    }

    #[test]
    fn parses_github_trending_article_fixture() {
        let html = r#"
          <article class="Box-row">
            <h2 class="h3 lh-condensed">
              <a href="/owner/repo-name" class="Link">owner / repo-name</a>
            </h2>
            <p class="col-9 color-fg-muted my-1">Useful repo description &amp; context.</p>
            <div class="f6 color-fg-muted mt-2">
              <span itemprop="programmingLanguage">Rust</span>
              <a href="/owner/repo-name/stargazers"> 12,345</a>
              <a href="/owner/repo-name/forks"> 678</a>
              <span><svg></svg> 91 stars today</span>
            </div>
          </article>
        "#;
        let repos = parse_github_trending_repositories(html, 5);
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].full_name, "owner/repo-name");
        assert_eq!(repos[0].url, "https://github.com/owner/repo-name");
        assert_eq!(repos[0].description, "Useful repo description & context.");
        assert_eq!(repos[0].language.as_deref(), Some("Rust"));
        assert_eq!(repos[0].stars, "12,345");
        assert_eq!(repos[0].forks, "678");
        assert_eq!(repos[0].stars_today, "91");
    }

    #[tokio::test]
    #[ignore]
    async fn real_github_trending_page_parses_repositories() {
        let result = github_trending_repositories(None, Some("daily"), 5)
            .await
            .unwrap();
        assert_eq!(result.since, "daily");
        assert!(result.source_url.starts_with("https://github.com/trending"));
        assert!(!result.repositories.is_empty());
        assert!(result.repositories[0]
            .url
            .starts_with("https://github.com/"));
    }

    #[test]
    fn searches_history_sqlite_copy() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("atlas_history_test_{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("History");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE urls (
              id INTEGER PRIMARY KEY,
              url TEXT NOT NULL,
              title TEXT,
              visit_count INTEGER,
              last_visit_time INTEGER
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO urls (url, title, visit_count, last_visit_time) VALUES (?1, ?2, ?3, ?4)",
            params![
                "https://example.com/atlas-browser",
                "Atlas Browser Notes",
                3,
                13_300_000_000_000_000_i64
            ],
        )
        .unwrap();
        drop(conn);

        let sources = vec![BrowserHistorySource {
            browser: "chrome".to_string(),
            profile: "Default".to_string(),
            path: db_path,
        }];
        let result = search_browser_history_sources("browser", "chrome", 10, &sources).unwrap();
        assert_eq!(result.count, 1);
        assert_eq!(result.items[0].title, "Atlas Browser Notes");
        assert_eq!(result.items[0].browser, "chrome");
        let _ = fs::remove_dir_all(dir);
    }
}
