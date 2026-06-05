use crate::agent::capabilities::{
    canonical_provider_id, resolve_capabilities, ProviderCapabilities,
};
use crate::agent::provider_economics::{
    list_model_quality_events, quality_penalty_for_model, ModelQualityEvent,
};
use crate::config::{Config, ModelConnectionConfig};
use crate::storage::LocalDb;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

const MODEL_ROUTE_DECISIONS_KEY: &str = "model_route_decisions:v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRouteTier {
    Economy,
    Balanced,
    Strong,
    LongContext,
}

impl ModelRouteTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Economy => "economy",
            Self::Balanced => "balanced",
            Self::Strong => "strong",
            Self::LongContext => "long_context",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRoutePolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub force_connection_id: Option<String>,
    #[serde(default)]
    pub preferred_tier: Option<ModelRouteTier>,
}

impl Default for ModelRoutePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            force_connection_id: None,
            preferred_tier: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRouteRequest {
    pub agent_mode: String,
    pub user_input: String,
    pub history_message_count: usize,
    pub estimated_input_chars: usize,
    pub estimated_input_tokens: u32,
    pub attachment_count: usize,
    pub image_attachment_count: usize,
    pub needs_vision: bool,
    pub needs_tools: bool,
}

impl ModelRouteRequest {
    pub fn new(
        agent_mode: impl Into<String>,
        user_input: impl Into<String>,
        history_message_count: usize,
        history_chars: usize,
        attachment_count: usize,
        image_attachment_count: usize,
        needs_tools: bool,
    ) -> Self {
        let user_input = user_input.into();
        let input_chars = user_input.chars().count().saturating_add(history_chars);
        Self {
            agent_mode: agent_mode.into(),
            user_input,
            history_message_count,
            estimated_input_chars: input_chars,
            estimated_input_tokens: estimate_tokens_from_chars(input_chars),
            attachment_count,
            image_attachment_count,
            needs_vision: image_attachment_count > 0,
            needs_tools,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelRouteCandidate {
    pub connection_id: String,
    pub provider_id: String,
    pub model: String,
    pub protocol: String,
    pub enabled: bool,
    pub ready: bool,
    pub cost_rank: u8,
    pub quality_rank: u8,
    pub max_context: u32,
    pub vision: bool,
    pub tool_calls: bool,
    pub capability_source: String,
    #[serde(default)]
    pub quality_penalty_points: i64,
    #[serde(default)]
    pub quality_recommendation: String,
    pub reject_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelRouteDecision {
    pub enabled: bool,
    pub tier: ModelRouteTier,
    pub selected_connection_id: Option<String>,
    pub selected_provider_id: Option<String>,
    pub selected_model: Option<String>,
    pub original_connection_id: Option<String>,
    pub original_provider_id: Option<String>,
    pub original_model: Option<String>,
    pub needs_vision: bool,
    pub needs_tools: bool,
    pub estimated_input_tokens: u32,
    pub reason_codes: Vec<String>,
    pub candidates: Vec<ModelRouteCandidate>,
    /// P3-3: ordered fallback chain (connection ids). The first entry is the
    /// selected connection; the rest are eligible alternates by route score, to
    /// be tried in order when an earlier connection fails at call time.
    #[serde(default)]
    pub fallback_chain: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelRouteDecisionAudit {
    pub id: String,
    pub tier: ModelRouteTier,
    pub selected_connection_id: Option<String>,
    pub selected_provider_id: Option<String>,
    pub selected_model: Option<String>,
    pub reason_codes: Vec<String>,
    pub candidate_count: usize,
    pub created_at: i64,
}

impl ModelRouteDecision {
    pub fn selected_connection<'a>(&self, config: &'a Config) -> Option<&'a ModelConnectionConfig> {
        let id = self.selected_connection_id.as_deref()?;
        config
            .llm
            .connections
            .iter()
            .find(|connection| connection.id == id)
    }

    pub fn apply_to_config(&self, config: &mut Config) {
        let Some(selected) = self.selected_connection(config).cloned() else {
            return;
        };
        config.llm.default_connection_id = Some(selected.id.clone());
        config.llm.default_provider = selected.provider_id.clone();
        config.llm.sync_legacy_slots_from_connections();
    }

    pub fn summary(&self) -> String {
        let selected = match (&self.selected_provider_id, &self.selected_model) {
            (Some(provider), Some(model)) => format!("{provider}/{model}"),
            _ => "未选择".to_string(),
        };
        let original = match (&self.original_provider_id, &self.original_model) {
            (Some(provider), Some(model)) => format!("{provider}/{model}"),
            _ => "无原始模型".to_string(),
        };
        format!(
            "模型路由：tier={}，selected={}，original={}，reason={}",
            self.tier.as_str(),
            selected,
            original,
            self.reason_codes.join(",")
        )
    }
}

pub fn select_model_route(
    config: &Config,
    db: &LocalDb,
    request: &ModelRouteRequest,
    policy: &ModelRoutePolicy,
) -> ModelRouteDecision {
    let original = config.llm.active_connection();
    let original_connection_id = original.map(|connection| connection.id.clone());
    let tier = policy
        .preferred_tier
        .unwrap_or_else(|| classify_route_tier(request));
    let quality_events = list_model_quality_events(db).unwrap_or_default();
    let candidates = config
        .llm
        .connections
        .iter()
        .map(|connection| route_candidate(connection, db, request, tier, &quality_events))
        .collect::<Vec<_>>();

    if !policy.enabled {
        return fixed_decision(
            false,
            tier,
            request,
            original,
            candidates,
            vec!["routing_disabled".to_string()],
        );
    }

    if let Some(forced) = policy
        .force_connection_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| {
                candidate.connection_id == forced && candidate.enabled && candidate.ready
            })
            .cloned()
        {
            // A user-forced connection is pinned on purpose: do not silently fall
            // back to a different one. The chain is just the forced connection.
            let forced_chain = vec![candidate.connection_id.clone()];
            return decision_from_candidate(
                true,
                tier,
                request,
                original,
                &candidate,
                candidates,
                vec!["user_forced_connection".to_string()],
                forced_chain,
            );
        }
        let mut reasons = vec!["user_forced_connection_unavailable".to_string()];
        reasons.push(format!("forceConnectionId={forced}"));
        return fixed_decision(true, tier, request, original, candidates, reasons);
    }

    let eligible = candidates
        .iter()
        .filter(|candidate| candidate.reject_reasons.is_empty())
        .collect::<Vec<_>>();

    if eligible.is_empty() {
        let mut reasons = vec!["no_eligible_route_kept_active".to_string()];
        if request.needs_vision {
            reasons.push("needs_vision".to_string());
        }
        if request.needs_tools {
            reasons.push("needs_tools".to_string());
        }
        return fixed_decision(true, tier, request, original, candidates, reasons);
    }

    let selected = eligible
        .into_iter()
        .max_by_key(|candidate| route_score(candidate, tier, original_connection_id.as_deref()))
        .cloned()
        .expect("eligible not empty");

    let mut reasons = vec![format!("tier={}", tier.as_str())];
    match tier {
        ModelRouteTier::Economy => reasons.push("prefer_low_cost".to_string()),
        ModelRouteTier::Balanced => reasons.push("balance_cost_and_quality".to_string()),
        ModelRouteTier::Strong => reasons.push("prefer_high_quality".to_string()),
        ModelRouteTier::LongContext => reasons.push("prefer_large_context".to_string()),
    }
    if request.needs_vision {
        reasons.push("requires_vision".to_string());
    }
    if request.needs_tools {
        reasons.push("requires_tool_calls".to_string());
    }
    let fallback_chain =
        eligible_fallback_chain(&candidates, tier, original_connection_id.as_deref());
    decision_from_candidate(
        true,
        tier,
        request,
        original,
        &selected,
        candidates,
        reasons,
        fallback_chain,
    )
}

pub fn record_model_route_decision(
    db: &LocalDb,
    decision: &ModelRouteDecision,
) -> Result<ModelRouteDecisionAudit, String> {
    let audit = ModelRouteDecisionAudit {
        id: format!("mrd_{}", Uuid::new_v4()),
        tier: decision.tier,
        selected_connection_id: decision.selected_connection_id.clone(),
        selected_provider_id: decision.selected_provider_id.clone(),
        selected_model: decision.selected_model.clone(),
        reason_codes: decision.reason_codes.clone(),
        candidate_count: decision.candidates.len(),
        created_at: chrono::Utc::now().timestamp_millis(),
    };
    let mut records = list_model_route_decisions(db)?;
    records.push(audit.clone());
    if records.len() > 300 {
        records = records.split_off(records.len() - 300);
    }
    db.set_app_state(MODEL_ROUTE_DECISIONS_KEY, json!(records))
        .map_err(|error| error.to_string())?;
    Ok(audit)
}

pub fn list_model_route_decisions(db: &LocalDb) -> Result<Vec<ModelRouteDecisionAudit>, String> {
    let value = db
        .get_app_state(MODEL_ROUTE_DECISIONS_KEY)
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| json!([]));
    serde_json::from_value(value).map_err(|error| error.to_string())
}

