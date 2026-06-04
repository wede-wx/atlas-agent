//! P1-5: one shared output-size limiter for tool results.
//!
//! Large tool output — file contents, git output, command output — must be
//! bounded before it reaches the model context or the UI, and every tool should
//! bound it the *same* way: keep the head **and** the tail (both ends carry
//! signal — a source file's imports and its end; a command log's start and the
//! error at its tail), mark how much was elided, and report consistent truncation
//! metadata so the model and the UI read the same shape regardless of which tool
//! produced the output.

use serde::{Deserialize, Serialize};

/// Default per-tool output cap, in characters. ~16k chars keeps a single tool
/// result well within the context budget while still surfacing substantial
/// content. Individual tools may pass a different cap (e.g. live command output).
pub const MAX_TOOL_OUTPUT_CHARS: usize = 16_000;

/// The outcome of bounding one piece of tool output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TruncatedOutput {
    /// Text to surface: the original when it fits, otherwise head + tail joined by
    /// an elision marker.
    pub text: String,
    /// Whether any content was dropped.
    pub truncated: bool,
    /// Length of the original text, in characters.
    pub original_chars: usize,
    /// Characters kept (length of `text`, including the marker).
    pub kept_chars: usize,
}

impl TruncatedOutput {
    /// Consistent JSON metadata block tools embed as `"truncation"`, so the model
    /// and UI read one shape no matter which tool produced the output.
    pub fn meta(&self) -> serde_json::Value {
        serde_json::json!({
            "truncated": self.truncated,
            "originalChars": self.original_chars,
            "keptChars": self.kept_chars,
        })
    }
}

/// Bound `text` to roughly `max_chars` characters, keeping the head and the tail
/// and inserting a marker stating how much was elided. Returns the text unchanged
/// when it already fits. Character-based throughout so multi-byte (CJK) content is
/// never split mid-codepoint.
pub fn truncate_middle(text: &str, max_chars: usize) -> TruncatedOutput {
    let chars: Vec<char> = text.chars().collect();
    let original_chars = chars.len();
    if original_chars <= max_chars {
        return TruncatedOutput {
            text: text.to_string(),
            truncated: false,
            original_chars,
            kept_chars: original_chars,
        };
    }

    let dropped = original_chars - max_chars;
    let marker = format!("\n…[已省略 {dropped} 字符，原文共 {original_chars} 字符]…\n");
    let marker_chars = marker.chars().count();
    let budget = max_chars.saturating_sub(marker_chars);
    // Degenerate cap smaller than the marker itself: hard-cut the head.
    if budget == 0 {
        let head: String = chars[..max_chars].iter().collect();
        return TruncatedOutput {
            text: head,
            truncated: true,
            original_chars,
            kept_chars: max_chars,
        };
    }

    // Head-heavy split (the head usually carries more structure) with a meaningful
    // tail retained.
    let head_len = (budget * 3) / 5;
    let tail_len = budget - head_len;
    let head: String = chars[..head_len].iter().collect();
    let tail: String = chars[original_chars - tail_len..].iter().collect();
    let combined = format!("{head}{marker}{tail}");
    let kept_chars = combined.chars().count();
    TruncatedOutput {
        text: combined,
        truncated: true,
        original_chars,
        kept_chars,
    }
}

