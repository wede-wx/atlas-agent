//! Provider capability matrix (M5 of millimeter plan).
//!
//! Replaces the §297 static vision blacklist with a structured per
//! (provider_id, model) capability record. Built-in defaults ship hard-coded
//! and get persisted into `provider_capabilities` on first use. Endpoint probes
//! set `source=probed`; dry-run completion probes set `source=verified`.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;

use crate::agent::{ProviderToolProtocolCaps, PseudoToolFormat, VisionInputFormat};
use crate::storage::{LocalDb, ProviderCapabilitiesRow, StorageError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySource {
    /// Static, hard-coded in the binary (this module's `builtin_capabilities`).
    Builtin,
    /// Set via a live endpoint probe (HTTP ping / model list).
    Probed,
    /// Set via a live dry-run completion that exercised capability inputs.
    Verified,
    /// User edited via settings UI / config.toml override.
    UserOverride,
}

impl CapabilitySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Probed => "probed",
            Self::Verified => "verified",
            Self::UserOverride => "user_override",
        }
    }

    // Infallible label→enum mapping with a `Builtin` fallback; not the fallible
    // `FromStr` contract (which returns `Result`), so the trait does not fit.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "verified" => Self::Verified,
            "probed" => Self::Probed,
            "user_override" => Self::UserOverride,
            _ => Self::Builtin,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub provider_id: String,
    pub model: String,
    pub vision: bool,
    pub tool_calls: bool,
    pub json_mode: bool,
    pub max_context: u32,
    pub source: CapabilitySource,
}

impl ProviderCapabilities {
    pub fn new(provider_id: &str, model: &str) -> Self {
        Self {
            provider_id: provider_id.to_string(),
            model: model.to_string(),
            vision: false,
            tool_calls: false,
            json_mode: false,
            max_context: 8_192,
            source: CapabilitySource::Builtin,
        }
    }

    pub fn missing_for_request(&self, needs_vision: bool, needs_tools: bool) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if needs_vision && !self.vision {
            missing.push("vision");
        }
        if needs_tools && !self.tool_calls {
            missing.push("tool_calls");
        }
        missing
    }

    /// Runtime Contract V2 capability view.
    ///
    /// This is intentionally derived from the existing persisted fields so old
    /// rows and settings continue to round-trip without a migration. Provider
    /// adapters can use this richer protocol surface while legacy UI/storage
    /// still reads `vision/tool_calls/json_mode/max_context`.
    pub fn tool_protocol_caps(&self) -> ProviderToolProtocolCaps {
        let provider = canonical_provider_id(&self.provider_id);
        let is_openai_compatible = matches!(
            provider.as_ref(),
            "openai"
                | "deepseek"
                | "xiaomi-mimo"
                | "qwen"
                | "doubao"
                | "hunyuan"
                | "ernie"
                | "zhipu"
                | "kimi"
                | "minimax"
                | "openrouter"
                | "siliconflow"
                | "lmstudio"
        );
        let is_anthropic = provider.as_ref() == "anthropic";

        ProviderToolProtocolCaps {
            structured_tool_calls: self.tool_calls,
            streaming_tool_calls: self.tool_calls && is_openai_compatible,
            pseudo_tool_call_format: self.tool_calls.then_some(PseudoToolFormat::Json),
            supports_tool_choice: self.tool_calls && is_openai_compatible,
            supports_parallel_tools: self.tool_calls && is_openai_compatible,
            supports_tool_result_role: self.tool_calls && is_openai_compatible,
            supports_json_response_format: self.json_mode,
            vision_input_format: if !self.vision {
                VisionInputFormat::None
            } else if is_anthropic {
                VisionInputFormat::AnthropicImageBlock
            } else {
                VisionInputFormat::OpenAiImageUrl
            },
        }
    }
}