pub fn classify_route_tier(request: &ModelRouteRequest) -> ModelRouteTier {
    let text = request.user_input.to_lowercase();
    let has_long_context = request.history_message_count > 24
        || request.estimated_input_chars > 24_000
        || contains_any(
            &text,
            &["代码审查", "code review", "review this", "审查这个"],
        );
    if has_long_context {
        return ModelRouteTier::LongContext;
    }
    let strong_markers = [
        "任务卡",
        "架构",
        "实现",
        "修复",
        "bug",
        "重构",
        "验证",
        "测试",
        "agent",
        "mcp",
        "权限",
        "安全",
        "计划",
        "规划",
        "repo",
        "代码",
        "commit",
        "debug",
        "architecture",
        "implement",
        "refactor",
        "security",
    ];
    if contains_any(&text, &strong_markers) || request.needs_tools {
        return ModelRouteTier::Strong;
    }
    let economy_markers = [
        "翻译",
        "总结",
        "分类",
        "格式化",
        "改写",
        "润色",
        "提取",
        "解释",
        "translate",
        "summarize",
        "classify",
        "format",
        "rewrite",
    ];
    if request.estimated_input_chars <= 4_000
        && (contains_any(&text, &economy_markers) || text.chars().count() <= 80)
    {
        return ModelRouteTier::Economy;
    }
    ModelRouteTier::Balanced
}

