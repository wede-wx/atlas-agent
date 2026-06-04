use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeIntelligenceRequest {
    pub workspace_root: String,
    #[serde(default)]
    pub document_path: Option<String>,
    #[serde(default)]
    pub lsp_diagnostics: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LspBackendStatus {
    pub available: bool,
    pub backend: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticSummary {
    pub uri: String,
    pub severity: String,
    pub message: String,
    pub line: i64,
    pub character: i64,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SymbolQuery {
    pub query: String,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceHit {
    pub uri: String,
    pub line: i64,
    pub character: i64,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIntelligenceReport {
    pub workspace_root: String,
    pub document_path: Option<String>,
    pub backend: LspBackendStatus,
    pub diagnostics: Vec<DiagnosticSummary>,
    pub bounded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LspSessionSpec {
    pub workspace_root: String,
    pub language: String,
    #[serde(default)]
    pub server_command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LspSessionPlan {
    pub workspace_root: String,
    pub language: String,
    pub command: String,
    pub backend: LspBackendStatus,
    pub bounded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LspLocation {
    pub uri: String,
    pub line: i64,
    pub character: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RenamePreview {
    pub file_count: usize,
    pub edit_count: usize,
    pub locations: Vec<LspLocation>,
}

pub fn inspect_code_intelligence(
    request: CodeIntelligenceRequest,
) -> Result<CodeIntelligenceReport, String> {
    let root = canonical_workspace_root(&request.workspace_root)?;
    let document_path = if let Some(path) = request.document_path.as_deref() {
        Some(validate_workspace_path(&root, path)?)
    } else {
        None
    };
    let diagnostics = request
        .lsp_diagnostics
        .as_ref()
        .map(parse_lsp_diagnostics)
        .unwrap_or_default()
        .into_iter()
        .take(200)
        .collect::<Vec<_>>();
    Ok(CodeIntelligenceReport {
        workspace_root: root.to_string_lossy().to_string(),
        document_path: document_path.map(|path| path.to_string_lossy().to_string()),
        backend: LspBackendStatus {
            available: request.lsp_diagnostics.is_some(),
            backend: if request.lsp_diagnostics.is_some() {
                "lsp-diagnostics-payload".to_string()
            } else {
                "not-started".to_string()
            },
            reason: if request.lsp_diagnostics.is_some() {
                "diagnostics parsed from real LSP-shaped payload".to_string()
            } else {
                "no language-server session is running; Aura does not fake diagnostics from grep"
                    .to_string()
            },
        },
        diagnostics,
        bounded: true,
    })
}

pub fn prepare_lsp_session(spec: LspSessionSpec) -> Result<LspSessionPlan, String> {
    let root = canonical_workspace_root(&spec.workspace_root)?;
    let language = normalize_language(&spec.language);
    let command = spec
        .server_command
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| default_lsp_command(&language).to_string());
    let available = command_available(&command);
    let reason = if available {
        "language-server command found on PATH; session may be started by the LSP host".to_string()
    } else {
        "language-server command was not found on PATH; Aura will not fake LSP results".to_string()
    };
    Ok(LspSessionPlan {
        workspace_root: root.to_string_lossy().to_string(),
        language,
        command: command.clone(),
        backend: LspBackendStatus {
            available,
            backend: command,
            reason,
        },
        bounded: true,
    })
}

pub fn parse_lsp_diagnostics(payload: &Value) -> Vec<DiagnosticSummary> {
    let uri = payload
        .get("uri")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    payload
        .get("diagnostics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|diagnostic| diagnostic_from_lsp(&uri, diagnostic))
        .collect()
}

pub fn parse_lsp_locations(payload: &Value) -> Vec<LspLocation> {
    if let Some(items) = payload.as_array() {
        return items.iter().filter_map(location_from_lsp).collect();
    }
    payload
        .get("locations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(location_from_lsp)
        .collect()
}

pub fn rename_preview_from_lsp(payload: &Value) -> RenamePreview {
    let mut locations = Vec::new();
    let mut file_count = 0usize;
    if let Some(changes) = payload.get("changes").and_then(Value::as_object) {
        file_count += changes.len();
        for (uri, edits) in changes {
            if let Some(edits) = edits.as_array() {
                for edit in edits {
                    if let Some(mut location) = location_from_text_edit(uri, edit) {
                        location.uri = uri.clone();
                        locations.push(location);
                    }
                }
            }
        }
    }
    if let Some(document_changes) = payload.get("documentChanges").and_then(Value::as_array) {
        for change in document_changes {
            let uri = change
                .get("textDocument")
                .and_then(|value| value.get("uri"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            file_count += usize::from(uri != "unknown");
            if let Some(edits) = change.get("edits").and_then(Value::as_array) {
                for edit in edits {
                    if let Some(mut location) = location_from_text_edit(uri, edit) {
                        location.uri = uri.to_string();
                        locations.push(location);
                    }
                }
            }
        }
    }
    locations.truncate(500);
    RenamePreview {
        file_count,
        edit_count: locations.len(),
        locations,
    }
}

fn diagnostic_from_lsp(uri: &str, diagnostic: &Value) -> Option<DiagnosticSummary> {
    let range = diagnostic.get("range")?;
    let start = range.get("start")?;
    let severity = match diagnostic
        .get("severity")
        .and_then(Value::as_i64)
        .unwrap_or(3)
    {
        1 => "error",
        2 => "warning",
        3 => "information",
        4 => "hint",
        _ => "unknown",
    };
    Some(DiagnosticSummary {
        uri: uri.to_string(),
        severity: severity.to_string(),
        message: diagnostic
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(500)
            .collect(),
        line: start.get("line").and_then(Value::as_i64).unwrap_or(0),
        character: start.get("character").and_then(Value::as_i64).unwrap_or(0),
        source: diagnostic
            .get("source")
            .and_then(Value::as_str)
            .map(|value| value.to_string()),
    })
}

fn location_from_lsp(value: &Value) -> Option<LspLocation> {
    let uri = value.get("uri").and_then(Value::as_str)?.to_string();
    let range = value.get("range")?;
    let start = range.get("start")?;
    Some(LspLocation {
        uri,
        line: start.get("line").and_then(Value::as_i64).unwrap_or(0),
        character: start.get("character").and_then(Value::as_i64).unwrap_or(0),
    })
}

fn location_from_text_edit(uri: &str, value: &Value) -> Option<LspLocation> {
    let range = value.get("range")?;
    let start = range.get("start")?;
    Some(LspLocation {
        uri: uri.to_string(),
        line: start.get("line").and_then(Value::as_i64).unwrap_or(0),
        character: start.get("character").and_then(Value::as_i64).unwrap_or(0),
    })
}

fn canonical_workspace_root(root: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(root.trim());
    if root.trim().is_empty() {
        return Err("workspace root is empty".to_string());
    }
    path.canonicalize()
        .map_err(|error| format!("workspace root cannot be resolved: {error}"))
}

fn validate_workspace_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    let raw = PathBuf::from(path.trim());
    let candidate = if raw.is_absolute() {
        raw
    } else {
        root.join(raw)
    }
    .canonicalize()
    .map_err(|error| format!("document path cannot be resolved: {error}"))?;
    if !path_starts_with(&candidate, root) {
        return Err("document path is outside workspace root".to_string());
    }
    Ok(candidate)
}

fn path_starts_with(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let path_text = path.to_string_lossy().to_ascii_lowercase();
        let root_text = root.to_string_lossy().to_ascii_lowercase();
        path_text == root_text
            || path_text.starts_with(&format!(
                "{}{}",
                root_text.trim_end_matches(['\\', '/']),
                std::path::MAIN_SEPARATOR
            ))
    }
    #[cfg(not(windows))]
    {
        path.starts_with(root)
    }
}

fn normalize_language(language: &str) -> String {
    match language.trim().to_ascii_lowercase().as_str() {
        "rust" | "rs" => "rust".to_string(),
        "typescript" | "ts" | "tsx" => "typescript".to_string(),
        "javascript" | "js" | "jsx" => "javascript".to_string(),
        "python" | "py" => "python".to_string(),
        other if !other.is_empty() => other.to_string(),
        _ => "unknown".to_string(),
    }
}

fn default_lsp_command(language: &str) -> &'static str {
    match language {
        "rust" => "rust-analyzer",
        "typescript" | "javascript" => "typescript-language-server",
        "python" => "pyright-langserver",
        _ => "language-server",
    }
}

fn command_available(command: &str) -> bool {
    let command = command
        .split_whitespace()
        .next()
        .unwrap_or(command)
        .trim_matches('"');
    if command.is_empty() {
        return false;
    }
    let candidate = PathBuf::from(command);
    if candidate.is_absolute() || command.contains(std::path::MAIN_SEPARATOR) {
        return candidate.is_file();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|path| command_exists_in_dir(&path, command))
}

fn command_exists_in_dir(dir: &Path, command: &str) -> bool {
    #[cfg(windows)]
    {
        let pathext = std::env::var_os("PATHEXT")
            .map(|value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .map(|item| item.trim().to_ascii_lowercase())
                    .filter(|item| !item.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![".exe".to_string(), ".cmd".to_string(), ".bat".to_string()]);
        let command_lower = command.to_ascii_lowercase();
        if pathext.iter().any(|ext| command_lower.ends_with(ext)) {
            return dir.join(command).is_file();
        }
        pathext
            .iter()
            .any(|ext| dir.join(format!("{command}{ext}")).is_file())
    }
    #[cfg(not(windows))]
    {
        dir.join(command).is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("aura-code-intel-{label}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn parses_real_lsp_diagnostics_payload_without_grep_fallback() {
        let root = temp_root("diag");
        let file = root.join("main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();
        let report = inspect_code_intelligence(CodeIntelligenceRequest {
            workspace_root: root.to_string_lossy().to_string(),
            document_path: Some(file.to_string_lossy().to_string()),
            lsp_diagnostics: Some(json!({
                "uri": "file:///main.rs",
                "diagnostics": [{
                    "range": { "start": { "line": 2, "character": 4 } },
                    "severity": 1,
                    "message": "cannot find value",
                    "source": "rust-analyzer"
                }]
            })),
        })
        .unwrap();
        assert!(report.backend.available);
        assert_eq!(report.diagnostics[0].severity, "error");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_document_outside_workspace() {
        let root = temp_root("root");
        let outside = temp_root("outside");
        let file = outside.join("main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();
        let error = inspect_code_intelligence(CodeIntelligenceRequest {
            workspace_root: root.to_string_lossy().to_string(),
            document_path: Some(file.to_string_lossy().to_string()),
            lsp_diagnostics: None,
        })
        .unwrap_err();
        assert!(error.contains("outside workspace"));
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn lsp_session_plan_reports_missing_backend_without_fake_results() {
        let root = temp_root("session");
        let plan = prepare_lsp_session(LspSessionSpec {
            workspace_root: root.to_string_lossy().to_string(),
            language: "rust".to_string(),
            server_command: Some("definitely-missing-aura-lsp".to_string()),
        })
        .unwrap();
        assert!(!plan.backend.available);
        assert!(plan.backend.reason.contains("will not fake"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn parses_locations_and_rename_preview_from_lsp_payloads() {
        let locations = parse_lsp_locations(&json!({
            "locations": [{
                "uri": "file:///main.rs",
                "range": { "start": { "line": 3, "character": 2 } }
            }]
        }));
        assert_eq!(locations[0].line, 3);
        let preview = rename_preview_from_lsp(&json!({
            "changes": {
                "file:///main.rs": [{
                    "range": { "start": { "line": 4, "character": 1 } },
                    "newText": "renamed"
                }]
            }
        }));
        assert_eq!(preview.file_count, 1);
        assert_eq!(preview.edit_count, 1);
        assert_eq!(preview.locations[0].character, 1);
    }
}