/// Canonical capability key for providers that share one model family but are
/// discovered through different OpenAI-compatible endpoints.
pub fn canonical_provider_id(provider_id: &str) -> Cow<'_, str> {
    let provider = provider_id.trim().to_lowercase();
    match provider.as_str() {
        "aliyun-bailian" | "dashscope" | "bailian" | "alibaba-cloud" => Cow::Borrowed("qwen"),
        "volcengine-ark" | "volcengine" | "ark" | "bytedance-ark" => Cow::Borrowed("doubao"),
        "zai" | "bigmodel" | "bigmodel.cn" | "glm" => Cow::Borrowed("zhipu"),
        "moonshot-kimi" | "moonshot" | "moonshot-ai" => Cow::Borrowed("kimi"),
        "baidu-qianfan" | "qianfan" | "baidu" => Cow::Borrowed("ernie"),
        "tencent-hunyuan" | "tencent" => Cow::Borrowed("hunyuan"),
        "iflytek-spark" | "xfyun" | "sparkdesk" => Cow::Borrowed("spark"),
        "azure" | "azure-openai" | "microsoft-openai" => Cow::Borrowed("openai"),
        "google" | "google-gemini" | "google-ai" => Cow::Borrowed("gemini"),
        "openai" => Cow::Borrowed("openai"),
        "anthropic" => Cow::Borrowed("anthropic"),
        "deepseek" => Cow::Borrowed("deepseek"),
        "xiaomi-mimo" => Cow::Borrowed("xiaomi-mimo"),
        "qwen" => Cow::Borrowed("qwen"),
        "doubao" => Cow::Borrowed("doubao"),
        "hunyuan" => Cow::Borrowed("hunyuan"),
        "ernie" => Cow::Borrowed("ernie"),
        "spark" => Cow::Borrowed("spark"),
        "zhipu" => Cow::Borrowed("zhipu"),
        "kimi" => Cow::Borrowed("kimi"),
        "minimax" => Cow::Borrowed("minimax"),
        "gemini" => Cow::Borrowed("gemini"),
        "openrouter" => Cow::Borrowed("openrouter"),
        "siliconflow" => Cow::Borrowed("siliconflow"),
        "ollama" => Cow::Borrowed("ollama"),
        "lmstudio" => Cow::Borrowed("lmstudio"),
        _ => Cow::Owned(provider),
    }
}