fn route_candidate(
    connection: &ModelConnectionConfig,
    db: &LocalDb,
    request: &ModelRouteRequest,
    tier: ModelRouteTier,
    quality_events: &[ModelQualityEvent],
) -> ModelRouteCandidate {
    let caps = resolve_capabilities(db, &connection.provider_id, &connection.model)
        .unwrap_or_else(|_| ProviderCapabilities::new(&connection.provider_id, &connection.model));
    let mut reject_reasons = Vec::new();
    let ready = connection_ready(connection, &mut reject_reasons);
    if !connection.enabled {
        reject_reasons.push("disabled".to_string());
    }
    if ready && request.needs_vision && !caps.vision {
        reject_reasons.push("missing_vision".to_string());
    }
    if ready && request.needs_tools && !caps.tool_calls {
        reject_reasons.push("missing_tool_calls".to_string());
    }
    let required_context = if tier == ModelRouteTier::LongContext {
        request.estimated_input_tokens.saturating_add(8_000)
    } else {
        request.estimated_input_tokens
    };
    if ready && required_context > caps.max_context {
        reject_reasons.push(format!(
            "insufficient_context:{}<{}",
            caps.max_context, required_context
        ));
    }
    let quality_penalty =
        quality_penalty_for_model(quality_events, &connection.provider_id, &connection.model);
    let quality_penalty_points = (quality_penalty * 100.0).round() as i64;
    let quality_recommendation = if quality_penalty >= 5.0 {
        reject_reasons.push(format!("quality_penalty_high:{quality_penalty:.2}"));
        "avoid"
    } else if quality_penalty >= 2.0 {
        "deprioritize"
    } else {
        "eligible"
    }
    .to_string();
    ModelRouteCandidate {
        connection_id: connection.id.clone(),
        provider_id: connection.provider_id.clone(),
        model: connection.model.clone(),
        protocol: connection.protocol.clone(),
        enabled: connection.enabled,
        ready,
        cost_rank: model_cost_rank(&connection.provider_id, &connection.model),
        quality_rank: model_quality_rank(&connection.provider_id, &connection.model),
        max_context: caps.max_context,
        vision: caps.vision,
        tool_calls: caps.tool_calls,
        capability_source: caps.source.as_str().to_string(),
        quality_penalty_points,
        quality_recommendation,
        reject_reasons,
    }
}

