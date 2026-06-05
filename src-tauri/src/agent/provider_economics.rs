use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::storage::{LocalDb, ModelUsageRecord};

const MODEL_QUALITY_EVENTS_KEY: &str = "model_quality_events:v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPriceRule {
    pub provider: String,
    pub model_pattern: String,
    pub input_per_million_usd: f64,
    pub output_per_million_usd: f64,
    pub max_context: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelCostEstimate {
    pub provider: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub price_known: bool,
    pub estimated_cost_usd: Option<f64>,
    pub rule: Option<ProviderPriceRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTokenEstimate {
    pub provider: String,
    pub model: String,
    pub tokenizer: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelTextCostEstimate {
    pub tokens: ProviderTokenEstimate,
    pub cost: ModelCostEstimate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelQualityEvent {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub run_id: Option<String>,
    pub event_type: String,
    pub severity: String,
    pub weight: f64,
    pub reason: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordModelQualityEventRequest {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub run_id: Option<String>,
    pub event_type: String,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub weight: Option<f64>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RouteEconomicsDecision {
    pub provider: String,
    pub model: String,
    pub estimated_cost_usd: Option<f64>,
    pub quality_penalty: f64,
    pub recommendation: String,
    pub reasons: Vec<String>,
}

pub fn builtin_price_rules() -> Vec<ProviderPriceRule> {
    vec![
        rule("openai", "gpt-4o-mini", 0.15, 0.60, Some(128_000)),
        rule("openai", "gpt-4o", 2.50, 10.0, Some(128_000)),
        rule("anthropic", "claude-3-5-sonnet", 3.0, 15.0, Some(200_000)),
        rule("anthropic", "claude-3-5-haiku", 0.80, 4.0, Some(200_000)),
        rule("deepseek", "deepseek-chat", 0.27, 1.10, Some(64_000)),
        rule("deepseek", "deepseek-reasoner", 0.55, 2.19, Some(64_000)),
        rule("qwen", "qwen", 0.30, 1.20, None),
        rule("doubao", "doubao", 0.30, 1.20, None),
        rule("siliconflow", "*", 0.50, 1.50, None),
        rule("openrouter", "*", 1.00, 3.00, None),
    ]
}

pub fn estimate_model_usage_cost(usage: &ModelUsageRecord) -> ModelCostEstimate {
    estimate_cost_from_parts(
        &usage.provider,
        &usage.model,
        usage.input_tokens,
        usage.output_tokens,
        usage.total_tokens,
    )
}

pub fn estimate_cost_from_text_parts(
    provider: &str,
    model: &str,
    input_text: &str,
    output_text: &str,
) -> (ProviderTokenEstimate, ModelCostEstimate) {
    let tokens = estimate_provider_tokens(provider, model, input_text, output_text);
    let cost = estimate_cost_from_parts(
        provider,
        model,
        tokens.input_tokens,
        tokens.output_tokens,
        tokens.total_tokens,
    );
    (tokens, cost)
}

pub fn estimate_model_text_cost(
    provider: &str,
    model: &str,
    input_text: &str,
    output_text: &str,
) -> ModelTextCostEstimate {
    let (tokens, cost) = estimate_cost_from_text_parts(provider, model, input_text, output_text);
    ModelTextCostEstimate { tokens, cost }
}

pub fn estimate_provider_tokens(
    provider: &str,
    model: &str,
    input_text: &str,
    output_text: &str,
) -> ProviderTokenEstimate {
    let tokenizer = tokenizer_family(provider, model);
    let input_tokens = estimate_text_tokens(&tokenizer, input_text);
    let output_tokens = estimate_text_tokens(&tokenizer, output_text);
    let mut warnings = Vec::new();
    if tokenizer.ends_with("_approx") {
        warnings.push(
            "provider did not return token usage; estimate uses provider-aware local tokenizer approximation"
                .to_string(),
        );
    }
    ProviderTokenEstimate {
        provider: provider.to_string(),
        model: model.to_string(),
        tokenizer,
        input_tokens,
        output_tokens,
        total_tokens: input_tokens.saturating_add(output_tokens),
        warnings,
    }
}

pub fn estimate_cost_from_parts(
    provider: &str,
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
) -> ModelCostEstimate {
    let rule = matching_price_rule(provider, model);
    let estimated_cost_usd = rule.as_ref().map(|rule| {
        (input_tokens.max(0) as f64 / 1_000_000.0) * rule.input_per_million_usd
            + (output_tokens.max(0) as f64 / 1_000_000.0) * rule.output_per_million_usd
    });
    ModelCostEstimate {
        provider: provider.to_string(),
        model: model.to_string(),
        input_tokens,
        output_tokens,
        total_tokens,
        price_known: rule.is_some(),
        estimated_cost_usd,
        rule,
    }
}

pub fn record_model_quality_event(
    db: &LocalDb,
    request: RecordModelQualityEventRequest,
) -> Result<ModelQualityEvent, String> {
    if request.provider.trim().is_empty() || request.model.trim().is_empty() {
        return Err("provider and model are required".to_string());
    }
    if request.reason.trim().is_empty() {
        return Err("quality event reason is required".to_string());
    }
    let event = ModelQualityEvent {
        id: format!("mqe_{}", Uuid::new_v4()),
        provider: request.provider.trim().to_string(),
        model: request.model.trim().to_string(),
        run_id: request.run_id,
        event_type: normalize_quality_event_type(&request.event_type),
        severity: normalize_quality_severity(request.severity.as_deref().unwrap_or("medium")),
        weight: request.weight.unwrap_or(1.0).clamp(0.0, 10.0),
        reason: request.reason.chars().take(500).collect(),
        created_at: chrono::Utc::now().timestamp_millis(),
    };
    let mut events = list_model_quality_events(db)?;
    events.push(event.clone());
    events.sort_by_key(|event| event.created_at);
    if events.len() > 200 {
        events = events.split_off(events.len() - 200);
    }
    db.set_app_state(MODEL_QUALITY_EVENTS_KEY, json!(events))
        .map_err(|error| error.to_string())?;
    Ok(event)
}

pub fn list_model_quality_events(db: &LocalDb) -> Result<Vec<ModelQualityEvent>, String> {
    let value = db
        .get_app_state(MODEL_QUALITY_EVENTS_KEY)
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| json!([]));
    serde_json::from_value(value).map_err(|error| error.to_string())
}

pub fn route_economics_decision(
    estimate: ModelCostEstimate,
    events: &[ModelQualityEvent],
) -> RouteEconomicsDecision {
    let quality_penalty = model_quality_penalty(events, &estimate.provider, &estimate.model);
    let mut reasons = Vec::new();
    if estimate.price_known {
        reasons.push("matched provider/model price rule".to_string());
    } else {
        reasons.push("no price rule matched; do not optimize on unknown cost".to_string());
    }
    if quality_penalty > 0.0 {
        reasons.push(format!("historical quality penalty {:.2}", quality_penalty));
    }
    let recommendation = if quality_penalty >= 3.0 {
        "upgrade_or_avoid".to_string()
    } else if estimate.estimated_cost_usd.is_some_and(|cost| cost > 1.0) {
        "use_when_quality_required".to_string()
    } else {
        "eligible".to_string()
    };
    RouteEconomicsDecision {
        provider: estimate.provider.clone(),
        model: estimate.model.clone(),
        estimated_cost_usd: estimate.estimated_cost_usd,
        quality_penalty,
        recommendation,
        reasons,
    }
}

pub fn model_quality_penalty(events: &[ModelQualityEvent], provider: &str, model: &str) -> f64 {
    events
        .iter()
        .filter(|event| {
            event.provider.eq_ignore_ascii_case(provider) && event.model.eq_ignore_ascii_case(model)
        })
        .map(|event| match event.severity.as_str() {
            "critical" => event.weight * 2.0,
            "high" => event.weight * 1.5,
            "low" => event.weight * 0.5,
            _ => event.weight,
        })
        .sum::<f64>()
}

pub fn quality_penalty_for_model(events: &[ModelQualityEvent], provider: &str, model: &str) -> f64 {
    events
        .iter()
        .filter(|event| {
            event.provider.eq_ignore_ascii_case(provider) && event.model.eq_ignore_ascii_case(model)
        })
        .map(|event| event.weight)
        .sum::<f64>()
}

fn matching_price_rule(provider: &str, model: &str) -> Option<ProviderPriceRule> {
    let provider = provider.to_ascii_lowercase();
    let model = model.to_ascii_lowercase();
    builtin_price_rules().into_iter().find(|rule| {
        provider == rule.provider
            && (rule.model_pattern == "*"
                || model == rule.model_pattern
                || model.contains(&rule.model_pattern))
    })
}

fn rule(
    provider: &str,
    model_pattern: &str,
    input: f64,
    output: f64,
    max_context: Option<i64>,
) -> ProviderPriceRule {
    ProviderPriceRule {
        provider: provider.to_string(),
        model_pattern: model_pattern.to_string(),
        input_per_million_usd: input,
        output_per_million_usd: output,
        max_context,
    }
}

fn tokenizer_family(provider: &str, model: &str) -> String {
    let provider = provider.to_ascii_lowercase();
    let model = model.to_ascii_lowercase();
    if provider == "openai" || model.starts_with("gpt-") || model.starts_with("o1") {
        "openai_cl100k_approx".to_string()
    } else if provider == "anthropic" || model.contains("claude") {
        "anthropic_sentencepiece_approx".to_string()
    } else if provider == "deepseek" || provider == "qwen" || model.contains("qwen") {
        "cjk_bpe_approx".to_string()
    } else {
        "generic_bpe_approx".to_string()
    }
}

fn estimate_text_tokens(tokenizer: &str, text: &str) -> i64 {
    if text.trim().is_empty() {
        return 0;
    }
    let mut ascii_word_chars = 0usize;
    let mut ascii_words = 0usize;
    let mut cjk_chars = 0usize;
    let mut punctuation = 0usize;
    let mut in_ascii_word = false;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            ascii_word_chars += 1;
            if !in_ascii_word {
                ascii_words += 1;
                in_ascii_word = true;
            }
        } else {
            in_ascii_word = false;
            if is_cjk(ch) {
                cjk_chars += 1;
            } else if !ch.is_whitespace() {
                punctuation += 1;
            }
        }
    }
    let ascii_tokens = (ascii_word_chars as f64
        / match tokenizer {
            "anthropic_sentencepiece_approx" => 3.8,
            "cjk_bpe_approx" => 3.2,
            _ => 4.0,
        })
    .ceil() as i64;
    let word_boundary_tokens = (ascii_words / 12) as i64;
    let cjk_tokens = match tokenizer {
        "cjk_bpe_approx" => cjk_chars as i64,
        _ => ((cjk_chars as f64) * 1.15).ceil() as i64,
    };
    let punctuation_tokens = ((punctuation as f64) / 2.0).ceil() as i64;
    ascii_tokens
        .saturating_add(word_boundary_tokens)
        .saturating_add(cjk_tokens)
        .saturating_add(punctuation_tokens)
        .max(1)
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
    )
}

fn normalize_quality_event_type(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "verify_failed" | "false_completion" | "tool_failure" | "timeout" | "success" => {
            value.trim().to_ascii_lowercase()
        }
        _ => "other".to_string(),
    }
}

