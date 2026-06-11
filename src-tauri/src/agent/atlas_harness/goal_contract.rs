//! Atlas Harness — Goal Contract data model（地基 / cornerstone）。
//!
//! 背景：现有架构里 `agent/contract.rs` 是“工具使用契约”（TaskIntent / ToolUseDecision），
//! 不是“目标契约”。Atlas Skill 产出的 Goal Contract（goal / must_do / must_not_do /
//! preserve）目前**只以对话文本存在**，harness 无法对它做机械校验。
//!
//! 本文件给 Goal Contract 一个**结构化、可持久化、可机械比对**的表示，并提供一个
//! 从 Skill 文本块解析成结构体的 parser（中英双语标签）。这是 ContractGate /
//! ImpactEvidenceGate / Verifier / CompletionGate 全部依赖的地基。

use serde::{Deserialize, Serialize};

/// 单条契约项（Must Do / Must Not Do / Constraints / Acceptance）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractItem {
    /// 稳定 ID：M1 / N1 / C1 / A1 ...
    pub id: String,
    pub text: String,
    /// 硬性 = 不可在无披露的情况下改动；软性 = 可在披露后权衡。
    pub hard: bool,
    /// 用户原话片段（抗压缩/抗改写的锚点）。
    #[serde(default)]
    pub source_quote: Option<String>,
    /// 如何验证这一项（命令 / 测试 / 可观察检查）。
    #[serde(default)]
    pub verify: Option<String>,
}