/// Hard-coded baseline capabilities. Conservative when unsure: prefer false
/// over an aspirational true. Sources documented inline.
///
/// Order is matched by `lookup_builtin` — earlier wildcards (model="*") fall
/// through to later more-specific entries; check exact match first then
/// provider-default.
pub fn builtin_capabilities() -> Vec<ProviderCapabilities> {
    vec![
        // --- xiaomi mimo (no vision; tool_calls supported on v2.5-pro) ---
        ProviderCapabilities {
            provider_id: "xiaomi-mimo".into(),
            model: "*".into(),
            vision: false,
            tool_calls: true,
            json_mode: false,
            max_context: 65_536,
            source: CapabilitySource::Builtin,
        },
        // --- deepseek ---
        ProviderCapabilities {
            provider_id: "deepseek".into(),
            model: "deepseek-reasoner".into(),
            vision: false,
            tool_calls: false, // reasoner endpoint does not accept tools
            json_mode: false,
            max_context: 65_536,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "deepseek".into(),
            model: "*".into(),
            vision: false,
            tool_calls: true,
            json_mode: true,
            max_context: 65_536,
            source: CapabilitySource::Builtin,
        },
        // --- openai (细分 per family) ---
        // o1 family: reasoning only; no tools, no vision, no json_mode
        ProviderCapabilities {
            provider_id: "openai".into(),
            model: "o1-mini*".into(),
            vision: false,
            tool_calls: false,
            json_mode: false,
            max_context: 128_000,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "openai".into(),
            model: "o1-preview*".into(),
            vision: false,
            tool_calls: false,
            json_mode: false,
            max_context: 128_000,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "openai".into(),
            model: "o1*".into(),
            vision: true,
            tool_calls: true,
            json_mode: true,
            max_context: 200_000,
            source: CapabilitySource::Builtin,
        },
        // gpt-3.5-turbo: text only, has tools+json
        ProviderCapabilities {
            provider_id: "openai".into(),
            model: "gpt-3.5*".into(),
            vision: false,
            tool_calls: true,
            json_mode: true,
            max_context: 16_385,
            source: CapabilitySource::Builtin,
        },
        // gpt-4 (legacy, non-turbo): text only
        ProviderCapabilities {
            provider_id: "openai".into(),
            model: "gpt-4-0613".into(),
            vision: false,
            tool_calls: true,
            json_mode: false,
            max_context: 8_192,
            source: CapabilitySource::Builtin,
        },
        // gpt-4-turbo and gpt-4o-* both support vision+tools+json_mode
        ProviderCapabilities {
            provider_id: "openai".into(),
            model: "gpt-4-turbo*".into(),
            vision: true,
            tool_calls: true,
            json_mode: true,
            max_context: 128_000,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "openai".into(),
            model: "gpt-4o*".into(),
            vision: true,
            tool_calls: true,
            json_mode: true,
            max_context: 128_000,
            source: CapabilitySource::Builtin,
        },
        // provider-level fallback: prefer vision+tools (modern default)
        ProviderCapabilities {
            provider_id: "openai".into(),
            model: "*".into(),
            vision: true,
            tool_calls: true,
            json_mode: true,
            max_context: 128_000,
            source: CapabilitySource::Builtin,
        },
        // --- anthropic claude ---
        ProviderCapabilities {
            provider_id: "anthropic".into(),
            model: "*".into(),
            vision: true,
            tool_calls: true,
            json_mode: false, // claude doesn't have OAI-style json_mode flag
            max_context: 200_000,
            source: CapabilitySource::Builtin,
        },
        // --- gemini (Google OpenAI-compatible endpoint) ---
        // Modern Gemini 1.5/2.x families are multimodal; older catch-all stays
        // text-only until a probe or user override confirms vision.
        ProviderCapabilities {
            provider_id: "gemini".into(),
            model: "gemini-1.5*".into(),
            vision: true,
            tool_calls: true,
            json_mode: true,
            max_context: 1_000_000,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "gemini".into(),
            model: "gemini-2*".into(),
            vision: true,
            tool_calls: true,
            json_mode: true,
            max_context: 1_000_000,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "gemini".into(),
            model: "*".into(),
            vision: false,
            tool_calls: true,
            json_mode: true,
            max_context: 128_000,
            source: CapabilitySource::Builtin,
        },
        // --- qwen / dashscope ---
        ProviderCapabilities {
            provider_id: "qwen".into(),
            model: "qwen-vl-*".into(),
            vision: true,
            tool_calls: true,
            json_mode: false,
            max_context: 32_768,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "qwen".into(),
            model: "*".into(),
            vision: false,
            tool_calls: true,
            json_mode: false,
            max_context: 32_768,
            source: CapabilitySource::Builtin,
        },
        // --- doubao (volcengine 火山方舟) ---
        // doubao-1.5-vision-pro, doubao-1.5-pro 等；vision 仅 vision 后缀，tools 全系
        ProviderCapabilities {
            provider_id: "doubao".into(),
            model: "doubao-*-vision-*".into(),
            vision: true,
            tool_calls: true,
            json_mode: false,
            max_context: 32_768,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "doubao".into(),
            model: "*".into(),
            vision: false,
            tool_calls: true,
            json_mode: false,
            max_context: 32_768,
            source: CapabilitySource::Builtin,
        },
        // --- hunyuan (tencent 混元) ---
        // hunyuan-vision 支持图像；hunyuan-pro / hunyuan-standard 不支持
        ProviderCapabilities {
            provider_id: "hunyuan".into(),
            model: "hunyuan-vision*".into(),
            vision: true,
            tool_calls: true,
            json_mode: false,
            max_context: 32_768,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "hunyuan".into(),
            model: "*".into(),
            vision: false,
            tool_calls: true,
            json_mode: false,
            max_context: 32_768,
            source: CapabilitySource::Builtin,
        },
        // --- ernie (baidu 文心 / 千帆) ---
        // ernie-4.0-turbo / ernie-3.5；vl 后缀支持图像
        ProviderCapabilities {
            provider_id: "ernie".into(),
            model: "ernie-*-vl-*".into(),
            vision: true,
            tool_calls: true,
            json_mode: false,
            max_context: 8_192,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "ernie".into(),
            model: "*".into(),
            vision: false,
            tool_calls: true,
            json_mode: false,
            max_context: 8_192,
            source: CapabilitySource::Builtin,
        },
        // --- spark (iflytek 讯飞星火) ---
        // SparkDesk v3.5 / v4 支持 tools；image 仅特定型号
        ProviderCapabilities {
            provider_id: "spark".into(),
            model: "*-vision*".into(),
            vision: true,
            tool_calls: false,
            json_mode: false,
            max_context: 8_192,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "spark".into(),
            model: "*".into(),
            vision: false,
            tool_calls: true,
            json_mode: false,
            max_context: 8_192,
            source: CapabilitySource::Builtin,
        },
        // --- 智谱 (zhipu / glm) ---
        // glm-4v 支持图像；glm-4 / glm-4-plus 文本+tools
        ProviderCapabilities {
            provider_id: "zhipu".into(),
            model: "glm-4v*".into(),
            vision: true,
            tool_calls: true,
            json_mode: false,
            max_context: 8_192,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "zhipu".into(),
            model: "*".into(),
            vision: false,
            tool_calls: true,
            json_mode: false,
            max_context: 131_072,
            source: CapabilitySource::Builtin,
        },
        // --- kimi (moonshot 月之暗面) ---
        // 全系 OpenAI 协议；vision 由 moonshot-v1-*-vision-preview 支持
        ProviderCapabilities {
            provider_id: "kimi".into(),
            model: "moonshot-v1-*-vision-*".into(),
            vision: true,
            tool_calls: true,
            json_mode: true,
            max_context: 131_072,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "kimi".into(),
            model: "*".into(),
            vision: false,
            tool_calls: true,
            json_mode: true,
            max_context: 131_072,
            source: CapabilitySource::Builtin,
        },
        // --- minimax (abab 系列 / MiniMax-Text-01) ---
        ProviderCapabilities {
            provider_id: "minimax".into(),
            model: "abab*vision*".into(),
            vision: true,
            tool_calls: true,
            json_mode: false,
            max_context: 245_760,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "minimax".into(),
            model: "*".into(),
            vision: false,
            tool_calls: true,
            json_mode: false,
            max_context: 245_760,
            source: CapabilitySource::Builtin,
        },
        // --- OpenRouter aggregator ---
        // Model support varies by upstream model. Vision is only enabled for
        // known multimodal families; text fallback remains tool/json capable so
        // routing does not collapse to "all false" before probes refine it.
        ProviderCapabilities {
            provider_id: "openrouter".into(),
            model: "*gpt-4o*".into(),
            vision: true,
            tool_calls: true,
            json_mode: true,
            max_context: 128_000,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "openrouter".into(),
            model: "*claude-3*".into(),
            vision: true,
            tool_calls: true,
            json_mode: false,
            max_context: 200_000,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "openrouter".into(),
            model: "*gemini*".into(),
            vision: true,
            tool_calls: true,
            json_mode: true,
            max_context: 1_000_000,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "openrouter".into(),
            model: "*vl*".into(),
            vision: true,
            tool_calls: true,
            json_mode: true,
            max_context: 128_000,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "openrouter".into(),
            model: "*".into(),
            vision: false,
            tool_calls: true,
            json_mode: true,
            max_context: 128_000,
            source: CapabilitySource::Builtin,
        },
        // --- SiliconFlow aggregator ---
        ProviderCapabilities {
            provider_id: "siliconflow".into(),
            model: "*vl*".into(),
            vision: true,
            tool_calls: true,
            json_mode: false,
            max_context: 128_000,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "siliconflow".into(),
            model: "*vision*".into(),
            vision: true,
            tool_calls: true,
            json_mode: false,
            max_context: 128_000,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "siliconflow".into(),
            model: "*".into(),
            vision: false,
            tool_calls: true,
            json_mode: false,
            max_context: 128_000,
            source: CapabilitySource::Builtin,
        },
        // --- local backends: assume minimal until probed ---
        ProviderCapabilities {
            provider_id: "ollama".into(),
            model: "*".into(),
            vision: false,
            tool_calls: false,
            json_mode: false,
            max_context: 8_192,
            source: CapabilitySource::Builtin,
        },
        ProviderCapabilities {
            provider_id: "lmstudio".into(),
            model: "*".into(),
            vision: false,
            tool_calls: false,
            json_mode: false,
            max_context: 8_192,
            source: CapabilitySource::Builtin,
        },
    ]
}

