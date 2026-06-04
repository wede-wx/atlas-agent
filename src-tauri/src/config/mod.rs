use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub llm: LLMConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub tmdb: TmdbConfig,
    /// P2-12: lightweight desktop execution isolation. Commands stay inside a
    /// configured workspace boundary and child processes receive only an
    /// allowlisted environment, so provider credentials stay in the Rust-side
    /// client/proxy path instead of leaking into shell commands.
    #[serde(default)]
    pub execution: crate::tools::execution_isolation::ExecutionIsolationConfig,
    /// P0-3: per-channel outbound network policy (provider / MCP / web /
    /// telemetry / shell). `#[serde(default)]` so older config files load with
    /// the permissive-safe default.
    #[serde(default)]
    pub outbound: crate::tools::outbound::OutboundPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TmdbConfig {
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_sound_enabled")]
    pub sound_enabled: bool,
}

fn default_theme() -> String {
    "dark".into()
}

fn default_sound_enabled() -> bool {
    true
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            sound_enabled: default_sound_enabled(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMConfig {
    #[serde(default = "default_provider")]
    pub default_provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_connection_id: Option<String>,
    #[serde(default)]
    pub connections: Vec<ModelConnectionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai: Option<ProviderConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<ProviderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub api_key: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConnectionConfig {
    pub id: String,
    pub name: String,
    pub provider_id: String,
    pub route_id: String,
    pub protocol: String,
    pub api_key: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default = "default_connection_enabled")]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,
}

fn default_provider() -> String {
    "openai".to_string()
}

fn default_connection_enabled() -> bool {
    true
}

#[derive(Debug, thiserror::Error, Serialize)]
pub enum ConfigError {
    #[error("Config file not found: {0}")]
    NotFound(String),
    #[error("Failed to read config: {0}")]
    Io(String),
    #[error("Failed to parse config: {0}")]
    Parse(String),
    #[error("Missing API key for provider: {0}")]
    MissingApiKey(String),
    #[error("Unsupported provider: {0}")]
    UnsupportedProvider(String),
    #[error("Outbound network boundary denied request: {0}")]
    OutboundDenied(String),
}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e.to_string())
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(e: toml::de::Error) -> Self {
        ConfigError::Parse(e.to_string())
    }
}

impl From<toml::ser::Error> for ConfigError {
    fn from(e: toml::ser::Error) -> Self {
        ConfigError::Parse(e.to_string())
    }
}

impl Config {
    pub fn load() -> Result<Self, ConfigError> {
        let config_path = Self::config_path()?;

        let mut config = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            toml::from_str(&content)?
        } else {
            Self::default()
        };

