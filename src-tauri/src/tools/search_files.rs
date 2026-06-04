use async_trait::async_trait;

use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::tools::fs_scope;
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};
use std::path::PathBuf;

const MAX_VISITED: usize = 2_000;
const MAX_TEXT_BYTES: u64 = 256 * 1024;

pub struct SearchFilesTool {
    extra_roots: Vec<PathBuf>,
}

impl SearchFilesTool {
    pub fn new(extra_roots: Vec<PathBuf>) -> Self {
        Self {
            extra_roots: fs_scope::normalize_extra_roots(extra_roots),
        }
    }
}

impl Default for SearchFilesTool {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[async_trait]
impl Tool for SearchFilesTool {
    fn name(&self) -> &str {
        "search_files"
    }

    fn description(&self) -> &str {
        "Search file names and small text files under an allowed directory."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description:
                "Search file names and small text file contents under an allowed local directory."
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory to search"
                    },
                    "query": {
                        "type": "string",
                        "description": "Case-insensitive text to search in file names and small text files"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results, default 30"
                    }
                },
                "required": ["path", "query"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "搜索文件".to_string(),
            description_zh: "在允许目录内搜索文件名和小文本文件内容，不修改文件。".to_string(),
            capability_labels_zh: vec!["文件系统".to_string(), "只读".to_string()],
            safety_label_zh: "安全".to_string(),
            capabilities: vec![ToolCapability::Filesystem, ToolCapability::ReadOnly],
            safety_level: ToolSafetyLevel::Safe,
            mutates_state: false,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| AgentError::Tool("缺少 path 参数。".to_string()))?;
        let query = args["query"]
            .as_str()
            .ok_or_else(|| AgentError::Tool("缺少 query 参数。".to_string()))?
            .trim()
            .to_ascii_lowercase();
        if query.is_empty() {
            return Err(AgentError::Tool("搜索关键词不能为空。".to_string()));
        }
        let limit = args
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(30)
            .clamp(1, 100) as usize;
        let root = fs_scope::allowed_directory_with_roots(path, &self.extra_roots)?;
        let mut results = Vec::new();
        let mut visited = 0usize;
        search_dir(&root, &query, limit, &mut visited, &mut results)?;

        Ok(ToolResult::success(
            format!("搜索完成，找到 {} 条结果。", results.len()),
            serde_json::json!({
                "root": root.to_string_lossy(),
                "query": query,
                "results": results,
                "visited": visited
            }),
        ))
    }
}

fn search_dir(
    path: &std::path::Path,
    query: &str,
    limit: usize,
    visited: &mut usize,
    results: &mut Vec<serde_json::Value>,
) -> Result<(), AgentError> {
    if *visited >= MAX_VISITED || results.len() >= limit {
        return Ok(());
    }
    if fs_scope::is_sensitive_path(path) {
        return Ok(());
    }
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        if *visited >= MAX_VISITED || results.len() >= limit {
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if fs_scope::is_sensitive_path(&path) {
            continue;
        }
        if entry
            .file_type()
            .map(|file_type| file_type.is_symlink())
            .unwrap_or(false)
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        *visited += 1;
        if name.to_ascii_lowercase().contains(query) {
            results.push(serde_json::json!({
                "path": path.to_string_lossy(),
                "matchType": "name",
                "name": name
            }));
            if results.len() >= limit {
                break;
            }
        }
        if path.is_dir() {
            search_dir(&path, query, limit, visited, results)?;
        } else if path.is_file() && looks_text_file(&path) {
            if let Ok(metadata) = std::fs::metadata(&path) {
                if metadata.len() <= MAX_TEXT_BYTES {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        if let Some(line) = find_line(&text, query) {
                            results.push(serde_json::json!({
                                "path": path.to_string_lossy(),
                                "matchType": "content",
                                "line": line
                            }));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn find_line(text: &str, query: &str) -> Option<String> {
    text.lines()
        .find(|line| line.to_ascii_lowercase().contains(query))
        .map(|line| line.chars().take(400).collect())
}

fn looks_text_file(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "txt"
            | "md"
            | "json"
            | "csv"
            | "log"
            | "toml"
            | "yaml"
            | "yml"
            | "rs"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "css"
            | "html"
    )
}
