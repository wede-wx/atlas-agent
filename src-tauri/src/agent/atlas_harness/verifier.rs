//! Atlas Harness — Independent Verifier（独立对抗式审查）。
//!
//! 原理:做事、并且下了那个自信错判的 agent,正是最不该复查它的人(它会替自己走过的路辩护)。
//! 在每个 phase/task 标 done 前,用一个**新上下文**的 reviewer——它只拿到
//! `GoalContract + diff + 测试证据`,**没有**实现过程的 trajectory,因此没有维护“我做得对”的动机。
//! reviewer 被 prompt 成“假设实现者偏了,去把偏在哪找出来”,对抗式而非验收式。
//!
//! INTEGRATION:用现有 `team_runtime.rs` 起一个只读角色 "atlas-verifier"(沿用 Reviewer 的
//! can't-write 权限),把 build_review_prompt() 作为它的输入,输出按 parse_verdict() 解析。

use super::goal_contract::GoalContract;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Hard,
    Soft,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deviation {
    pub contract_item_id: String,
    pub description: String,
    pub severity: Severity,
    /// 支撑证据(diff 行号 / 测试名 / 缺失项)。
    pub evidence: String,
}

#[derive(Debug, Clone)]
pub enum VerifierVerdict {
    Pass,
    Deviations(Vec<Deviation>),
}

impl VerifierVerdict {
    /// 有任何 hard deviation → 必须阻止 done。
    pub fn blocks_completion(&self) -> bool {
        matches!(self, VerifierVerdict::Deviations(ds) if ds.iter().any(|d| d.severity == Severity::Hard))
    }
}

/// 构造对抗式审查 prompt。刻意要求“假设偏了”,并强制逐条对照契约项 + 强制给证据。
pub fn build_review_prompt(contract: &GoalContract, diff: &str, test_evidence: &str) -> String {
    let mut p = String::new();
    p.push_str(
        "你是 Atlas 独立审查员。你**没有**参与这次实现,不需要替任何结果辩护。\n\
         假设实现者在执行中偏离了目标——你的任务是把偏在哪**找出来**,而不是确认它做得对。\n\
         逐条对照下面的 Goal Contract。对每一条:它是被真正满足了,还是被静默地降级/隐藏/替换/绕过了?\n\
         任何“看起来完成但缺证据”的项,记为 deviation。不要因为 diff 看起来整洁就放过。\n\n",
    );
    p.push_str("== Goal ==\n");
    p.push_str(&contract.goal);
    p.push_str("\n\n== Must Do ==\n");
    for m in &contract.must_do {
        p.push_str(&format!("- [{}] {}\n", m.id, m.text));
    }
    p.push_str("== Must Not Do ==\n");
    for n in &contract.must_not_do {
        p.push_str(&format!("- [{}] {}\n", n.id, n.text));
    }
    p.push_str("== Preserve ==\n");
    for pr in &contract.preserve {
        p.push_str(&format!("- [{}] {} ({:?})\n", pr.id, pr.text, pr.kind));
    }
    if contract.reference_fidelity.has_reference {
        p.push_str("== Reference layout (必须匹配,布局优先于风格) ==\n");
        for l in &contract.reference_fidelity.layout_structure {
            p.push_str(&format!("- {l}\n"));
        }
    }
    p.push_str("\n== 实际改动 (diff) ==\n");
    p.push_str(diff);
    p.push_str("\n\n== 验证证据 ==\n");
    p.push_str(if test_evidence.trim().is_empty() {
        "(无验证证据)"
    } else {
        test_evidence
    });
    p.push_str(
        "\n\n只输出 JSON(无其他文字):\n\
         {\"verdict\":\"pass\"|\"deviations\",\
         \"deviations\":[{\"contract_item_id\":\"...\",\"description\":\"...\",\
         \"severity\":\"hard\"|\"soft\",\"evidence\":\"...\"}]}\n\
         若任何 Must Do 缺证据或 Preserve/Must Not Do 被触碰,verdict 必须是 deviations。",
    );
    p
}

/// 解析 reviewer 的 JSON 输出为 verdict。容错:解析失败按“保守阻断”处理。
pub fn parse_verdict(reviewer_output: &str) -> VerifierVerdict {
    #[derive(Deserialize)]
    struct Raw {
        verdict: String,
        #[serde(default)]
        deviations: Vec<Deviation>,
    }
    // 抽出第一个 JSON 对象(reviewer 可能裹了 ```json)
    let json = extract_json(reviewer_output);
    match serde_json::from_str::<Raw>(&json) {
        Ok(r) if r.verdict == "pass" && r.deviations.is_empty() => VerifierVerdict::Pass,
        Ok(r) => VerifierVerdict::Deviations(r.deviations),
        Err(_) => VerifierVerdict::Deviations(vec![Deviation {
            contract_item_id: "verifier".into(),
            description: "审查员输出无法解析,按保守策略阻断完成".into(),
            severity: Severity::Hard,
            evidence: reviewer_output.chars().take(200).collect(),
        }]),
    }
}

fn extract_json(s: &str) -> String {
    let start = s.find('{');
    let end = s.rfind('}');
    match (start, end) {
        (Some(a), Some(b)) if b > a => s[a..=b].to_string(),
        _ => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::atlas_harness::goal_contract::GoalContract;

    #[test]
    fn prompt_includes_contract_and_is_adversarial() {
        let c = GoalContract::parse_from_skill_block("Goal:\n- x\nMust Do:\n- [M1] do A (hard)\n")
            .contract;
        let p = build_review_prompt(&c, "diff here", "");
        assert!(p.contains("假设实现者"));
        assert!(p.contains("[M1] do A"));
        assert!(p.contains("(无验证证据)"));
    }

    #[test]
    fn parses_pass_and_deviations() {
        assert!(matches!(
            parse_verdict(r#"{"verdict":"pass","deviations":[]}"#),
            VerifierVerdict::Pass
        ));
        let v = parse_verdict(
            r#"prefix ```json {"verdict":"deviations","deviations":[{"contract_item_id":"M1","description":"only frontend","severity":"hard","evidence":"no backend file"}]} ``` suffix"#,
        );
        assert!(v.blocks_completion());
    }

    #[test]
    fn unparseable_blocks_conservatively() {
        assert!(parse_verdict("garbage").blocks_completion());
    }
}