/// Preserve 项的类别——决定 ContractGate 用什么方式比对。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreserveKind {
    /// 既有行为（不能破坏正在工作的东西）。
    Behavior,
    /// 参考图/既有设计的**布局结构**（Reference Drift 的修复点）。
    LayoutStructure,
    /// 公共 API 形状 / 响应语义。
    ApiContract,
    /// 范围边界（不能擅自扩大/缩小）。
    Scope,
    /// 数据：schema / enum / 持久化 / 统计。
    Data,
    /// 具体文件或路径（最强、最机械的一类）。
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreserveItem {
    pub id: String,
    pub text: String,
    pub kind: PreserveKind,
    /// 当 kind = File/LayoutStructure 时，可选的路径 glob（如 `src/ui/**`）。
    /// ContractGate 用它对 proposed action 的 target_path 做结构匹配。
    #[serde(default)]
    pub path_glob: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScopeBoundaries {
    pub in_scope: Vec<String>,
    pub out_of_scope: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReferenceFidelity {
    pub has_reference: bool,
    /// 必须匹配的布局结构（元素顺序、栅格、导航位置…）。
    pub layout_structure: Vec<String>,
    /// 风格（配色、字体观感…）。
    pub style: Vec<String>,
    /// 冲突时布局优先于风格——Reference Drift 的核心约束。
    pub layout_over_style: bool,
}

/// 结构化目标契约。由 Skill 文本解析而来，确认后冻结，作为 harness 的执行基线。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalContract {
    pub goal: String,
    pub must_do: Vec<ContractItem>,
    pub must_not_do: Vec<ContractItem>,
    pub preserve: Vec<PreserveItem>,
    pub constraints: Vec<ContractItem>,
    pub acceptance_criteria: Vec<ContractItem>,
    pub scope: ScopeBoundaries,
    pub reference_fidelity: ReferenceFidelity,
    /// 用户确认后置 true；冻结后只能通过 Deviation Notice 修改。
    pub frozen: bool,
}

impl GoalContract {
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// 是否存在任何硬性约束——决定一个 session 是否需要 ContractGate 全程开启。
    pub fn has_hard_constraints(&self) -> bool {
        self.must_do.iter().any(|i| i.hard)
            || self.must_not_do.iter().any(|i| i.hard)
            || !self.preserve.is_empty()
            || self.constraints.iter().any(|i| i.hard)
    }

    /// 默认 must_not_do——即使用户没写，也预装这些“背叛”防线。
    /// 这些条目在解析后被注入（除非用户显式放开）。
    pub fn inject_default_guards(&mut self) {
        let defaults = [
            (
                "N-hide",
                "未经披露不得隐藏/移除/禁用/注释掉/stub 用户要求的功能",
            ),
            (
                "N-downgrade",
                "未经披露不得缩小范围（如 full-stack 降成 frontend-only）",
            ),
            (
                "N-mock",
                "未经披露不得用 mock/占位实现替换真实实现并声称完成",
            ),
            ("N-layout", "未经披露不得替换用户要求的布局结构"),
            ("N-test", "未经披露不得删除/弱化保护契约项的测试或断言"),
        ];
        for (id, text) in defaults {
            if !self.must_not_do.iter().any(|i| i.id == id) {
                self.must_not_do.push(ContractItem {
                    id: id.to_string(),
                    text: text.to_string(),
                    hard: true,
                    source_quote: None,
                    verify: None,
                });
            }
        }
    }

    /// 从 Atlas Skill 输出的 Goal Contract 文本块解析（中英双语标签）。
    ///
    /// 解析策略：行式扫描，按本地化的 section 标签切段，逐条解析
    /// `- [M1] text (hard, source: "...", verify: ...)`。容错：标签缺失就跳过该段，
    /// 不抛错——解析失败时返回尽力而为的部分契约 + diagnostics，由调用方决定是否要求重述。
    pub fn parse_from_skill_block(text: &str) -> ParseResult {
        let mut c = GoalContract::default();
        let mut diags: Vec<String> = Vec::new();
        let mut section = Section::None;

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(next) = Section::detect(line) {
                section = next;
                // section 标题行里如果跟了内容（如 "Goal: xxx"）也尝试抓取
                if let (Section::Goal, Some(rest)) = (section, after_colon(line)) {
                    if !rest.is_empty() {
                        c.goal = rest.to_string();
                    }
                }
                continue;
            }
            match section {
                Section::Goal => {
                    if c.goal.is_empty() {
                        c.goal = strip_bullet(line).to_string();
                    }
                }
                Section::MustDo => push_item(&mut c.must_do, line, &mut diags),
                Section::MustNotDo => push_item(&mut c.must_not_do, line, &mut diags),
                Section::Constraints => push_item(&mut c.constraints, line, &mut diags),
                Section::Acceptance => push_item(&mut c.acceptance_criteria, line, &mut diags),
                Section::Preserve => push_preserve(&mut c.preserve, line, &mut diags),
                Section::InScope => c.scope.in_scope.push(strip_bullet(line).to_string()),
                Section::OutScope => c.scope.out_of_scope.push(strip_bullet(line).to_string()),
                Section::ReferenceLayout => {
                    c.reference_fidelity.has_reference = true;
                    c.reference_fidelity.layout_over_style = true;
                    c.reference_fidelity
                        .layout_structure
                        .push(strip_bullet(line).to_string());
                }
                Section::ReferenceStyle => {
                    c.reference_fidelity.has_reference = true;
                    c.reference_fidelity
                        .style
                        .push(strip_bullet(line).to_string());
                }
                Section::None => {}
            }
        }

        finalize_reference_fidelity(&mut c, text);
        if c.goal.is_empty() {
            diags.push("contract has no Goal line".to_string());
        }
        c.inject_default_guards();
        ParseResult {
            contract: c,
            diagnostics: diags,
        }
    }

    /// 契约结构化通道（Step 1）：从结构化 JSON（`atlas_freeze_goal_contract`
    /// 工具的参数）解析契约。这是对 REVIEW_FINDINGS 第 7 条的地基级修复——
    /// 不再从自由文本里刮标题，模型通过专用工具调用提交结构化契约。
    ///
    /// 语义与 `parse_from_skill_block` 严格对齐：容错（坏条目记 diagnostics
    /// 不抛错）、`hard` 拿不准就当 true、缺 id 自动补、最后注入默认 guards。
    /// 文本通道保留为后备（模型不调工具时仍按旧路径抽取）。
    pub fn from_structured(value: &serde_json::Value) -> ParseResult {
        let mut c = GoalContract::default();
        let mut diags: Vec<String> = Vec::new();

        c.goal = value
            .get("goal")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        if c.goal.is_empty() {
            diags.push("structured contract has no goal".to_string());
        }

        structured_items(value, "must_do", "M", &mut c.must_do, &mut diags);
        structured_items(value, "must_not_do", "N", &mut c.must_not_do, &mut diags);
        structured_items(value, "constraints", "C", &mut c.constraints, &mut diags);
        structured_items(
            value,
            "acceptance_criteria",
            "A",
            &mut c.acceptance_criteria,
            &mut diags,
        );
        structured_preserve(value, &mut c.preserve, &mut diags);

        // scope：顶层 in_scope/out_of_scope 与嵌套 scope.{in_scope,out_of_scope} 都认。
        let scope_node = value.get("scope");
        c.scope.in_scope = structured_strings(value, scope_node, "in_scope");
        c.scope.out_of_scope = structured_strings(value, scope_node, "out_of_scope");

        // reference fidelity：与文本通道一致，存在布局项即视为有参考，且布局优先。
        let reference_node = value
            .get("reference")
            .or_else(|| value.get("reference_fidelity"));
        c.reference_fidelity.layout_structure =
            structured_strings(value, reference_node, "layout_structure");
        c.reference_fidelity.style = structured_strings(value, reference_node, "style");
        c.reference_fidelity.has_reference = !c.reference_fidelity.layout_structure.is_empty()
            || !c.reference_fidelity.style.is_empty();
        c.reference_fidelity.layout_over_style = !c.reference_fidelity.layout_structure.is_empty();

        c.inject_default_guards();
        ParseResult {
            contract: c,
            diagnostics: diags,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParseResult {
    pub contract: GoalContract,
    pub diagnostics: Vec<String>,
}

impl ParseResult {
    /// 解析是否“足够干净”可以直接冻结使用。
    pub fn is_usable(&self) -> bool {
        !self.contract.goal.is_empty()
    }
}

// ---------------------------------------------------------------------------
// 内部：section 识别 + 行解析（中英双语）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Goal,
    MustDo,
    MustNotDo,
    Preserve,
    Constraints,
    Acceptance,
    InScope,
    OutScope,
    ReferenceLayout,
    ReferenceStyle,
}

impl Section {
    fn detect(line: &str) -> Option<Section> {
        if strip_bullet(line).starts_with('[') {
            return None;
        }
        let l = line.to_lowercase();
        let head = l.trim_end_matches(['：', ':']).trim();
        // 英文 + 中文标签都认。用 contains 容忍 "Must Do:" / "必须做：" / "## Must Do"
        let hit = |needles: &[&str]| needles.iter().any(|n| head.contains(n));
        if hit(&["goal", "目标"]) && !hit(&["non-goal", "非目标"]) {
            Some(Section::Goal)
        } else if hit(&["must not do", "must_not", "禁止做", "禁止"]) {
            Some(Section::MustNotDo)
        } else if hit(&["must do", "must_do", "必须做"]) {
            Some(Section::MustDo)
        } else if hit(&["preserve", "必须保留", "保留"]) {
            Some(Section::Preserve)
        } else if hit(&["acceptance", "completion check", "完成检查", "验收"]) {
            Some(Section::Acceptance)
        } else if hit(&["constraint", "约束"]) {
            Some(Section::Constraints)
        } else if hit(&["in scope", "in_scope", "范围内"]) {
            Some(Section::InScope)
        } else if hit(&["out of scope", "out_of_scope", "范围外", "超范围"]) {
            Some(Section::OutScope)
        } else if hit(&["layout", "布局", "结构"]) {
            Some(Section::ReferenceLayout)
        } else if hit(&["style", "风格", "样式"]) {
            Some(Section::ReferenceStyle)
        } else {
            None
        }
    }
}

fn after_colon(line: &str) -> Option<&str> {
    line.split_once([':', '：']).map(|(_, rest)| rest.trim())
}

fn strip_bullet(line: &str) -> &str {
    line.trim_start_matches(['-', '*', '•', ' ']).trim()
}

/// 解析 `[M1] text (hard, source: "...", verify: ...)` / `[N1] ...（硬性，来源："..."）`。
fn parse_item(line: &str) -> Option<ContractItem> {
    let body = strip_bullet(line);
    let (id, rest) = extract_id(body)?;
    // 拆出尾部括号里的元信息（英文/中文括号都认）
    let (text, meta) = split_meta(rest);
    let lower = meta.to_lowercase();
    let soft = lower.contains("soft") || meta.contains("软性");
    Some(ContractItem {
        id,
        text: text.trim().to_string(),
        // 默认按硬处理（“拿不准就当 hard”），除非明确标 soft。
        hard: !soft,
        source_quote: extract_quoted(meta, &["source:", "来源："]),
        verify: extract_field(meta, &["verify:", "验证："]),
    })
}

fn push_item(into: &mut Vec<ContractItem>, line: &str, diags: &mut Vec<String>) {
    match parse_item(line) {
        Some(item) => into.push(item),
        None => diags.push(format!("could not parse item: {line}")),
    }
}

fn push_preserve(into: &mut Vec<PreserveItem>, line: &str, diags: &mut Vec<String>) {
    let body = strip_bullet(line);
    match extract_id(body) {
        Some((id, rest)) => {
            let (text, meta) = split_meta(rest);
            let text = text.trim().to_string();
            let kind = infer_preserve_kind(&text, meta);
            let path_glob = if matches!(kind, PreserveKind::File | PreserveKind::LayoutStructure) {
                extract_path_glob(&text)
            } else {
                None
            };
            into.push(PreserveItem {
                id,
                text,
                kind,
                path_glob,
            });
        }
        None => diags.push(format!("could not parse preserve item: {line}")),
    }
}

fn extract_id(body: &str) -> Option<(String, &str)> {
    // 形如 [M1] / [N-hide] / [P1]
    let body = body.trim_start();
    if !body.starts_with('[') {
        return None;
    }
    let close = body.find(']')?;
    let id = body[1..close].trim().to_string();
    if id.is_empty() {
        return None;
    }
    Some((id, body[close + 1..].trim()))
}

fn split_meta(rest: &str) -> (&str, &str) {
    // 取最后一个 '(' 或 '（' 作为元信息起点
    let open = rest
        .rfind('(')
        .or_else(|| rest.rfind('（'))
        .unwrap_or(rest.len());
    (&rest[..open], &rest[open..])
}

fn extract_field(meta: &str, keys: &[&str]) -> Option<String> {
    let lower = meta.to_lowercase();
    for k in keys {
        if let Some(pos) = lower.find(&k.to_lowercase()) {
            let after = &meta[pos + k.len()..];
            let val = after
                .trim_start()
                .trim_start_matches([':', '：'])
                .split([',', '，', ')', '）'])
                .next()
                .unwrap_or("")
                .trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

fn extract_quoted(meta: &str, keys: &[&str]) -> Option<String> {
    // 优先抓引号内的原话
    for q in ['"', '“'] {
        if let Some(start) = meta.find(q) {
            let close = if q == '“' { '”' } else { '"' };
            if let Some(end_rel) = meta[start + q.len_utf8()..].find(close) {
                let s = &meta[start + q.len_utf8()..start + q.len_utf8() + end_rel];
                if !s.trim().is_empty() {
                    return Some(s.trim().to_string());
                }
            }
        }
    }
    extract_field(meta, keys)
}

/// 结构化通道：解析一个契约项数组（对象或纯字符串都容忍）。
/// 与文本通道同语义：text 缺失记 diagnostic 跳过；id 缺失按 `{前缀}{序号}` 补；
/// hard 缺失默认 true（“拿不准就当硬”）。
fn structured_items(
    value: &serde_json::Value,
    key: &str,
    id_prefix: &str,
    into: &mut Vec<ContractItem>,
    diags: &mut Vec<String>,
) {
    let Some(items) = value.get(key).and_then(|v| v.as_array()) else {
        return;
    };
    for (index, item) in items.iter().enumerate() {
        let (text, id, hard, source_quote, verify) = match item {
            serde_json::Value::String(text) => (text.trim().to_string(), None, true, None, None),
            serde_json::Value::Object(_) => (
                item.get("text")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .unwrap_or("")
                    .to_string(),
                item.get("id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string),
                item.get("hard").and_then(|v| v.as_bool()).unwrap_or(true),
                item.get("source_quote")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                item.get("verify")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            ),
            other => {
                diags.push(format!("could not parse structured {key} item: {other}"));
                continue;
            }
        };
        if text.is_empty() {
            diags.push(format!("structured {key} item {} has no text", index + 1));
            continue;
        }
        into.push(ContractItem {
            id: id.unwrap_or_else(|| format!("{id_prefix}{}", index + 1)),
            text,
            hard,
            source_quote,
            verify,
        });
    }
}

/// 结构化通道：解析 preserve 数组。kind 接受 snake_case 枚举字符串
/// （behavior/layout_structure/api_contract/scope/data/file），不认识的值
/// 回落到与文本通道相同的关键词推断；File/LayoutStructure 在缺 path_glob
/// 时尝试从 text 提取路径模式。
fn structured_preserve(
    value: &serde_json::Value,
    into: &mut Vec<PreserveItem>,
    diags: &mut Vec<String>,
) {
    let Some(items) = value.get("preserve").and_then(|v| v.as_array()) else {
        return;
    };
    for (index, item) in items.iter().enumerate() {
        let text = item
            .get("text")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        if text.is_empty() {
            diags.push(format!(
                "structured preserve item {} has no text",
                index + 1
            ));
            continue;
        }
        let kind_raw = item.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let kind =
            serde_json::from_value::<PreserveKind>(serde_json::Value::String(kind_raw.to_string()))
                .unwrap_or_else(|_| infer_preserve_kind(&text, kind_raw));
        let path_glob = item
            .get("path_glob")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .or_else(|| {
                if matches!(kind, PreserveKind::File | PreserveKind::LayoutStructure) {
                    extract_path_glob(&text)
                } else {
                    None
                }
            });
        into.push(PreserveItem {
            id: item
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("P{}", index + 1)),
            text,
            kind,
            path_glob,
        });
    }
}

/// 结构化通道：读字符串数组，顶层 key 与嵌套节点（scope/reference）下的同名
/// key 都认，顶层优先。
fn structured_strings(
    value: &serde_json::Value,
    nested: Option<&serde_json::Value>,
    key: &str,
) -> Vec<String> {
    let node = value
        .get(key)
        .or_else(|| nested.and_then(|nested| nested.get(key)));
    node.and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn finalize_reference_fidelity(contract: &mut GoalContract, source: &str) {
    if has_explicit_layout_priority(source) {
        contract.reference_fidelity.layout_over_style = true;
        return;
    }

    let has_layout_requirement = !contract.reference_fidelity.layout_structure.is_empty()
        || contract
            .preserve
            .iter()
            .any(|p| p.kind == PreserveKind::LayoutStructure);

    if contract.reference_fidelity.has_reference
        && has_layout_requirement
        && !contract.reference_fidelity.style.is_empty()
    {
        contract.reference_fidelity.layout_over_style = true;
    }
}

fn has_explicit_layout_priority(source: &str) -> bool {
    let l = source.to_lowercase();
    [
        "布局优先于风格",
        "结构优先于风格",
        "layout over style",
        "layout before style",
        "structure over style",
    ]
    .iter()
    .any(|needle| l.contains(needle))
}

fn infer_preserve_kind(text: &str, meta: &str) -> PreserveKind {
    let meta_lower = meta.to_lowercase();
    let meta_has = |n: &[&str]| n.iter().any(|x| meta_lower.contains(x) || meta.contains(x));
    if meta_has(&["behavior", "行为"]) {
        return PreserveKind::Behavior;
    }
    if meta_has(&["layout", "布局", "结构"]) {
        return PreserveKind::LayoutStructure;
    }
    if meta_has(&["api", "接口"]) {
        return PreserveKind::ApiContract;
    }
    if meta_has(&["scope", "范围"]) {
        return PreserveKind::Scope;
    }
    if meta_has(&["data", "数据"]) {
        return PreserveKind::Data;
    }
    if meta_has(&["file", "文件"]) {
        return PreserveKind::File;
    }

    let l = text.to_lowercase();
    let has = |n: &[&str]| n.iter().any(|x| l.contains(x) || text.contains(x));
    if has(&["layout", "布局", "栅格", "导航", "结构 ", "结构"]) {
        PreserveKind::LayoutStructure
    } else if has(&["api", "接口", "endpoint", "response"]) {
        PreserveKind::ApiContract
    } else if has(&[
        "schema",
        "enum",
        "枚举",
        "数据",
        "持久化",
        "统计",
        "dashboard",
    ]) {
        PreserveKind::Data
    } else if has(&["scope", "范围"]) {
        PreserveKind::Scope
    } else if has(&["文件", "path"]) || extract_path_glob(text).is_some() {
        PreserveKind::File
    } else {
        PreserveKind::Behavior
    }
}

fn extract_path_glob(text: &str) -> Option<String> {
    // 抓出形如 src/ui/** 或 path/to/file.rs 的 token
    text.split_whitespace()
        .map(clean_path_token)
        .find(|t| looks_like_path_token(t))
}

fn clean_path_token(token: &str) -> String {
    token
        .trim_matches([
            '"', '“', '”', '`', ',', '，', '.', '。', ')', '）', '(', '（',
        ])
        .to_string()
}

fn looks_like_path_token(token: &str) -> bool {
    let t = token.replace('\\', "/");
    if t.starts_with("./") || t.starts_with("../") || t.starts_with('/') {
        return true;
    }
    if (t.contains('*') || t.contains("**")) && t.contains('/') {
        return true;
    }
    let known_root = [
        "src/",
        "app/",
        "lib/",
        "docs/",
        "test/",
        "tests/",
        "components/",
        "packages/",
        "crates/",
    ]
    .iter()
    .any(|prefix| t.starts_with(prefix));
    if known_root {
        return true;
    }
    let known_ext = [
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".json", ".toml", ".md", ".css", ".html",
        ".yml", ".yaml",
    ]
    .iter()
    .any(|ext| t.ends_with(ext));
    t.contains('/') && known_ext
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_english_contract() {
        let text = r#"
Goal:
- Ship a working context-circle feature on the canvas

Must Do:
- [M1] Implement the context-circle feature (hard, source: "implement the context-circle feature", verify: visible in running app)

Must Not Do:
- [N1] Do not hide the feature (hard)

Preserve:
- [P1] Keep existing canvas pan/zoom behavior (behavior)

Acceptance:
- [C1] Feature is interactive in the running app
"#;
        let r = GoalContract::parse_from_skill_block(text);
        assert!(r.is_usable());
        let c = r.contract;
        assert!(c.goal.contains("context-circle"));
        assert_eq!(c.must_do.len(), 1);
        assert_eq!(c.must_do[0].id, "M1");
        assert!(c.must_do[0].hard);
        assert_eq!(
            c.must_do[0].source_quote.as_deref(),
            Some("implement the context-circle feature")
        );
        // 默认 guard 被注入
        assert!(c.must_not_do.iter().any(|i| i.id == "N-hide"));
        assert_eq!(c.preserve[0].kind, PreserveKind::Behavior);
    }

    #[test]
    fn parses_chinese_contract_and_reference() {
        let text = r#"
目标：
- 按参考图实现落地页

必须做：
- [M1] 完整实现页面（硬性，来源："完整实现"）

必须保留：
- [P1] 三栏栅格布局 src/ui/** （布局）

风格：
- 靛蓝配色
"#;
        let r = GoalContract::parse_from_skill_block(text);
        let c = r.contract;
        assert!(c.goal.contains("落地页"));
        assert_eq!(c.must_do[0].id, "M1");
        assert!(c.must_do[0].hard);
        assert_eq!(c.preserve[0].kind, PreserveKind::LayoutStructure);
        assert_eq!(c.preserve[0].path_glob.as_deref(), Some("src/ui/**"));
        assert!(c.reference_fidelity.has_reference);
        assert!(c.reference_fidelity.layout_over_style);
        assert_eq!(c.reference_fidelity.style, vec!["靛蓝配色".to_string()]);
    }

    #[test]
    fn soft_items_are_not_hard() {
        let text = "Goal:\n- x\nConstraints:\n- [C1] prefer tailwind if possible (soft)";
        let c = GoalContract::parse_from_skill_block(text).contract;
        assert!(!c.constraints[0].hard);
    }

    #[test]
    fn structured_parse_mirrors_text_channel_semantics() {
        // Step 1（契约结构化通道）：结构化入口与文本入口语义一致——
        // id 自动补、hard 默认 true、默认 guards 注入、布局优先。
        let args = serde_json::json!({
            "goal": "按参考图实现落地页",
            "must_do": [
                { "id": "M1", "text": "完整实现页面", "hard": true, "source_quote": "完整实现" },
                { "text": "补充移动端适配" }
            ],
            "constraints": [ { "text": "prefer tailwind if possible", "hard": false } ],
            "preserve": [
                { "id": "P1", "text": "三栏栅格布局 src/ui/**", "kind": "layout_structure", "path_glob": "src/ui/**" }
            ],
            "in_scope": ["src/ui"],
            "out_of_scope": ["backend"],
            "reference": { "layout_structure": ["三栏栅格"], "style": ["靛蓝配色"] }
        });
        let r = GoalContract::from_structured(&args);
        assert!(r.is_usable());
        let c = r.contract;
        assert_eq!(c.must_do[0].id, "M1");
        assert_eq!(c.must_do[0].source_quote.as_deref(), Some("完整实现"));
        assert_eq!(c.must_do[1].id, "M2", "missing id is auto-assigned");
        assert!(c.must_do[1].hard, "hard defaults to true");
        assert!(!c.constraints[0].hard);
        assert_eq!(c.preserve[0].kind, PreserveKind::LayoutStructure);
        assert_eq!(c.preserve[0].path_glob.as_deref(), Some("src/ui/**"));
        assert_eq!(c.scope.in_scope, vec!["src/ui".to_string()]);
        assert_eq!(c.scope.out_of_scope, vec!["backend".to_string()]);
        assert!(c.reference_fidelity.has_reference);
        assert!(c.reference_fidelity.layout_over_style);
        // 默认 guards 注入（背叛防线在结构化通道同样成立）。
        assert!(c.must_not_do.iter().any(|i| i.id == "N-hide"));
        assert!(c.has_hard_constraints());
        assert!(!c.frozen, "freeze happens at install, not at parse");
    }

    #[test]
    fn structured_parse_is_tolerant_and_diagnosed() {
        // 容错语义：缺 goal / 坏条目 / 不认识的 kind 都不抛错，记 diagnostics。
        let args = serde_json::json!({
            "must_do": [ { "hard": true }, "plain string item", 42 ],
            "preserve": [ { "text": "保持 布局 结构", "kind": "??" } ]
        });
        let r = GoalContract::from_structured(&args);
        assert!(!r.is_usable(), "missing goal must not be usable");
        assert!(r.diagnostics.iter().any(|d| d.contains("no goal")));
        assert!(r.diagnostics.iter().any(|d| d.contains("has no text")));
        assert!(r.diagnostics.iter().any(|d| d.contains("could not parse")));
        // 纯字符串条目被接受。
        assert_eq!(r.contract.must_do.len(), 1);
        assert_eq!(r.contract.must_do[0].text, "plain string item");
        // 不认识的 kind 回落到关键词推断（“布局/结构”→ LayoutStructure）。
        assert_eq!(r.contract.preserve[0].kind, PreserveKind::LayoutStructure);
    }

    #[test]
    fn structured_contract_round_trips_through_persistence_shape() {
        // 结构化解析 → 冻结 → serde 往返（A6 持久化形态）→ 字段完整。
        let args = serde_json::json!({
            "goal": "ship X",
            "preserve": [ { "text": "keep src/ui/**", "kind": "file", "path_glob": "src/ui/**" } ]
        });
        let mut contract = GoalContract::from_structured(&args).contract;
        contract.freeze();
        let json = serde_json::to_value(&contract).unwrap();
        let restored: GoalContract = serde_json::from_value(json).unwrap();
        assert_eq!(restored, contract);
        assert!(restored.frozen);
    }
}
