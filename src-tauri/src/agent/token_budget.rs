use crate::agent::ModelTokenUsage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenBudgetScope {
    Run,
    Session,
    Day,
}

impl TokenBudgetScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Session => "session",
            Self::Day => "day",
        }
    }

    pub fn zh_label(self) -> &'static str {
        match self {
            Self::Run => "本次 run",
            Self::Session => "当前会话",
            Self::Day => "今日",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TokenBudgetHardLimitAction {
    #[default]
    PauseAndConfirm,
    Block,
}

impl TokenBudgetHardLimitAction {
    pub fn status(self) -> &'static str {
        match self {
            Self::PauseAndConfirm => "waiting_confirmation",
            Self::Block => "blocked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBudget {
    pub scope: TokenBudgetScope,
    #[serde(default)]
    pub soft_limit_tokens: Option<i64>,
    #[serde(default)]
    pub hard_limit_tokens: Option<i64>,
    #[serde(default)]
    pub spent_tokens: i64,
    #[serde(default)]
    pub on_hard_limit: TokenBudgetHardLimitAction,
}

impl TokenBudget {
    pub fn new(
        scope: TokenBudgetScope,
        soft_limit_tokens: Option<i64>,
        hard_limit_tokens: Option<i64>,
        spent_tokens: i64,
    ) -> Self {
        Self {
            scope,
            soft_limit_tokens,
            hard_limit_tokens,
            spent_tokens: spent_tokens.max(0),
            on_hard_limit: TokenBudgetHardLimitAction::PauseAndConfirm,
        }
    }

    fn normalized(mut self) -> Self {
        self.spent_tokens = self.spent_tokens.max(0);
        self.soft_limit_tokens = positive_limit(self.soft_limit_tokens);
        self.hard_limit_tokens = positive_limit(self.hard_limit_tokens);
        if let (Some(soft), Some(hard)) = (self.soft_limit_tokens, self.hard_limit_tokens) {
            if soft > hard {
                self.soft_limit_tokens = Some(hard);
            }
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBudgetCircuitBreaker {
    pub enabled: bool,
    pub high_total_tokens: i64,
    pub low_output_tokens: i64,
    /// Consecutive high-cost low-output calls required before tripping. A single
    /// large-context terse-answer turn (e.g. reading a big file then replying
    /// "yes") is normal, not spinning; only a *streak* of high-cost low-output
    /// calls indicates wasted spend. Defaults to 3.
    #[serde(default = "default_consecutive_low_yield_trigger")]
    pub consecutive_low_yield_trigger: i64,
    #[serde(default)]
    pub on_trigger: TokenBudgetHardLimitAction,
}

impl TokenBudgetCircuitBreaker {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            high_total_tokens: 0,
            low_output_tokens: 0,
            consecutive_low_yield_trigger: default_consecutive_low_yield_trigger(),
            on_trigger: TokenBudgetHardLimitAction::PauseAndConfirm,
        }
    }

    pub fn enabled_defaults() -> Self {
        Self {
            enabled: true,
            high_total_tokens: 40_000,
            low_output_tokens: 64,
            consecutive_low_yield_trigger: default_consecutive_low_yield_trigger(),
            on_trigger: TokenBudgetHardLimitAction::PauseAndConfirm,
        }
    }

    fn normalized(mut self) -> Self {
        if !self.enabled {
            return Self::disabled();
        }
        self.high_total_tokens = self.high_total_tokens.max(1);
        self.low_output_tokens = self.low_output_tokens.max(0);
        self.consecutive_low_yield_trigger = self.consecutive_low_yield_trigger.max(1);
        self
    }
}

impl Default for TokenBudgetCircuitBreaker {
    fn default() -> Self {
        Self::disabled()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBudgetSnapshot {
    #[serde(default)]
    pub budgets: Vec<TokenBudget>,
    #[serde(default)]
    pub circuit_breaker: TokenBudgetCircuitBreaker,
}

impl TokenBudgetSnapshot {
    pub fn disabled() -> Self {
        Self {
            budgets: Vec::new(),
            circuit_breaker: TokenBudgetCircuitBreaker::disabled(),
        }
    }

    pub fn active(budgets: Vec<TokenBudget>, circuit_breaker: TokenBudgetCircuitBreaker) -> Self {
        Self {
            budgets,
            circuit_breaker,
        }
        .normalized()
    }

    fn normalized(mut self) -> Self {
        self.budgets = self
            .budgets
            .into_iter()
            .map(TokenBudget::normalized)
            .filter(|budget| {
                budget.soft_limit_tokens.is_some() || budget.hard_limit_tokens.is_some()
            })
            .collect();
        self.budgets.sort_by_key(|budget| budget.scope);
        self.circuit_breaker = self.circuit_breaker.normalized();
        self
    }
}

impl Default for TokenBudgetSnapshot {
    fn default() -> Self {
        Self::disabled()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenBudgetWarning {
    pub scope: TokenBudgetScope,
    pub spent_tokens: i64,
    pub soft_limit_tokens: i64,
    pub hard_limit_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenBudgetStopReason {
    HardLimit,
    LowYieldCircuitBreaker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenBudgetStop {
    pub reason: TokenBudgetStopReason,
    pub scope: Option<TokenBudgetScope>,
    pub spent_tokens: i64,
    pub limit_tokens: Option<i64>,
    pub action: TokenBudgetHardLimitAction,
    pub detail: String,
}

impl TokenBudgetStop {
    pub fn event_status(&self) -> &'static str {
        self.action.status()
    }

    pub fn user_message(&self) -> String {
        match self.reason {
            TokenBudgetStopReason::HardLimit => {
                let scope = self
                    .scope
                    .map(TokenBudgetScope::zh_label)
                    .unwrap_or("当前范围");
                let limit = self
                    .limit_tokens
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "未知".to_string());
                format!(
                    "TokenBudget 已暂停本次运行：{scope} 已用 {} tokens，达到硬上限 {limit} tokens。我已经停止继续调用模型，避免继续消耗额度。请确认提高预算、切换更省的模型，或拆分任务后再继续。\n\n{}",
                    self.spent_tokens, self.detail
                )
            }
            TokenBudgetStopReason::LowYieldCircuitBreaker => format!(
                "TokenBudget 已触发高消耗低产出熔断：上一轮模型调用消耗 {} tokens，但输出过低。我已经停止继续调用模型，避免继续空转烧额度。请确认是否继续、换模型，或缩小上下文后再继续。\n\n{}",
                self.spent_tokens, self.detail
            ),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenBudgetPreflight {
    pub warning: Option<String>,
    pub stop: Option<TokenBudgetStop>,
}

#[derive(Debug, Clone)]
pub struct TokenBudgetEnforcer {
    snapshot: TokenBudgetSnapshot,
    run_spent_tokens: i64,
    warned_scopes: BTreeSet<TokenBudgetScope>,
    consecutive_low_yield: i64,
}

impl TokenBudgetEnforcer {
    pub fn new(snapshot: TokenBudgetSnapshot) -> Self {
        Self {
            snapshot: snapshot.normalized(),
            run_spent_tokens: 0,
            warned_scopes: BTreeSet::new(),
            consecutive_low_yield: 0,
        }
    }

    pub fn is_enabled(&self) -> bool {
        !self.snapshot.budgets.is_empty() || self.snapshot.circuit_breaker.enabled
    }

    pub fn preflight(&mut self) -> TokenBudgetPreflight {
        if !self.is_enabled() {
            return TokenBudgetPreflight::default();
        }
        if let Some(stop) = self.hard_limit_stop() {
            return TokenBudgetPreflight {
                warning: None,
                stop: Some(stop),
            };
        }
        let warnings = self.soft_limit_warnings();
        TokenBudgetPreflight {
            warning: (!warnings.is_empty()).then(|| soft_warning_message(&warnings)),
            stop: None,
        }
    }

    pub fn record_usage(&mut self, usage: &ModelTokenUsage) -> Option<TokenBudgetStop> {
        let total_tokens = normalized_total_tokens(usage);
        if total_tokens <= 0 {
            return None;
        }
        self.run_spent_tokens = self.run_spent_tokens.saturating_add(total_tokens);

        if let Some(stop) = self.circuit_breaker_stop(usage, total_tokens) {
            return Some(stop);
        }
        self.hard_limit_stop()
    }

    fn effective_spent(&self, budget: &TokenBudget) -> i64 {
        budget
            .spent_tokens
            .saturating_add(self.run_spent_tokens)
            .max(0)
    }

    fn hard_limit_stop(&self) -> Option<TokenBudgetStop> {
        self.snapshot
            .budgets
            .iter()
            .filter_map(|budget| {
                let hard_limit = budget.hard_limit_tokens?;
                let spent = self.effective_spent(budget);
                (spent >= hard_limit).then(|| TokenBudgetStop {
                    reason: TokenBudgetStopReason::HardLimit,
                    scope: Some(budget.scope),
                    spent_tokens: spent,
                    limit_tokens: Some(hard_limit),
                    action: budget.on_hard_limit,
                    detail: format!(
                        "scope={} spentTokens={} hardLimitTokens={}",
                        budget.scope.label(),
                        spent,
                        hard_limit
                    ),
                })
            })
            .min_by_key(|stop| stop.limit_tokens.unwrap_or(i64::MAX))
    }

    fn soft_limit_warnings(&mut self) -> Vec<TokenBudgetWarning> {
        let mut warnings = Vec::new();
        for budget in &self.snapshot.budgets {
            let Some(soft_limit) = budget.soft_limit_tokens else {
                continue;
            };
            if self.warned_scopes.contains(&budget.scope) {
                continue;
            }
            let spent = self.effective_spent(budget);
            if spent >= soft_limit {
                self.warned_scopes.insert(budget.scope);
                warnings.push(TokenBudgetWarning {
                    scope: budget.scope,
                    spent_tokens: spent,
                    soft_limit_tokens: soft_limit,
                    hard_limit_tokens: budget.hard_limit_tokens,
                });
            }
        }
        warnings
    }

    fn circuit_breaker_stop(
        &mut self,
        usage: &ModelTokenUsage,
        total_tokens: i64,
    ) -> Option<TokenBudgetStop> {
        let enabled = self.snapshot.circuit_breaker.enabled;
        if !enabled {
            self.consecutive_low_yield = 0;
            return None;
        }
        let high_total_tokens = self.snapshot.circuit_breaker.high_total_tokens;
        let low_output_tokens = self.snapshot.circuit_breaker.low_output_tokens;
        let trigger = self
            .snapshot
            .circuit_breaker
            .consecutive_low_yield_trigger
            .max(1);
        let on_trigger = self.snapshot.circuit_breaker.on_trigger;
        let output_tokens = usage.output_tokens.max(0);
        let is_low_yield = total_tokens >= high_total_tokens && output_tokens <= low_output_tokens;
        if !is_low_yield {
            // A productive turn breaks the streak; only *consecutive* high-cost
            // low-output calls count as spinning, so a lone large-context terse
            // answer never trips the breaker.
            self.consecutive_low_yield = 0;
            return None;
        }
        self.consecutive_low_yield = self.consecutive_low_yield.saturating_add(1);
        if self.consecutive_low_yield < trigger {
            return None;
        }
        Some(TokenBudgetStop {
            reason: TokenBudgetStopReason::LowYieldCircuitBreaker,
            scope: None,
            spent_tokens: total_tokens,
            limit_tokens: Some(high_total_tokens),
            action: on_trigger,
            detail: format!(
                "totalTokens={} outputTokens={} highTotalTokens={} lowOutputTokens={} consecutiveLowYield={} trigger={}",
                total_tokens,
                output_tokens,
                high_total_tokens,
                low_output_tokens,
                self.consecutive_low_yield,
                trigger
            ),
        })
    }
}

impl Default for TokenBudgetEnforcer {
    fn default() -> Self {
        Self::new(TokenBudgetSnapshot::disabled())
    }
}

fn positive_limit(limit: Option<i64>) -> Option<i64> {
    limit.filter(|value| *value > 0)
}

fn default_consecutive_low_yield_trigger() -> i64 {
    3
}

fn normalized_total_tokens(usage: &ModelTokenUsage) -> i64 {
    usage
        .total_tokens
        .max(usage.input_tokens.saturating_add(usage.output_tokens))
        .max(0)
}

/// M-9 (a): when a provider returns no `usage`, the turn must still be counted
/// against the budget instead of being silently skipped (which would let a
/// no-usage provider bypass the hard limit entirely). This is a deliberately
/// coarse character-based estimate — NOT a real tokenizer — using ~4 chars per
/// token, a reasonable rough bound for mixed Latin/CJK content. The recorded
/// usage event is tagged `model_estimated_usage` so the audit trail never
/// conflates an estimate with a provider-reported count.
pub fn estimate_token_usage(input_chars: i64, output_chars: i64) -> ModelTokenUsage {
    let input_tokens = estimate_tokens_from_chars(input_chars);
    let output_tokens = estimate_tokens_from_chars(output_chars);
    ModelTokenUsage {
        input_tokens,
        output_tokens,
        total_tokens: input_tokens.saturating_add(output_tokens),
    }
}

fn estimate_tokens_from_chars(chars: i64) -> i64 {
    const CHARS_PER_TOKEN: i64 = 4;
    let chars = chars.max(0);
    (chars + CHARS_PER_TOKEN - 1) / CHARS_PER_TOKEN
}

fn soft_warning_message(warnings: &[TokenBudgetWarning]) -> String {
    let lines = warnings
        .iter()
        .map(|warning| {
            let hard = warning
                .hard_limit_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "未设置".to_string());
            format!(
                "- {}: spentTokens={} softLimitTokens={} hardLimitTokens={}",
                warning.scope.zh_label(),
                warning.spent_tokens,
                warning.soft_limit_tokens,
                hard
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "TokenBudget 软限提示：以下范围已经达到软上限。继续前请优先压缩上下文、减少工具循环、避免重复读取；如无必要，不要扩大任务范围。\n{lines}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: i64, output: i64) -> ModelTokenUsage {
        ModelTokenUsage {
            input_tokens: input,
            output_tokens: output,
            total_tokens: input + output,
        }
    }

    #[test]
    fn estimate_token_usage_is_conservative_ceil_of_chars() {
        // ~4 chars/token, rounded up; zero/negative chars never go below zero.
        let est = estimate_token_usage(10, 5);
        assert_eq!(est.input_tokens, 3); // ceil(10/4)
        assert_eq!(est.output_tokens, 2); // ceil(5/4)
        assert_eq!(est.total_tokens, 5);
        assert_eq!(estimate_token_usage(0, 0).total_tokens, 0);
        assert_eq!(estimate_token_usage(-100, -1).total_tokens, 0);
    }

    #[test]
    fn estimated_usage_is_counted_against_the_budget() {
        // M-9 (a) red line: a no-usage turn must still be charged. Feeding an
        // estimated usage through record_usage must advance spend — proven by
        // crossing the hard limit and getting a stop, not a silent skip.
        let snapshot = TokenBudgetSnapshot::active(
            vec![TokenBudget::new(
                TokenBudgetScope::Run,
                Some(900),
                Some(1_000),
                0,
            )],
            TokenBudgetCircuitBreaker::disabled(),
        );
        let mut enforcer = TokenBudgetEnforcer::new(snapshot);
        let estimated = estimate_token_usage(4_000, 400); // ceil: 1000 + 100 = 1100
        assert_eq!(estimated.total_tokens, 1_100);
        let stop = enforcer
            .record_usage(&estimated)
            .expect("estimated usage must be charged and cross the hard limit");
        assert_eq!(stop.scope, Some(TokenBudgetScope::Run));
    }

    #[test]
    fn hard_limit_blocks_before_next_model_call() {
        let snapshot = TokenBudgetSnapshot::active(
            vec![TokenBudget::new(
                TokenBudgetScope::Run,
                Some(80),
                Some(100),
                100,
            )],
            TokenBudgetCircuitBreaker::disabled(),
        );
        let mut enforcer = TokenBudgetEnforcer::new(snapshot);
        let stop = enforcer.preflight().stop.expect("hard stop");
        assert_eq!(stop.reason, TokenBudgetStopReason::HardLimit);
        assert_eq!(stop.scope, Some(TokenBudgetScope::Run));
        assert_eq!(stop.event_status(), "waiting_confirmation");
    }

    #[test]
    fn soft_limit_warns_once_and_reaches_model_as_note() {
        let snapshot = TokenBudgetSnapshot::active(
            vec![TokenBudget::new(
                TokenBudgetScope::Session,
                Some(50),
                Some(100),
                60,
            )],
            TokenBudgetCircuitBreaker::disabled(),
        );
        let mut enforcer = TokenBudgetEnforcer::new(snapshot);
        let first = enforcer.preflight().warning.expect("soft warning");
        assert!(first.contains("TokenBudget 软限提示"));
        assert!(first.contains("当前会话"));
        assert!(enforcer.preflight().warning.is_none());
    }

    #[test]
    fn usage_crossing_hard_limit_stops_immediately() {
        let snapshot = TokenBudgetSnapshot::active(
            vec![TokenBudget::new(
                TokenBudgetScope::Day,
                Some(80),
                Some(100),
                70,
            )],
            TokenBudgetCircuitBreaker::disabled(),
        );
        let mut enforcer = TokenBudgetEnforcer::new(snapshot);
        assert!(enforcer.preflight().stop.is_none());
        let stop = enforcer.record_usage(&usage(20, 15)).expect("crosses hard");
        assert_eq!(stop.scope, Some(TokenBudgetScope::Day));
        assert_eq!(stop.spent_tokens, 105);
    }

    fn low_yield_breaker(trigger: i64) -> TokenBudgetCircuitBreaker {
        TokenBudgetCircuitBreaker {
            enabled: true,
            high_total_tokens: 100,
            low_output_tokens: 4,
            consecutive_low_yield_trigger: trigger,
            on_trigger: TokenBudgetHardLimitAction::PauseAndConfirm,
        }
    }

    #[test]
    fn single_low_yield_call_does_not_trip_breaker() {
        let snapshot = TokenBudgetSnapshot::active(Vec::new(), low_yield_breaker(3));
        let mut enforcer = TokenBudgetEnforcer::new(snapshot);
        // One large-context terse-answer turn is normal work, not spinning.
        assert!(enforcer.record_usage(&usage(120, 3)).is_none());
    }

    #[test]
    fn consecutive_low_yield_calls_trip_breaker_after_threshold() {
        let snapshot = TokenBudgetSnapshot::active(Vec::new(), low_yield_breaker(3));
        let mut enforcer = TokenBudgetEnforcer::new(snapshot);
        assert!(enforcer.record_usage(&usage(120, 3)).is_none());
        assert!(enforcer.record_usage(&usage(120, 3)).is_none());
        let stop = enforcer
            .record_usage(&usage(120, 3))
            .expect("breaker stop after 3 consecutive low-yield calls");
        assert_eq!(stop.reason, TokenBudgetStopReason::LowYieldCircuitBreaker);
        assert_eq!(stop.event_status(), "waiting_confirmation");
    }

    #[test]
    fn productive_turn_resets_low_yield_streak() {
        let snapshot = TokenBudgetSnapshot::active(Vec::new(), low_yield_breaker(3));
        let mut enforcer = TokenBudgetEnforcer::new(snapshot);
        assert!(enforcer.record_usage(&usage(120, 3)).is_none());
        assert!(enforcer.record_usage(&usage(120, 3)).is_none());
        // A productive (high-output) turn breaks the streak...
        assert!(enforcer.record_usage(&usage(120, 500)).is_none());
        // ...so two further low-yield turns are still below the threshold.
        assert!(enforcer.record_usage(&usage(120, 3)).is_none());
        assert!(enforcer.record_usage(&usage(120, 3)).is_none());
    }
}