impl From<&ProviderCapabilitiesRow> for ProviderCapabilities {
    fn from(row: &ProviderCapabilitiesRow) -> Self {
        Self {
            provider_id: row.provider_id.clone(),
            model: row.model.clone(),
            vision: row.vision,
            tool_calls: row.tool_calls,
            json_mode: row.json_mode,
            max_context: row.max_context,
            source: CapabilitySource::from_str(&row.source),
        }
    }
}

impl ProviderCapabilities {
    pub fn into_row(self) -> ProviderCapabilitiesRow {
        ProviderCapabilitiesRow {
            provider_id: self.provider_id,
            model: self.model,
            vision: self.vision,
            tool_calls: self.tool_calls,
            json_mode: self.json_mode,
            max_context: self.max_context,
            source: self.source.as_str().to_string(),
            updated_at: 0,
        }
    }
}

/// Resolve capabilities for (provider_id, model) with this priority:
/// 1. existing row in `provider_capabilities` (user_override > verified > probed > builtin)
/// 2. fall back to `lookup_builtin`; if found, persist to DB so subsequent
///    sessions and the settings UI can see it.
/// 3. final fallback: a minimal-rights default (everything false) — returned
///    but **not** persisted, so a later builtin upgrade can claim it.
pub fn resolve_capabilities(
    db: &LocalDb,
    provider_id: &str,
    model: &str,
) -> Result<ProviderCapabilities, StorageError> {
    let canonical_provider = canonical_provider_id(provider_id);
    if let Some(row) = db.get_provider_capabilities(provider_id, model)? {
        return Ok((&row).into());
    }
    if canonical_provider.as_ref() != provider_id {
        if let Some(row) = db.get_provider_capabilities(canonical_provider.as_ref(), model)? {
            return Ok(ProviderCapabilities {
                provider_id: provider_id.to_string(),
                model: model.to_string(),
                ..(&row).into()
            });
        }
    }
    if let Some(builtin) = lookup_builtin(provider_id, model) {
        // Persist a concrete (provider_id, model) row even if the lookup hit a
        // wildcard, so future calls bypass the wildcard scan.
        let mut row = builtin.clone().into_row();
        row.provider_id = provider_id.to_string();
        row.model = model.to_string();
        let _ = db.upsert_provider_capabilities(&row);
        return Ok(ProviderCapabilities {
            provider_id: provider_id.to_string(),
            model: model.to_string(),
            ..builtin
        });
    }
    Ok(ProviderCapabilities::new(provider_id, model))
}

