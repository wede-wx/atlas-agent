//! P2-4: structured working memory for the current run. Tools write into it as
//! they execute; a compact summary is injected before each model call (see
//! `core::handle_chat`) so the agent knows what it has already read / edited /
//! run / failed and avoids repeating ineffective reads.
//!
//! Archival (plan step 3): every tool call is already persisted to the run
//! timeline (P1-3 五源), so WorkingMemory is the *in-run injection view* over
//! that same data rather than a second persisted store — keeping it DRY with the
//! timeline instead of duplicating a working_memory table.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Max entries per category surfaced in the injected summary (context budget).
const MAX_PER_CATEGORY: usize = 20;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingMemory {
    pub read_files: Vec<String>,
    pub edited_files: Vec<String>,
    pub ran_commands: Vec<String>,
    pub failures: Vec<String>,
}

impl WorkingMemory {
    /// Record one tool call. `failed` marks a tool result that errored, tracked
    /// separately so the model can avoid repeating a failing action. Entries are
    /// de-duplicated so a re-read of the same file doesn't grow the list.
    pub fn record(&mut self, tool_name: &str, args: &Value, failed: bool) {
        match tool_name {
            "read_file" => {
                if let Some(p) = path_arg(args) {
                    push_unique(&mut self.read_files, p);
                }
            }
            "write_file" | "edit_file" => {
                if let Some(p) = path_arg(args) {
                    push_unique(&mut self.edited_files, p);
                }
            }
            "run_command" => {
                if let Some(c) = args.get("command").and_then(Value::as_str) {
                    let c = c.trim();
                    if !c.is_empty() {
                        push_unique(&mut self.ran_commands, c.to_string());
                    }
                }
            }
            _ => {}
        }
        if failed {
            push_unique(&mut self.failures, format!("{tool_name} 失败"));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.read_files.is_empty()
            && self.edited_files.is_empty()
            && self.ran_commands.is_empty()
            && self.failures.is_empty()
    }

    /// Build a compact summary note to inject before a model call. Returns `None`
    /// when nothing has been recorded yet (avoids an empty system message).
    pub fn summary_note(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut lines =
            vec!["[工作记忆] 本次运行已经做过以下事，避免重复无意义的读取/操作：".to_string()];
        if !self.read_files.is_empty() {
            lines.push(format!("- 已读文件：{}", join_capped(&self.read_files)));
        }
        if !self.edited_files.is_empty() {
            lines.push(format!("- 已改文件：{}", join_capped(&self.edited_files)));
        }
        if !self.ran_commands.is_empty() {
            lines.push(format!("- 已跑命令：{}", join_capped(&self.ran_commands)));
        }
        if !self.failures.is_empty() {
            lines.push(format!("- 失败过：{}", join_capped(&self.failures)));
        }
        Some(lines.join("\n"))
    }
}

fn path_arg(args: &Value) -> Option<String> {
    args.get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

fn join_capped(items: &[String]) -> String {
    let shown = items.len().min(MAX_PER_CATEGORY);
    let mut s = items[..shown].join("、");
    if items.len() > MAX_PER_CATEGORY {
        s.push_str(&format!("…(共 {} 项)", items.len()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn records_and_classifies_tool_calls() {
        let mut wm = WorkingMemory::default();
        wm.record("read_file", &json!({"path": "src/a.rs"}), false);
        wm.record("read_file", &json!({"path": "src/a.rs"}), false); // dup ignored
        wm.record("write_file", &json!({"path": "src/b.rs"}), false);
        wm.record("edit_file", &json!({"path": "src/c.rs"}), false);
        wm.record("run_command", &json!({"command": "cargo test"}), false);
        wm.record("run_command", &json!({"command": "boom"}), true); // failed

        assert_eq!(wm.read_files, vec!["src/a.rs"]);
        assert_eq!(wm.edited_files, vec!["src/b.rs", "src/c.rs"]);
        assert_eq!(wm.ran_commands, vec!["cargo test", "boom"]);
        assert!(wm.failures.iter().any(|f| f.contains("run_command")));
    }

    #[test]
    fn summary_note_none_when_empty_and_lists_when_populated() {
        assert!(WorkingMemory::default().summary_note().is_none());

        let mut wm = WorkingMemory::default();
        wm.record("read_file", &json!({"path": "x.rs"}), false);
        let note = wm.summary_note().expect("note");
        assert!(
            note.contains("[工作记忆]") && note.contains("已读文件") && note.contains("x.rs"),
            "note: {note}"
        );
    }

    #[test]
    fn summary_caps_long_lists() {
        let mut wm = WorkingMemory::default();
        for i in 0..(MAX_PER_CATEGORY + 5) {
            wm.record("read_file", &json!({ "path": format!("f{i}.rs") }), false);
        }
        let note = wm.summary_note().unwrap();
        assert!(
            note.contains(&format!("共 {} 项", MAX_PER_CATEGORY + 5)),
            "note: {note}"
        );
    }
}