        config.migrate_legacy_providers();
        Self::apply_env_overrides(&mut config);
        Ok(config)
    }

    fn config_path() -> Result<PathBuf, ConfigError> {
        if let Ok(path) = std::env::var("AURA_HOME") {
            let path = path.trim();
            if !path.is_empty() {
                return Ok(PathBuf::from(path).join("config.toml"));
            }
        }
        let home = dirs::home_dir()
            .ok_or_else(|| ConfigError::NotFound("Home directory not found".to_string()))?;
        Ok(home.join(".aura").join("config.toml"))
    }

    fn apply_env_overrides(config: &mut Self) {
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            if let Some(openai) = &mut config.llm.openai {
                openai.api_key = key.clone();
            }
            for connection in &mut config.llm.connections {
                if connection.provider_id == "openai" {
                    connection.api_key = key.clone();
                }
            }
            std::env::remove_var("OPENAI_API_KEY");
        }
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            if let Some(anthropic) = &mut config.llm.anthropic {
                anthropic.api_key = key.clone();
            }
            for connection in &mut config.llm.connections {
                if connection.provider_id == "anthropic" {
                    connection.api_key = key.clone();
                }
            }
            std::env::remove_var("ANTHROPIC_API_KEY");
        }
    }

    pub fn validate_for_chat(&self) -> Result<(), ConfigError> {
        let Some(connection) = self.llm.active_connection() else {
            return Err(ConfigError::MissingApiKey(
                self.llm.default_provider.clone(),
            ));
        };
        match connection.protocol.as_str() {
            "openai-compatible" => {
                if connection
                    .base_url
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
                    return Err(ConfigError::UnsupportedProvider(format!(
                        "{} missing base_url",
                        connection.provider_id
                    )));
                }
                if !connection.is_local_runtime() && connection.api_key.trim().is_empty() {
                    return Err(ConfigError::MissingApiKey(connection.provider_id.clone()));
                }
            }
            "anthropic" => {
                if connection.api_key.trim().is_empty() {
                    return Err(ConfigError::MissingApiKey(connection.provider_id.clone()));
                }
            }
            other => return Err(ConfigError::UnsupportedProvider(other.to_string())),
        }
        Ok(())
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        let config_path = Self::config_path()?;

        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut config = self.clone();
        config.migrate_legacy_providers();
        config.llm.sync_legacy_slots_from_connections();
        let content = toml::to_string_pretty(&config)?;
        std::fs::write(&config_path, content)?;

        Ok(())
    }

    pub fn redacted_for_client(&self) -> Self {
        let mut config = self.clone();
        config.tmdb.api_key.clear();
        config
    }

    fn migrate_legacy_providers(&mut self) {
        let mut legacy_deepseek: Option<ModelConnectionConfig> = None;
        if self.llm.default_provider == "deepseek" {
            self.llm.default_provider = "openai".to_string();
            if let Some(openai) = &mut self.llm.openai {
                if openai.base_url.is_none() {
                    openai.base_url = Some("https://api.deepseek.com/v1".to_string());
                }
                if openai.model.trim().is_empty() {
                    openai.model = "deepseek-chat".to_string();
                }
                legacy_deepseek = Some(ModelConnectionConfig {
                    id: "deepseek:deepseek-openai".to_string(),
                    name: "DeepSeek".to_string(),
                    provider_id: "deepseek".to_string(),
                    route_id: "deepseek-openai".to_string(),
                    protocol: "openai-compatible".to_string(),
                    api_key: openai.api_key.clone(),
                    model: openai.model.clone(),
                    base_url: Some(normalize_base_url(
                        openai
                            .base_url
                            .as_deref()
                            .unwrap_or("https://api.deepseek.com/v1"),
                    )),
                    enabled: true,
                    auth_header: None,
                });
            }
        }
        if let Some(connection) = legacy_deepseek {
            self.llm.upsert_connection(connection);
            self.llm.default_connection_id = Some("deepseek:deepseek-openai".to_string());
        }
        self.llm.ensure_connections_from_legacy();
        self.llm.sync_legacy_slots_from_connections();
    }
}

impl Default for Config {
    fn default() -> Self {
        let openai = ModelConnectionConfig {
            id: "openai:openai-default".to_string(),
            name: "OpenAI".to_string(),
            provider_id: "openai".to_string(),
            route_id: "openai-default".to_string(),
            protocol: "openai-compatible".to_string(),
            api_key: String::new(),
            model: "gpt-4o-mini".to_string(),
            base_url: Some("https://api.openai.com/v1".to_string()),
            enabled: true,
            auth_header: None,
        };
        let anthropic = ModelConnectionConfig {
            id: "anthropic:anthropic-default".to_string(),
            name: "Anthropic".to_string(),
            provider_id: "anthropic".to_string(),
            route_id: "anthropic-default".to_string(),
            protocol: "anthropic".to_string(),
            api_key: String::new(),
            model: "claude-opus-4-8".to_string(),
            base_url: Some("https://api.anthropic.com/v1".to_string()),
            enabled: true,
            auth_header: None,
        };
        let mut config = Self {
            llm: LLMConfig {
                default_provider: "openai".to_string(),
                default_connection_id: Some(openai.id.clone()),
                connections: vec![openai, anthropic],
                openai: None,
                anthropic: None,
            },
            ui: UiConfig::default(),
            tmdb: TmdbConfig::default(),
            execution: crate::tools::execution_isolation::ExecutionIsolationConfig::default(),
            outbound: crate::tools::outbound::OutboundPolicy::default(),
        };
        config.llm.sync_legacy_slots_from_connections();
        config
    }
}

impl LLMConfig {
    pub fn active_connection(&self) -> Option<&ModelConnectionConfig> {
        self.default_connection_id
            .as_deref()
            .and_then(|id| {
                self.connections
                    .iter()
                    .find(|connection| connection.id == id)
            })
            .or_else(|| {
                self.connections
                    .iter()
                    .find(|connection| connection.provider_id == self.default_provider)
            })
            .or_else(|| {
                self.connections
                    .iter()
                    .find(|connection| connection.enabled)
            })
            .or_else(|| self.connections.first())
    }