fn connection_ready(connection: &ModelConnectionConfig, reject_reasons: &mut Vec<String>) -> bool {
    match connection.protocol.as_str() {
        "openai-compatible" => {
            if connection
                .base_url
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
            {
                reject_reasons.push("missing_base_url".to_string());
            }
            if !connection.is_local_runtime() && connection.api_key.trim().is_empty() {
                reject_reasons.push("missing_api_key".to_string());
            }
        }
        "anthropic" => {
            if connection.api_key.trim().is_empty() {
                reject_reasons.push("missing_api_key".to_string());
            }
        }
        other => reject_reasons.push(format!("unsupported_protocol:{other}")),
    }
    reject_reasons.is_empty()
}

// Internal decision constructor assembling the full route record from already
// computed parts; bundling these into a struct would only add indirection.
#[allow(clippy::too_many_arguments)]
fn decision_from_candidate(
    enabled: bool,
    tier: ModelRouteTier,
    request: &ModelRouteRequest,
    original: Option<&ModelConnectionConfig>,
    selected: &ModelRouteCandidate,
    candidates: Vec<ModelRouteCandidate>,
    reason_codes: Vec<String>,
    fallback_chain: Vec<String>,
) -> ModelRouteDecision {
    ModelRouteDecision {
        enabled,
        tier,
        selected_connection_id: Some(selected.connection_id.clone()),
        selected_provider_id: Some(selected.provider_id.clone()),
        selected_model: Some(selected.model.clone()),
        original_connection_id: original.map(|connection| connection.id.clone()),
        original_provider_id: original.map(|connection| connection.provider_id.clone()),
        original_model: original.map(|connection| connection.model.clone()),
        needs_vision: request.needs_vision,
        needs_tools: request.needs_tools,
        estimated_input_tokens: request.estimated_input_tokens,
        reason_codes,
        candidates,
        fallback_chain,
    }
}

fn fixed_decision(
    enabled: bool,
    tier: ModelRouteTier,
    request: &ModelRouteRequest,
    original: Option<&ModelConnectionConfig>,
    candidates: Vec<ModelRouteCandidate>,
    reason_codes: Vec<String>,
) -> ModelRouteDecision {
    ModelRouteDecision {
        enabled,
        tier,
        selected_connection_id: original.map(|connection| connection.id.clone()),
        selected_provider_id: original.map(|connection| connection.provider_id.clone()),
        selected_model: original.map(|connection| connection.model.clone()),
        original_connection_id: original.map(|connection| connection.id.clone()),
        original_provider_id: original.map(|connection| connection.provider_id.clone()),
        original_model: original.map(|connection| connection.model.clone()),
        needs_vision: request.needs_vision,
        needs_tools: request.needs_tools,
        estimated_input_tokens: request.estimated_input_tokens,
        reason_codes,
        candidates,
        // A fixed decision (routing disabled / forced-unavailable / no eligible
        // candidate) keeps the active connection only; there is no alternate to
        // fall back to in these paths.
        fallback_chain: original
            .map(|connection| connection.id.clone())
            .into_iter()
            .collect(),
    }
}