/// Bound an arbitrary JSON tool payload destined for the model context, returning
/// a ready-to-use `data` object with one consistent shape across tools:
/// - within `max_chars`: the original object, plus a `truncation` meta block;
/// - over `max_chars`: `{ truncated, preview, truncation }`, where `preview` is the
///   head+tail-bounded serialization (structure is sacrificed only when oversized).
///
/// Use this for list/aggregate results (search hits, trending rows, MCP output)
/// that have no single dominant text field. Tools with one big text field (file
/// content, page body) should instead bound that field directly with
/// [`truncate_middle`] and attach `out.meta()` as `data["truncation"]`.
pub fn bounded_tool_data(payload: serde_json::Value, max_chars: usize) -> serde_json::Value {
    let out = truncate_middle(&payload.to_string(), max_chars);
    if out.truncated {
        return serde_json::json!({
            "truncated": true,
            "preview": out.text,
            "truncation": out.meta(),
        });
    }
    let mut data = payload;
    match data.as_object_mut() {
        Some(object) => {
            object.insert("truncation".to_string(), out.meta());
            data
        }
        // Non-object payload (array/scalar) that fits: wrap so the meta still rides along.
        None => serde_json::json!({ "value": data, "truncation": out.meta() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_truncation_when_within_cap() {
        let out = truncate_middle("hello", 100);
        assert!(!out.truncated);
        assert_eq!(out.text, "hello");
        assert_eq!(out.original_chars, 5);
        assert_eq!(out.kept_chars, 5);
    }

    #[test]
    fn truncate_keeps_head_and_tail_and_marks_elision() {
        let text = format!("HEAD_START{}TAIL_END", "x".repeat(1_000));
        let out = truncate_middle(&text, 200);
        assert!(out.truncated);
        assert!(out.text.starts_with("HEAD_START"), "head preserved");
        assert!(out.text.ends_with("TAIL_END"), "tail preserved");
        assert!(out.text.contains("已省略"), "elision marker present");
        assert_eq!(out.original_chars, text.chars().count());
        assert!(
            out.kept_chars <= 200,
            "kept within the cap, got {}",
            out.kept_chars
        );
    }

    #[test]
    fn meta_has_consistent_shape() {
        let out = truncate_middle(&"x".repeat(500), 100);
        let meta = out.meta();
        assert_eq!(meta["truncated"], serde_json::json!(true));
        assert_eq!(meta["originalChars"], serde_json::json!(500));
        assert!(meta["keptChars"].as_u64().unwrap() <= 100);
    }

    #[test]
    fn bounded_tool_data_keeps_structure_and_adds_meta_within_cap() {
        let payload = serde_json::json!({ "results": [1, 2, 3], "count": 3 });
        let data = bounded_tool_data(payload, 10_000);
        // Structure preserved...
        assert_eq!(data["count"], serde_json::json!(3));
        assert_eq!(data["results"], serde_json::json!([1, 2, 3]));
        // ...with a consistent truncation meta attached.
        assert_eq!(data["truncation"]["truncated"], serde_json::json!(false));
        assert!(data.get("preview").is_none());
    }

    #[test]
    fn bounded_tool_data_replaces_with_preview_when_oversized() {
        let big = serde_json::json!({ "blob": "x".repeat(5_000) });
        let data = bounded_tool_data(big, 200);
        assert_eq!(data["truncated"], serde_json::json!(true));
        assert!(data["preview"].as_str().unwrap().contains("已省略"));
        assert_eq!(data["truncation"]["truncated"], serde_json::json!(true));
        assert!(
            data["preview"].as_str().unwrap().chars().count() <= 200,
            "preview within cap"
        );
        // Original structure is gone (only a bounded preview survives).
        assert!(data.get("blob").is_none());
    }

    #[test]
    fn bounded_tool_data_wraps_non_object_payload() {
        let data = bounded_tool_data(serde_json::json!([1, 2, 3]), 10_000);
        assert_eq!(data["value"], serde_json::json!([1, 2, 3]));
        assert_eq!(data["truncation"]["truncated"], serde_json::json!(false));
    }

    #[test]
    fn multibyte_content_is_not_split_mid_codepoint() {
        // All-CJK input: truncation must stay on char boundaries (Vec<char>, not
        // byte slicing) — no panic, valid UTF-8, head and tail both CJK.
        let text = "锦".repeat(1_000);
        let out = truncate_middle(&text, 100);
        assert!(out.truncated);
        assert!(out.text.starts_with('锦'), "head is CJK");
        assert!(out.text.ends_with('锦'), "tail is CJK");
        assert!(out.text.contains("已省略"), "elision marker present");
    }
}