    pub fn active_connection_mut(&mut self) -> Option<&mut ModelConnectionConfig> {
        let id = self.default_connection_id.clone().or_else(|| {
            self.active_connection()
                .map(|connection| connection.id.clone())
        });
        id.and_then(|id| {
            self.connections
                .iter_mut()
                .find(|connection| connection.id == id)
        })
    }

    pub fn upsert_connection(&mut self, mut connection: ModelConnectionConfig) {
        if connection.id.trim().is_empty() {
            connection.id = format!("{}:{}", connection.provider_id, connection.route_id);
        }
        if let Some(existing) = self
            .connections
            .iter_mut()
            .find(|item| item.id == connection.id)
        {
            *existing = connection;
        } else {
            self.connections.push(connection);
        }
    }

    fn ensure_connections_from_legacy(&mut self) {
        if self.connections.is_empty() {
            if let Some(openai) = &self.openai {
                let (provider_id, route_id, name) =
                    infer_openai_compatible_provider(openai.base_url.as_deref(), &openai.model);
                self.connections.push(ModelConnectionConfig {
                    id: format!("{provider_id}:{route_id}"),
                    name,
                    provider_id,
                    route_id,
                    protocol: "openai-compatible".to_string(),
                    api_key: openai.api_key.clone(),
                    model: openai.model.clone(),
                    base_url: Some(normalize_base_url(
                        openai
                            .base_url
                            .as_deref()
                            .unwrap_or("https://api.openai.com/v1"),
                    )),
                    enabled: true,
                    auth_header: None,
                });
            }
            if let Some(anthropic) = &self.anthropic {
                self.connections.push(ModelConnectionConfig {
                    id: "anthropic:anthropic-default".to_string(),
                    name: "Anthropic".to_string(),
                    provider_id: "anthropic".to_string(),
                    route_id: "anthropic-default".to_string(),
                    protocol: "anthropic".to_string(),
                    api_key: anthropic.api_key.clone(),
                    model: anthropic.model.clone(),
                    base_url: Some(normalize_base_url(
                        anthropic
                            .base_url
                            .as_deref()
                            .unwrap_or("https://api.anthropic.com/v1"),
                    )),
                    enabled: true,
                    auth_header: None,
                });
            }
        }
        if self.default_connection_id.is_none() {
            let desired = self.default_provider.clone();
            self.default_connection_id = self
                .connections
                .iter()
                .find(|connection| connection.provider_id == desired)
                .or_else(|| self.connections.first())
                .map(|connection| connection.id.clone());
        }
        if let Some(connection) = self.active_connection() {
            self.default_provider = connection.provider_id.clone();
        }
    }

    pub(crate) fn sync_legacy_slots_from_connections(&mut self) {
        let active = self.active_connection().cloned();
        if let Some(connection) = active.as_ref() {
            self.default_provider = connection.provider_id.clone();
            self.default_connection_id = Some(connection.id.clone());
        }
        if let Some(openai) = active
            .as_ref()
            .filter(|connection| connection.protocol == "openai-compatible")
            .or_else(|| {
                self.connections
                    .iter()
                    .find(|connection| connection.protocol == "openai-compatible")
            })
        {
            self.openai = Some(ProviderConfig {
                api_key: openai.api_key.clone(),
                model: openai.model.clone(),
                base_url: openai.base_url.clone(),
            });
        }
        if let Some(anthropic) = active
            .as_ref()
            .filter(|connection| connection.protocol == "anthropic")
            .or_else(|| {
                self.connections
                    .iter()
                    .find(|connection| connection.protocol == "anthropic")
            })
        {
            self.anthropic = Some(ProviderConfig {
                api_key: anthropic.api_key.clone(),
                model: anthropic.model.clone(),
                base_url: anthropic.base_url.clone(),
            });
        }
    }
}

impl ModelConnectionConfig {
    pub fn is_local_runtime(&self) -> bool {
        matches!(self.provider_id.as_str(), "ollama" | "lmstudio")
            || self
                .base_url
                .as_deref()
                .map(|url| url.contains("localhost") || url.contains("127.0.0.1"))
                .unwrap_or(false)
    }
}

pub fn normalize_base_url(value: &str) -> String {
    let mut next = value.trim().trim_end_matches('/').to_string();
    for suffix in [
        "/chat/completions",
        "/v1/chat/completions",
        "/messages",
        "/v1/messages",
        "/models",
        "/v1/models",
    ] {
        if next.to_lowercase().ends_with(suffix) {
            next.truncate(next.len() - suffix.len());
            next = next.trim_end_matches('/').to_string();
        }
    }
    next
}