/// Look up built-in caps for (provider_id, model). Priority:
/// 1. exact match
/// 2. glob match (pattern contains `*`, but not the bare `*`)
/// 3. provider-level `*` fallback
///
/// Returns `None` if provider_id is unknown entirely.
pub fn lookup_builtin(provider_id: &str, model: &str) -> Option<ProviderCapabilities> {
    let canonical_provider = canonical_provider_id(provider_id);
    let provider = canonical_provider.as_ref();
    let model_key = model.trim().to_lowercase();
    let all = builtin_capabilities();
    if let Some(cap) = all
        .iter()
        .find(|c| c.provider_id == provider && c.model.to_lowercase() == model_key)
    {
        return Some(cap.clone());
    }
    if let Some(cap) = all.iter().find(|c| {
        c.provider_id == provider
            && c.model != "*"
            && glob_match(&c.model.to_lowercase(), &model_key)
    }) {
        return Some(cap.clone());
    }
    all.into_iter()
        .find(|c| c.provider_id == provider && c.model == "*")
}

/// Minimal glob: `*` matches any (possibly empty) sequence of characters.
/// Implementation: split pattern on '*', then walk segments left-to-right.
/// Anchored at start unless pattern begins with '*'; anchored at end unless
/// pattern ends with '*'.
fn glob_match(pattern: &str, target: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == target;
    }
    let segments: Vec<&str> = pattern.split('*').collect();
    let mut cursor = 0usize;
    let starts_anchored = !pattern.starts_with('*');
    let ends_anchored = !pattern.ends_with('*');

    for (i, seg) in segments.iter().enumerate() {
        if seg.is_empty() {
            continue;
        }
        if i == 0 && starts_anchored {
            if !target[cursor..].starts_with(seg) {
                return false;
            }
            cursor += seg.len();
        } else if i == segments.len() - 1 && ends_anchored {
            return target[cursor..].ends_with(seg) && target.len() - cursor >= seg.len();
        } else {
            match target[cursor..].find(seg) {
                Some(pos) => cursor += pos + seg.len(),
                None => return false,
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_db() -> LocalDb {
        LocalDb::open(std::env::temp_dir().join(format!("aura_capabilities_{}.db", Uuid::new_v4())))
            .unwrap()
    }

    #[test]
    fn lookup_exact_match_wins_over_wildcard() {
        let caps = lookup_builtin("deepseek", "deepseek-reasoner").unwrap();
        assert!(!caps.tool_calls, "reasoner explicitly disables tool_calls");
        let chat = lookup_builtin("deepseek", "deepseek-chat").unwrap();
        assert!(chat.tool_calls, "non-reasoner deepseek allows tool_calls");
    }

    #[test]
    fn lookup_prefix_match_for_qwen_vl() {
        let caps = lookup_builtin("qwen", "qwen-vl-plus").unwrap();
        assert!(caps.vision);
        let text = lookup_builtin("qwen", "qwen-max").unwrap();
        assert!(!text.vision);
    }

    #[test]
    fn provider_aliases_resolve_to_canonical_builtin() {
        let bailian = lookup_builtin("aliyun-bailian", "qwen-vl-plus").unwrap();
        assert_eq!(bailian.provider_id, "qwen");
        assert!(bailian.vision);

        let ark = lookup_builtin("volcengine-ark", "doubao-1.5-pro").unwrap();
        assert_eq!(ark.provider_id, "doubao");
        assert!(ark.tool_calls);
        assert!(!ark.vision);

        let zai = lookup_builtin("zai", "glm-4v-plus").unwrap();
        assert_eq!(zai.provider_id, "zhipu");
        assert!(zai.vision);

        let azure = lookup_builtin("azure-openai", "gpt-4o-mini").unwrap();
        assert_eq!(azure.provider_id, "openai");
        assert!(azure.vision);
        assert!(azure.tool_calls);
    }

    #[test]
    fn resolve_uses_canonical_user_override_before_builtin() {
        let db = temp_db();
        db.upsert_provider_capabilities(
            &ProviderCapabilities {
                provider_id: "qwen".to_string(),
                model: "qwen-max".to_string(),
                vision: true,
                tool_calls: false,
                json_mode: true,
                max_context: 99_999,
                source: CapabilitySource::UserOverride,
            }
            .into_row(),
        )
        .unwrap();

        let caps = resolve_capabilities(&db, "aliyun-bailian", "qwen-max").unwrap();
        assert_eq!(caps.provider_id, "aliyun-bailian");
        assert_eq!(caps.model, "qwen-max");
        assert!(caps.vision);
        assert!(!caps.tool_calls);
        assert_eq!(caps.source, CapabilitySource::UserOverride);
        assert_eq!(caps.max_context, 99_999);
    }

    #[test]
    fn edge_provider_builtins_do_not_collapse_to_all_false() {
        let gemini_15 = lookup_builtin("gemini", "gemini-1.5-flash").unwrap();
        assert!(gemini_15.vision);
        assert!(gemini_15.tool_calls);
        assert!(gemini_15.json_mode);

        let gemini_legacy = lookup_builtin("google-gemini", "gemini-pro").unwrap();
        assert!(!gemini_legacy.vision);
        assert!(gemini_legacy.tool_calls);

        let openrouter_vision =
            lookup_builtin("openrouter", "anthropic/claude-3.5-sonnet").unwrap();
        assert!(openrouter_vision.vision);
        assert!(openrouter_vision.tool_calls);

        let openrouter_text = lookup_builtin("openrouter", "mistral-large").unwrap();
        assert!(!openrouter_text.vision);
        assert!(openrouter_text.tool_calls);

        let siliconflow_vl = lookup_builtin("siliconflow", "Qwen/Qwen2.5-VL-72B-Instruct").unwrap();
        assert!(siliconflow_vl.vision);
        assert!(siliconflow_vl.tool_calls);
    }

    #[test]
    fn lookup_unknown_provider_returns_none() {
        assert!(lookup_builtin("nonexistent", "foo").is_none());
    }

    #[test]
    fn capability_source_parses_verified() {
        assert_eq!(
            CapabilitySource::from_str("verified"),
            CapabilitySource::Verified
        );
        assert_eq!(CapabilitySource::Verified.as_str(), "verified");
    }

    #[test]
    fn missing_for_request_reports_each_gap() {
        let caps = lookup_builtin("xiaomi-mimo", "mimo-v2.5-pro").unwrap();
        let missing = caps.missing_for_request(true, false);
        assert_eq!(missing, vec!["vision"]);
        let missing_both = lookup_builtin("ollama", "llama3")
            .unwrap()
            .missing_for_request(true, true);
        assert_eq!(missing_both, vec!["vision", "tool_calls"]);
    }

    #[test]
    fn glob_match_handles_middle_wildcard() {
        assert!(glob_match("doubao-*-vision-*", "doubao-1.5-vision-pro"));
        assert!(!glob_match("doubao-*-vision-*", "doubao-1.5-pro"));
        assert!(glob_match("abab*vision*", "abab6.5-vision-preview"));
        assert!(glob_match("*-vision*", "spark-vision-3.5"));
        assert!(!glob_match("*-vision*", "spark-pro"));
    }

    #[test]
    fn doubao_vision_split_from_text() {
        let v = lookup_builtin("doubao", "doubao-1.5-vision-pro").unwrap();
        assert!(v.vision);
        let t = lookup_builtin("doubao", "doubao-1.5-pro").unwrap();
        assert!(!t.vision);
        assert!(t.tool_calls);
    }

    #[test]
    fn hunyuan_vision_prefix() {
        assert!(lookup_builtin("hunyuan", "hunyuan-vision").unwrap().vision);
        assert!(!lookup_builtin("hunyuan", "hunyuan-pro").unwrap().vision);
    }

    #[test]
    fn ernie_vl_match() {
        assert!(
            lookup_builtin("ernie", "ernie-4.0-vl-preview")
                .unwrap()
                .vision
        );
        assert!(!lookup_builtin("ernie", "ernie-4.0-turbo").unwrap().vision);
    }

    #[test]
    fn spark_vision_drops_tools() {
        let v = lookup_builtin("spark", "spark-vision-3.5").unwrap();
        assert!(v.vision);
        assert!(
            !v.tool_calls,
            "spark vision endpoint not confirmed to support tools"
        );
    }

    #[test]
    fn zhipu_glm4v_has_vision() {
        assert!(lookup_builtin("zhipu", "glm-4v-plus").unwrap().vision);
        assert!(!lookup_builtin("zhipu", "glm-4-plus").unwrap().vision);
    }

    #[test]
    fn kimi_vision_only_for_vision_models() {
        assert!(
            lookup_builtin("kimi", "moonshot-v1-128k-vision-preview")
                .unwrap()
                .vision
        );
        let t = lookup_builtin("kimi", "moonshot-v1-128k").unwrap();
        assert!(!t.vision);
        assert!(t.json_mode);
    }

    #[test]
    fn minimax_abab_vision_split() {
        assert!(
            lookup_builtin("minimax", "abab6.5-vision-preview")
                .unwrap()
                .vision
        );
        assert!(!lookup_builtin("minimax", "abab6.5-chat").unwrap().vision);
    }

    #[test]
    fn openai_o1_mini_drops_tools_and_vision() {
        let c = lookup_builtin("openai", "o1-mini-2024-09-12").unwrap();
        assert!(!c.tool_calls);
        assert!(!c.vision);
        assert!(!c.json_mode);
    }

    #[test]
    fn openai_gpt35_no_vision_but_tools() {
        let c = lookup_builtin("openai", "gpt-3.5-turbo-0125").unwrap();
        assert!(!c.vision);
        assert!(c.tool_calls);
        assert!(c.json_mode);
    }

    #[test]
    fn openai_gpt4o_full_capability() {
        let c = lookup_builtin("openai", "gpt-4o-mini").unwrap();
        assert!(c.vision);
        assert!(c.tool_calls);
        assert!(c.json_mode);
    }

    #[test]
    fn openai_gpt4_legacy_text_only() {
        let c = lookup_builtin("openai", "gpt-4-0613").unwrap();
        assert!(!c.vision);
        assert!(c.tool_calls);
    }

    #[test]
    fn anthropic_has_vision_and_tools() {
        let caps = lookup_builtin("anthropic", "claude-opus-4-8").unwrap();
        assert!(caps.vision);
        assert!(caps.tool_calls);
        assert!(caps.max_context >= 200_000);
    }
}