fn normalize_quality_severity(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "low" | "medium" | "high" | "critical" => value.trim().to_ascii_lowercase(),
        _ => "medium".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_db() -> LocalDb {
        LocalDb::open(
            std::env::temp_dir().join(format!("atlas_provider_economics_{}.db", Uuid::new_v4())),
        )
        .unwrap()
    }

    #[test]
    fn estimates_cost_from_matching_price_rule() {
        let estimate =
            estimate_cost_from_parts("openai", "gpt-4o-mini", 1_000_000, 500_000, 1_500_000);
        assert!(estimate.price_known);
        assert!((estimate.estimated_cost_usd.unwrap() - 0.45).abs() < 0.000001);
    }

    #[test]
    fn provider_aware_text_token_estimate_feeds_price_table() {
        let (tokens, cost) = estimate_cost_from_text_parts(
            "openai",
            "gpt-4o-mini",
            "Implement 架构 verification with command output.",
            "Done with evidence.",
        );
        assert!(tokens.input_tokens > tokens.output_tokens);
        assert_eq!(tokens.tokenizer, "openai_cl100k_approx");
        assert!(tokens
            .warnings
            .iter()
            .any(|warning| warning.contains("provider-aware")));
        assert!(cost.price_known);
        assert_eq!(cost.total_tokens, tokens.total_tokens);
    }

    #[test]
    fn quality_events_affect_route_recommendation() {
        let db = temp_db();
        let event = record_model_quality_event(
            &db,
            RecordModelQualityEventRequest {
                provider: "openai".to_string(),
                model: "gpt-4o-mini".to_string(),
                run_id: Some("run-a".to_string()),
                event_type: "false_completion".to_string(),
                severity: Some("high".to_string()),
                weight: Some(3.5),
                reason: "claimed completion without evidence".to_string(),
            },
        )
        .unwrap();
        assert_eq!(list_model_quality_events(&db).unwrap().len(), 1);
        let decision = route_economics_decision(
            estimate_cost_from_parts("openai", "gpt-4o-mini", 100, 100, 200),
            &[event],
        );
        assert!(decision.quality_penalty > 3.5);
        assert_eq!(decision.recommendation, "upgrade_or_avoid");
    }
}