fn infer_openai_compatible_provider(
    base_url: Option<&str>,
    model: &str,
) -> (String, String, String) {
    let base = base_url.unwrap_or("").to_lowercase();
    let model = model.to_lowercase();
    if base.contains("xiaomimimo.com") || model.starts_with("mimo-") {
        return (
            "xiaomi-mimo".to_string(),
            "mimo-standard".to_string(),
            "小米 MiMo".to_string(),
        );
    }
    if base.contains("deepseek.com") || model.contains("deepseek") {
        return (
            "deepseek".to_string(),
            "deepseek-openai".to_string(),
            "DeepSeek".to_string(),
        );
    }
    if base.contains("openai.azure.com") || base.contains("azure.com/openai") {
        return (
            "azure-openai".to_string(),
            "azure-openai".to_string(),
            "Azure OpenAI".to_string(),
        );
    }
    if base.contains("siliconflow.cn") {
        return (
            "siliconflow".to_string(),
            "siliconflow-openai".to_string(),
            "硅基流动".to_string(),
        );
    }
    if base.contains("generativelanguage.googleapis.com") {
        return (
            "gemini".to_string(),
            "gemini-openai".to_string(),
            "Gemini".to_string(),
        );
    }
    if base.contains("openrouter.ai") {
        return (
            "openrouter".to_string(),
            "openrouter-default".to_string(),
            "OpenRouter".to_string(),
        );
    }
    if base.contains("dashscope-intl") {
        return (
            "aliyun-bailian".to_string(),
            "bailian-intl".to_string(),
            "阿里云百炼".to_string(),
        );
    }
    if base.contains("dashscope.aliyuncs.com") || model.starts_with("qwen") {
        return (
            "aliyun-bailian".to_string(),
            "bailian-cn".to_string(),
            "阿里云百炼".to_string(),
        );
    }
    if base.contains("/api/coding") {
        return (
            "volcengine-ark".to_string(),
            "ark-coding-plan".to_string(),
            "火山方舟".to_string(),
        );
    }
    if base.contains("ark.cn-beijing.volces.com") || model.contains("doubao") {
        return (
            "volcengine-ark".to_string(),
            "ark-standard".to_string(),
            "火山方舟".to_string(),
        );
    }
    if base.contains("bigmodel.cn") || model.starts_with("glm-") {
        return (
            "zai".to_string(),
            "zai-openai".to_string(),
            "智谱 AI / Z.ai".to_string(),
        );
    }
    if base.contains("moonshot.ai") || model.contains("kimi") || model.contains("moonshot") {
        return (
            "moonshot-kimi".to_string(),
            "kimi-openai".to_string(),
            "Kimi / 月之暗面".to_string(),
        );
    }
    if base.contains("qianfan") || model.contains("ernie") {
        return (
            "baidu-qianfan".to_string(),
            "qianfan-cn".to_string(),
            "百度千帆".to_string(),
        );
    }
    if base.contains("hunyuan") || model.contains("hunyuan") {
        return (
            "tencent-hunyuan".to_string(),
            "hunyuan-openai".to_string(),
            "腾讯混元".to_string(),
        );
    }
    if base.contains("spark-api") || base.contains("xf-yun.com") || model.contains("spark") {
        return (
            "spark".to_string(),
            "spark-openai".to_string(),
            "讯飞星火".to_string(),
        );
    }
    if base.contains("minimaxi.com") {
        return (
            "minimax".to_string(),
            "minimax-cn".to_string(),
            "MiniMax".to_string(),
        );
    }
    if base.contains("minimax.io") || model.starts_with("minimax-") {
        return (
            "minimax".to_string(),
            "minimax-global".to_string(),
            "MiniMax".to_string(),
        );
    }
    if model.contains("gemini") {
        return (
            "gemini".to_string(),
            "gemini-openai".to_string(),
            "Gemini".to_string(),
        );
    }
    if base.contains("localhost:11434") {
        return (
            "ollama".to_string(),
            "ollama-local".to_string(),
            "Ollama".to_string(),
        );
    }
    if base.contains("localhost:1234") {
        return (
            "lmstudio".to_string(),
            "lmstudio-local".to_string(),
            "LM Studio".to_string(),
        );
    }
    (
        "openai".to_string(),
        "openai-default".to_string(),
        "OpenAI".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.llm.default_provider, "openai");
        assert_eq!(
            config.llm.openai.as_ref().unwrap().base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(
            config.llm.default_connection_id.as_deref(),
            Some("openai:openai-default")
        );
        assert_eq!(config.llm.connections.len(), 2);
        assert!(config.llm.openai.is_some());
        assert!(config.llm.anthropic.is_some());
        assert!(config.ui.sound_enabled);
    }

    #[test]
    fn test_loadable_default_allows_missing_api_key() {
        let config = Config::default();
        assert_eq!(config.llm.default_provider, "openai");
    }

    #[test]
    fn test_validate_for_chat_missing_api_key() {
        let config = Config::default();
        let result = config.validate_for_chat();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ConfigError::MissingApiKey(_)));
    }

    #[test]
    fn test_validate_with_api_key() {
        let mut config = Config::default();
        if let Some(openai) = config.llm.active_connection_mut() {
            openai.api_key = "test-key".to_string();
        }
        config.llm.sync_legacy_slots_from_connections();
        let result = config.validate_for_chat();
        assert!(result.is_ok());
    }

    #[test]
    fn test_client_config_keeps_local_model_keys_but_redacts_tmdb_key() {
        let mut config = Config::default();
        if let Some(openai) = &mut config.llm.openai {
            openai.api_key = "openai-secret".to_string();
        }
        if let Some(anthropic) = &mut config.llm.anthropic {
            anthropic.api_key = "anthropic-secret".to_string();
        }
        for connection in &mut config.llm.connections {
            connection.api_key = format!("{}-secret", connection.provider_id);
        }
        config.tmdb.api_key = "tmdb-secret".to_string();

        let redacted = config.redacted_for_client();
        assert_eq!(redacted.llm.openai.unwrap().api_key, "openai-secret");
        assert_eq!(redacted.llm.anthropic.unwrap().api_key, "anthropic-secret");
        assert!(redacted
            .llm
            .connections
            .iter()
            .all(|connection| connection.api_key.ends_with("-secret")));
        assert_eq!(redacted.tmdb.api_key, "");
    }

    #[test]
    fn persisted_config_shape_keeps_api_keys() {
        let mut config = Config::default();
        if let Some(openai) = config.llm.active_connection_mut() {
            openai.api_key = "openai-secret".to_string();
        }
        config.llm.sync_legacy_slots_from_connections();

        let serialized = toml::to_string_pretty(&config).unwrap();

        assert!(serialized.contains("openai-secret"));
        assert_eq!(
            config.redacted_for_client().llm.openai.unwrap().api_key,
            "openai-secret"
        );
    }

    #[test]
    fn aura_home_env_redirects_config_load_and_save() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        let dir = std::env::temp_dir().join(format!("aura_config_home_{}", uuid::Uuid::new_v4()));
        std::env::set_var("AURA_HOME", &dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut config = Config::default();
        if let Some(connection) = config.llm.active_connection_mut() {
            connection.api_key = "aura-home-secret".to_string();
            connection.model = "aura-home-model".to_string();
        }
        config.save().unwrap();

        let config_path = dir.join("config.toml");
        assert!(config_path.exists());
        assert!(std::fs::read_to_string(&config_path)
            .unwrap()
            .contains("aura-home-secret"));
        let loaded = Config::load().unwrap();
        let active = loaded.llm.active_connection().unwrap();
        assert_eq!(active.api_key, "aura-home-secret");
        assert_eq!(active.model, "aura-home-model");

        std::env::remove_var("AURA_HOME");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_apply_env_overrides() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        std::env::set_var("OPENAI_API_KEY", "env-test-key");
        let mut config = Config::default();
        config.llm.upsert_connection(ModelConnectionConfig {
            id: "deepseek:deepseek-openai".to_string(),
            name: "DeepSeek".to_string(),
            provider_id: "deepseek".to_string(),
            route_id: "deepseek-openai".to_string(),
            protocol: "openai-compatible".to_string(),
            api_key: "deepseek-key".to_string(),
            model: "deepseek-chat".to_string(),
            base_url: Some("https://api.deepseek.com/v1".to_string()),
            enabled: true,
            auth_header: None,
        });
        Config::apply_env_overrides(&mut config);

        assert_eq!(config.llm.openai.as_ref().unwrap().api_key, "env-test-key");
        assert_eq!(
            config
                .llm
                .connections
                .iter()
                .find(|connection| connection.provider_id == "openai")
                .unwrap()
                .api_key,
            "env-test-key"
        );
        assert_eq!(
            config
                .llm
                .connections
                .iter()
                .find(|connection| connection.provider_id == "deepseek")
                .unwrap()
                .api_key,
            "deepseek-key"
        );

        std::env::remove_var("OPENAI_API_KEY");
    }

    #[test]
    fn test_anthropic_provider() {
        let mut config = Config::default();
        config.llm.default_connection_id = Some("anthropic:anthropic-default".to_string());
        if let Some(anthropic) = config.llm.active_connection_mut() {
            anthropic.api_key = "test-key".to_string();
        }
        config.llm.sync_legacy_slots_from_connections();
        let result = config.validate_for_chat();
        assert!(result.is_ok());
    }

    #[test]
    fn test_migrate_legacy_deepseek_provider() {
        let mut config = Config::default();
        config.llm.default_provider = "deepseek".to_string();
        config.llm.default_connection_id = None;
        if let Some(openai) = &mut config.llm.openai {
            openai.base_url = None;
            openai.model.clear();
        }
        config.migrate_legacy_providers();

        assert_eq!(config.llm.default_provider, "deepseek");
        assert_eq!(
            config.llm.default_connection_id.as_deref(),
            Some("deepseek:deepseek-openai")
        );
        assert_eq!(
            config.llm.openai.as_ref().unwrap().base_url.as_deref(),
            Some("https://api.deepseek.com/v1")
        );
    }

    #[test]
    fn test_migrate_legacy_mimo_chat_completion_url() {
        let content = r#"
[llm]
default_provider = "openai"

[llm.openai]
api_key = "mimo-key"
model = "mimo-v4-flash"
base_url = "https://api.xiaomimimo.com/v1/chat/completions"

[llm.anthropic]
api_key = "claude-key"
model = "claude-opus-4-8"
base_url = "https://api.anthropic.com/v1"
"#;
        let mut config: Config = toml::from_str(content).unwrap();
        config.migrate_legacy_providers();

        let active = config.llm.active_connection().unwrap();
        assert_eq!(active.provider_id, "xiaomi-mimo");
        assert_eq!(active.route_id, "mimo-standard");
        assert_eq!(active.api_key, "mimo-key");
        assert_eq!(
            active.base_url.as_deref(),
            Some("https://api.xiaomimimo.com/v1")
        );
        assert!(config
            .llm
            .connections
            .iter()
            .any(|connection| connection.provider_id == "anthropic"
                && connection.api_key == "claude-key"));
    }

    #[test]
    fn infer_openai_compatible_provider_covers_alias_and_edge_routes() {
        let cases = [
            (
                Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
                "qwen-max",
                "aliyun-bailian",
                "bailian-cn",
            ),
            (
                Some("https://ark.cn-beijing.volces.com/api/v3"),
                "doubao-1.5-pro",
                "volcengine-ark",
                "ark-standard",
            ),
            (
                Some("https://open.bigmodel.cn/api/paas/v4"),
                "glm-4-plus",
                "zai",
                "zai-openai",
            ),
            (
                Some("https://api.moonshot.ai/v1"),
                "moonshot-v1-128k",
                "moonshot-kimi",
                "kimi-openai",
            ),
            (
                Some("https://qianfan.baidubce.com/v2"),
                "ernie-4.0-turbo",
                "baidu-qianfan",
                "qianfan-cn",
            ),
            (
                Some("https://api.hunyuan.cloud.tencent.com/v1"),
                "hunyuan-pro",
                "tencent-hunyuan",
                "hunyuan-openai",
            ),
            (
                Some("https://spark-api-open.xf-yun.com/v1"),
                "spark-max",
                "spark",
                "spark-openai",
            ),
            (
                Some("https://example.openai.azure.com/openai/deployments/aura"),
                "gpt-4o",
                "azure-openai",
                "azure-openai",
            ),
            (
                Some("https://generativelanguage.googleapis.com/v1beta/openai"),
                "gemini-1.5-flash",
                "gemini",
                "gemini-openai",
            ),
            (
                Some("https://openrouter.ai/api/v1"),
                "openai/gpt-4o-mini",
                "openrouter",
                "openrouter-default",
            ),
            (
                Some("https://api.siliconflow.cn/v1"),
                "Qwen/Qwen2.5-72B-Instruct",
                "siliconflow",
                "siliconflow-openai",
            ),
        ];

        for (base_url, model, expected_provider, expected_route) in cases {
            let (provider, route, name) = infer_openai_compatible_provider(base_url, model);
            assert_eq!(provider, expected_provider, "{name} provider mismatch");
            assert_eq!(route, expected_route, "{name} route mismatch");
        }
    }
}