/// P3-3: ordered fallback chain = eligible candidates (no reject reasons) sorted
/// by route score descending. The first entry equals the selected connection.
fn eligible_fallback_chain(
    candidates: &[ModelRouteCandidate],
    tier: ModelRouteTier,
    active_connection_id: Option<&str>,
) -> Vec<String> {
    let mut eligible = candidates
        .iter()
        .filter(|candidate| candidate.reject_reasons.is_empty())
        .collect::<Vec<_>>();
    eligible.sort_by(|a, b| {
        route_score(b, tier, active_connection_id).cmp(&route_score(a, tier, active_connection_id))
    });
    eligible
        .into_iter()
        .map(|candidate| candidate.connection_id.clone())
        .collect()
}

fn route_score(
    candidate: &ModelRouteCandidate,
    tier: ModelRouteTier,
    active_connection_id: Option<&str>,
) -> (i64, String) {
    let active_bonus = if active_connection_id == Some(candidate.connection_id.as_str()) {
        8
    } else {
        0
    };
    let context_bucket = (candidate.max_context / 16_000) as i64;
    let quality_penalty = candidate.quality_penalty_points / 10;
    let score = match tier {
        ModelRouteTier::Economy => {
            1_000 - i64::from(candidate.cost_rank) * 100
                + i64::from(candidate.quality_rank) * 5
                + active_bonus
                - quality_penalty
        }
        ModelRouteTier::Balanced => {
            i64::from(candidate.quality_rank) * 35 - i64::from(candidate.cost_rank) * 25
                + context_bucket
                + active_bonus
                - quality_penalty * 2
        }
        ModelRouteTier::Strong => {
            i64::from(candidate.quality_rank) * 100 - i64::from(candidate.cost_rank) * 10
                + context_bucket
                + active_bonus
                - quality_penalty * 3
        }
        ModelRouteTier::LongContext => {
            i64::from(candidate.max_context / 1_000) * 10 + i64::from(candidate.quality_rank) * 20
                - i64::from(candidate.cost_rank) * 5
                + active_bonus
                - quality_penalty * 2
        }
    };
    (score, candidate.connection_id.clone())
}

fn model_cost_rank(provider_id: &str, model: &str) -> u8 {
    let provider = canonical_provider_id(provider_id).into_owned();
    let model = model.to_lowercase();
    if matches!(provider.as_str(), "ollama" | "lmstudio") || model.contains("local") {
        return 0;
    }
    if model.contains("mini")
        || model.contains("flash")
        || model.contains("haiku")
        || model.contains("turbo")
        || model.contains("lite")
        || model.contains("small")
    {
        return 1;
    }
    if model.contains("deepseek-chat")
        || model.contains("qwen")
        || model.contains("doubao")
        || model.contains("glm")
    {
        return 2;
    }
    if model.contains("sonnet") || model.contains("gpt-4o") || model.contains("plus") {
        return 4;
    }
    if model.contains("opus") || model.contains("reasoner") || model.starts_with("o1") {
        return 6;
    }
    3
}

fn model_quality_rank(provider_id: &str, model: &str) -> u8 {
    let provider = canonical_provider_id(provider_id).into_owned();
    let model = model.to_lowercase();
    if model.contains("opus") || model.starts_with("o1") || model.contains("reasoner") {
        return 6;
    }
    if model.contains("sonnet")
        || model.contains("gpt-4o")
        || model.contains("plus")
        || model.contains("pro")
    {
        return 5;
    }
    if model.contains("mini") || model.contains("haiku") || model.contains("flash") {
        return 3;
    }
    if matches!(provider.as_str(), "ollama" | "lmstudio") {
        return 2;
    }
    4
}

fn estimate_tokens_from_chars(chars: usize) -> u32 {
    ((chars / 4).max(1)).min(u32::MAX as usize) as u32
}

fn contains_any(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| text.contains(marker))
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, LLMConfig, ModelConnectionConfig, TmdbConfig, UiConfig};
    use uuid::Uuid;

    fn temp_db() -> LocalDb {
        LocalDb::open(std::env::temp_dir().join(format!("atlas_model_route_{}.db", Uuid::new_v4())))
            .unwrap()
    }

    fn connection(
        id: &str,
        provider: &str,
        model: &str,
        protocol: &str,
        api_key: &str,
    ) -> ModelConnectionConfig {
        ModelConnectionConfig {
            id: id.to_string(),
            name: id.to_string(),
            provider_id: provider.to_string(),
            route_id: id.to_string(),
            protocol: protocol.to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            base_url: Some(if protocol == "anthropic" {
                "https://api.anthropic.com/v1".to_string()
            } else {
                "https://api.example.com/v1".to_string()
            }),
            enabled: true,
            auth_header: None,
        }
    }

    fn config(default_id: &str, connections: Vec<ModelConnectionConfig>) -> Config {
        let default_provider = connections
            .iter()
            .find(|connection| connection.id == default_id)
            .map(|connection| connection.provider_id.clone())
            .unwrap_or_else(|| "openai".to_string());
        Config {
            llm: LLMConfig {
                default_provider,
                default_connection_id: Some(default_id.to_string()),
                connections,
                openai: None,
                anthropic: None,
            },
            ui: UiConfig::default(),
            tmdb: TmdbConfig::default(),
            execution: crate::tools::execution_isolation::ExecutionIsolationConfig::default(),
            outbound: crate::tools::outbound::OutboundPolicy::default(),
        }
    }

    #[test]
    fn simple_task_routes_to_lower_cost_candidate() {
        let db = temp_db();
        let config = config(
            "expensive",
            vec![
                connection(
                    "expensive",
                    "anthropic",
                    "claude-opus-4-8",
                    "anthropic",
                    "key",
                ),
                connection("cheap", "openai", "gpt-4o-mini", "openai-compatible", "key"),
            ],
        );
        let request = ModelRouteRequest::new("chat", "总结这段话", 0, 0, 0, 0, false);
        let decision = select_model_route(&config, &db, &request, &ModelRoutePolicy::default());
        assert_eq!(decision.tier, ModelRouteTier::Economy);
        assert_eq!(decision.selected_connection_id.as_deref(), Some("cheap"));
        assert!(decision
            .reason_codes
            .contains(&"prefer_low_cost".to_string()));
    }

    #[test]
    fn complex_planning_routes_to_stronger_candidate() {
        let db = temp_db();
        let config = config(
            "cheap",
            vec![
                connection("cheap", "openai", "gpt-4o-mini", "openai-compatible", "key"),
                connection("strong", "anthropic", "claude-opus-4-8", "anthropic", "key"),
            ],
        );
        let request = ModelRouteRequest::new(
            "chat",
            "设计 agent 架构并实现第23张任务卡",
            0,
            0,
            0,
            0,
            true,
        );
        let decision = select_model_route(&config, &db, &request, &ModelRoutePolicy::default());
        assert_eq!(decision.tier, ModelRouteTier::Strong);
        assert_eq!(decision.selected_connection_id.as_deref(), Some("strong"));
        assert!(decision
            .reason_codes
            .contains(&"requires_tool_calls".to_string()));
    }

    #[test]
    fn long_context_review_routes_to_largest_context_candidate() {
        let db = temp_db();
        let config = config(
            "cheap",
            vec![
                connection("cheap", "openai", "gpt-4o-mini", "openai-compatible", "key"),
                connection(
                    "long",
                    "minimax",
                    "MiniMax-Text-01",
                    "openai-compatible",
                    "key",
                ),
            ],
        );
        let request = ModelRouteRequest::new("code_review", "代码审查", 30, 80_000, 0, 0, false);
        let decision = select_model_route(&config, &db, &request, &ModelRoutePolicy::default());
        assert_eq!(decision.tier, ModelRouteTier::LongContext);
        assert_eq!(decision.selected_connection_id.as_deref(), Some("long"));
    }

    #[test]
    fn fallback_chain_is_ordered_with_selected_first() {
        let db = temp_db();
        let config = config(
            "cheap",
            vec![
                connection("cheap", "openai", "gpt-4o-mini", "openai-compatible", "key"),
                connection("strong", "anthropic", "claude-opus-4-8", "anthropic", "key"),
            ],
        );
        // Strong tier, no tools required so both connections stay eligible.
        let request = ModelRouteRequest::new("chat", "实现这个架构重构", 0, 0, 0, 0, false);
        let decision = select_model_route(&config, &db, &request, &ModelRoutePolicy::default());

        assert_eq!(decision.selected_connection_id.as_deref(), Some("strong"));
        // The chain starts with the selected connection, then the eligible alternate.
        assert_eq!(decision.fallback_chain, vec!["strong", "cheap"]);
        assert_eq!(
            decision.fallback_chain.first().map(String::as_str),
            decision.selected_connection_id.as_deref()
        );
    }

    #[test]
    fn forced_connection_has_single_link_chain() {
        let db = temp_db();
        let config = config(
            "cheap",
            vec![
                connection("cheap", "openai", "gpt-4o-mini", "openai-compatible", "key"),
                connection("strong", "anthropic", "claude-opus-4-8", "anthropic", "key"),
            ],
        );
        let policy = ModelRoutePolicy {
            enabled: true,
            force_connection_id: Some("cheap".to_string()),
            preferred_tier: None,
        };
        let request = ModelRouteRequest::new("chat", "实现这个架构重构", 0, 0, 0, 0, false);
        let decision = select_model_route(&config, &db, &request, &policy);

        // A user-forced connection is pinned: no silent fallback to another model.
        assert_eq!(decision.selected_connection_id.as_deref(), Some("cheap"));
        assert_eq!(decision.fallback_chain, vec!["cheap"]);
    }

    #[test]
    fn ineligible_connection_is_excluded_from_chain() {
        let db = temp_db();
        let config = config(
            "strong",
            vec![
                connection("strong", "anthropic", "claude-opus-4-8", "anthropic", "key"),
                // Missing api key on a non-local connection => not ready => ineligible.
                connection("broken", "openai", "gpt-4o-mini", "openai-compatible", ""),
            ],
        );
        let request = ModelRouteRequest::new("chat", "实现这个架构重构", 0, 0, 0, 0, false);
        let decision = select_model_route(&config, &db, &request, &ModelRoutePolicy::default());

        assert_eq!(decision.selected_connection_id.as_deref(), Some("strong"));
        assert_eq!(decision.fallback_chain, vec!["strong"]);
        assert!(!decision.fallback_chain.contains(&"broken".to_string()));
    }

    #[test]
    fn capability_filter_rejects_missing_vision_or_tools() {
        let db = temp_db();
        let config = config(
            "text",
            vec![
                connection(
                    "text",
                    "deepseek",
                    "deepseek-reasoner",
                    "openai-compatible",
                    "key",
                ),
                connection("vision", "openai", "gpt-4o", "openai-compatible", "key"),
            ],
        );
        let request = ModelRouteRequest::new("chat", "看图并解释", 0, 0, 1, 1, true);
        let decision = select_model_route(&config, &db, &request, &ModelRoutePolicy::default());
        assert_eq!(decision.selected_connection_id.as_deref(), Some("vision"));
        let text = decision
            .candidates
            .iter()
            .find(|candidate| candidate.connection_id == "text")
            .unwrap();
        assert!(text.reject_reasons.contains(&"missing_vision".to_string()));
        assert!(text
            .reject_reasons
            .contains(&"missing_tool_calls".to_string()));
    }

    #[test]
    fn provider_alias_capabilities_are_used_for_route_filtering() {
        let db = temp_db();
        let config = config(
            "bailian",
            vec![connection(
                "bailian",
                "aliyun-bailian",
                "qwen-vl-plus",
                "openai-compatible",
                "key",
            )],
        );
        let request = ModelRouteRequest::new("chat", "分析这张图", 0, 0, 1, 1, true);
        let decision = select_model_route(&config, &db, &request, &ModelRoutePolicy::default());

        assert_eq!(decision.selected_connection_id.as_deref(), Some("bailian"));
        let candidate = decision
            .candidates
            .iter()
            .find(|candidate| candidate.connection_id == "bailian")
            .unwrap();
        assert!(candidate.vision);
        assert!(candidate.tool_calls);
        assert!(candidate.reject_reasons.is_empty());
        assert_eq!(candidate.capability_source, "builtin");
    }

    #[test]
    fn user_force_and_disable_policy_are_respected() {
        let db = temp_db();
        let config = config(
            "cheap",
            vec![
                connection("cheap", "openai", "gpt-4o-mini", "openai-compatible", "key"),
                connection("strong", "anthropic", "claude-opus-4-8", "anthropic", "key"),
            ],
        );
        let request = ModelRouteRequest::new("chat", "总结一下", 0, 0, 0, 0, false);
        let forced = select_model_route(
            &config,
            &db,
            &request,
            &ModelRoutePolicy {
                force_connection_id: Some("strong".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(forced.selected_connection_id.as_deref(), Some("strong"));
        assert!(forced
            .reason_codes
            .contains(&"user_forced_connection".to_string()));

        let disabled = select_model_route(
            &config,
            &db,
            &request,
            &ModelRoutePolicy {
                enabled: false,
                ..Default::default()
            },
        );
        assert_eq!(disabled.selected_connection_id.as_deref(), Some("cheap"));
        assert!(disabled
            .reason_codes
            .contains(&"routing_disabled".to_string()));
    }

    #[test]
    fn quality_events_deprioritize_bad_model_and_route_decisions_are_audited() {
        let db = temp_db();
        crate::agent::provider_economics::record_model_quality_event(
            &db,
            crate::agent::provider_economics::RecordModelQualityEventRequest {
                provider: "anthropic".to_string(),
                model: "claude-opus-4-8".to_string(),
                run_id: Some("run-bad".to_string()),
                event_type: "false_completion".to_string(),
                severity: Some("critical".to_string()),
                weight: Some(6.0),
                reason: "claimed completion without verification".to_string(),
            },
        )
        .unwrap();
        let config = config(
            "strong",
            vec![
                connection("strong", "anthropic", "claude-opus-4-8", "anthropic", "key"),
                connection("balanced", "openai", "gpt-4o", "openai-compatible", "key"),
            ],
        );
        let request = ModelRouteRequest::new("chat", "实现 agent 架构", 0, 0, 0, 0, false);
        let decision = select_model_route(&config, &db, &request, &ModelRoutePolicy::default());
        assert_eq!(decision.selected_connection_id.as_deref(), Some("balanced"));
        let strong = decision
            .candidates
            .iter()
            .find(|candidate| candidate.connection_id == "strong")
            .unwrap();
        assert_eq!(strong.quality_recommendation, "avoid");
        assert!(strong
            .reject_reasons
            .iter()
            .any(|reason| reason.starts_with("quality_penalty_high")));
        let audit = record_model_route_decision(&db, &decision).unwrap();
        assert_eq!(audit.selected_connection_id.as_deref(), Some("balanced"));
        assert_eq!(list_model_route_decisions(&db).unwrap().len(), 1);
    }
}
