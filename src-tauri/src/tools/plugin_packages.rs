use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent::{evaluate_plugin_quality_gate, AgentError, ToolResult, ToolSchema};
use crate::mcp::invoke_mcp_tool;
use crate::storage::{LocalDb, LogPluginCapabilityEventPayload, PluginPackageRecord};
use crate::tools::outbound::{active_policy, OutboundChannel, OutboundDecision};
use crate::tools::{
    output_limit, CommandIsolationPolicy, RunCommandTool, Tool, ToolCapability, ToolMetadata,
    ToolRegistry, ToolSafetyLevel,
};

const TOOL_INSTALL_PLUGIN_PACKAGE: &str = "install_plugin_package";
const TOOL_LIST_PLUGIN_PACKAGES: &str = "list_plugin_packages";
const TOOL_SET_PLUGIN_PACKAGE_ENABLED: &str = "set_plugin_package_enabled";
const TOOL_INVOKE_PLUGIN_CAPABILITY: &str = "invoke_plugin_capability";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPackageManifest {
    pub id: String,
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub capabilities: Vec<PluginCapabilityManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCapabilityManifest {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_safe_risk")]
    pub risk: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub input_schema: Option<Value>,
}

pub struct InstallPluginPackageTool {
    db: LocalDb,
}

impl InstallPluginPackageTool {
    pub fn new(db: LocalDb) -> Self {
        Self { db }
    }
}

#[async_trait]
impl Tool for InstallPluginPackageTool {
    fn name(&self) -> &str {
        TOOL_INSTALL_PLUGIN_PACKAGE
    }

    fn description(&self) -> &str {
        "Install a plugin capability package after confirmation, preserving source, risk, permissions, and manifest."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "manifest": {
                        "type": "object",
                        "description": "Plugin manifest with id/name/version/source/capabilities."
                    },
                    "enabled": {
                        "type": "boolean",
                        "description": "Whether the package should be enabled immediately after installation."
                    },
                    "trusted": {
                        "type": "boolean",
                        "description": "Whether the user explicitly trusts this package source."
                    },
                    "confirmed": {
                        "type": "boolean",
                        "description": "Must be true after user confirmation. Without this, the tool only returns a preview."
                    }
                },
                "required": ["manifest", "confirmed"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "安装插件能力包".to_string(),
            description_zh: "安装带来源、权限和风险标注的插件能力包。".to_string(),
            capability_labels_zh: vec![
                "插件".to_string(),
                "本地数据".to_string(),
                "需确认".to_string(),
            ],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::LocalData],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: true,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let manifest_value = args
            .get("manifest")
            .cloned()
            .ok_or_else(|| AgentError::Tool("缺少 manifest 参数。".to_string()))?;
        let manifest = parse_manifest(manifest_value.clone())?;
        let enabled = args
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let trusted = args
            .get("trusted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut preview =
            package_record_from_manifest(&manifest, manifest_value.clone(), trusted, enabled)?;
        let quality_gate = evaluate_plugin_quality_gate(&preview, false);
        if enabled && !quality_gate.can_enable {
            preview.enabled = false;
        }
        let confirmed = args
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !confirmed {
            return Ok(ToolResult::warning(
                format!("插件能力包待确认：{} {}", preview.name, preview.version),
                json!({
                    "playbackState": "requires_confirmation",
                    "plugin": preview,
                    "message": "安装插件会改变 Agent 可用能力，必须确认。"
                }),
                vec!["确认来源可信后用 confirmed=true 重试。".to_string()],
            ));
        }

        let installed = self
            .db
            .upsert_plugin_package(preview)
            .map_err(|error| AgentError::Tool(error.to_string()))?;
        let _ = self
            .db
            .log_plugin_capability_event(LogPluginCapabilityEventPayload {
                plugin_id: installed.id.clone(),
                capability_id: "*".to_string(),
                action: "install".to_string(),
                status: "ok".to_string(),
                risk: installed.risk.clone(),
                reason: "用户确认安装插件能力包。".to_string(),
                input: manifest_value,
                output: json!({
                    "plugin": installed,
                    "qualityGate": quality_gate
                }),
            });
        if enabled && !quality_gate.can_enable {
            return Ok(ToolResult::warning(
                format!(
                    "插件能力包已安装但未启用：{} {}",
                    installed.name, installed.version
                ),
                json!({ "plugin": installed, "qualityGate": quality_gate }),
                vec!["质量门禁未通过，插件保持禁用；补齐 eval/权限/可信来源后再启用。".to_string()],
            ));
        }
        Ok(ToolResult::success(
            format!("插件能力包已安装：{} {}", installed.name, installed.version),
            json!({ "plugin": installed, "qualityGate": quality_gate }),
        ))
    }
}

pub struct ListPluginPackagesTool {
    db: LocalDb,
}

impl ListPluginPackagesTool {
    pub fn new(db: LocalDb) -> Self {
        Self { db }
    }
}

#[async_trait]
impl Tool for ListPluginPackagesTool {
    fn name(&self) -> &str {
        TOOL_LIST_PLUGIN_PACKAGES
    }

    fn description(&self) -> &str {
        "List installed plugin capability packages with source, risk, permissions, and enabled state."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata::safe_readonly(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<ToolResult, AgentError> {
        let packages = self
            .db
            .list_plugin_packages()
            .map_err(|error| AgentError::Tool(error.to_string()))?;
        Ok(ToolResult::success(
            format!("已安装插件能力包：{} 个", packages.len()),
            json!({ "plugins": packages }),
        ))
    }
}

pub struct SetPluginPackageEnabledTool {
    db: LocalDb,
}

impl SetPluginPackageEnabledTool {
    pub fn new(db: LocalDb) -> Self {
        Self { db }
    }
}

#[async_trait]
impl Tool for SetPluginPackageEnabledTool {
    fn name(&self) -> &str {
        TOOL_SET_PLUGIN_PACKAGE_ENABLED
    }

    fn description(&self) -> &str {
        "Enable or disable an installed plugin package after confirmation."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pluginId": { "type": "string" },
                    "enabled": { "type": "boolean" },
                    "reason": { "type": "string" },
                    "confirmed": { "type": "boolean" }
                },
                "required": ["pluginId", "enabled", "reason", "confirmed"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "启用/停用插件包".to_string(),
            description_zh: "修改插件能力包启用状态，并记录原因。".to_string(),
            capability_labels_zh: vec![
                "插件".to_string(),
                "本地数据".to_string(),
                "需确认".to_string(),
            ],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::LocalData],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: true,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let plugin_id = required_str(&args, "pluginId")?;
        let enabled = args
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| AgentError::Tool("缺少 enabled 参数。".to_string()))?;
        let reason = required_str(&args, "reason")?;
        let confirmed = args
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !confirmed {
            return Ok(ToolResult::warning(
                "插件启用状态变更待确认。",
                json!({
                    "playbackState": "requires_confirmation",
                    "pluginId": plugin_id,
                    "enabled": enabled,
                    "reason": reason
                }),
                vec!["确认后用 confirmed=true 重试。".to_string()],
            ));
        }
        let existing = self
            .db
            .list_plugin_packages()
            .map_err(|error| AgentError::Tool(error.to_string()))?
            .into_iter()
            .find(|package| package.id == plugin_id)
            .ok_or_else(|| AgentError::Tool(format!("未找到插件能力包：{plugin_id}")))?;
        if enabled {
            let quality_gate = evaluate_plugin_quality_gate(&existing, false);
            if !quality_gate.can_enable {
                let _ = self
                    .db
                    .log_plugin_capability_event(LogPluginCapabilityEventPayload {
                        plugin_id: plugin_id.clone(),
                        capability_id: "*".to_string(),
                        action: "enable".to_string(),
                        status: "blocked".to_string(),
                        risk: existing.risk.clone(),
                        reason: quality_gate.reasons.join("; "),
                        input: json!({ "enabled": enabled, "reason": reason }),
                        output: serde_json::to_value(&quality_gate).unwrap_or(Value::Null),
                    });
                return Ok(ToolResult::error(
                    "插件质量门禁未通过，不能启用。".to_string(),
                    vec![format!("门禁原因：{}", quality_gate.reasons.join("; "))],
                ));
            }
        }
        let record = self
            .db
            .set_plugin_package_enabled(&plugin_id, enabled)
            .map_err(|error| AgentError::Tool(error.to_string()))?;
        let action = if enabled { "enable" } else { "disable" };
        let _ = self
            .db
            .log_plugin_capability_event(LogPluginCapabilityEventPayload {
                plugin_id: plugin_id.clone(),
                capability_id: "*".to_string(),
                action: action.to_string(),
                status: "ok".to_string(),
                risk: record.risk.clone(),
                reason: reason.clone(),
                input: json!({ "enabled": enabled, "reason": reason }),
                output: serde_json::to_value(&record).unwrap_or(Value::Null),
            });
        Ok(ToolResult::success(
            format!(
                "插件能力包已{}：{}",
                if enabled { "启用" } else { "停用" },
                record.name
            ),
            json!({ "plugin": record }),
        ))
    }
}

pub struct InvokePluginCapabilityTool {
    db: LocalDb,
}

impl InvokePluginCapabilityTool {
    pub fn new(db: LocalDb) -> Self {
        Self { db }
    }
}

#[async_trait]
impl Tool for InvokePluginCapabilityTool {
    fn name(&self) -> &str {
        TOOL_INVOKE_PLUGIN_CAPABILITY
    }

    fn description(&self) -> &str {
        "Invoke a capability from an enabled plugin package through Aura's plugin guardrail boundary."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pluginId": { "type": "string" },
                    "capabilityId": { "type": "string" },
                    "input": { "type": "object" },
                    "confirmed": { "type": "boolean" }
                },
                "required": ["pluginId", "capabilityId"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        // This tool can invoke any enabled capability, including `script`
        // (shell) and network kinds, so it is classified by its most powerful
        // reachable effect (System) rather than as read-only local data —
        // otherwise it would execute in Plan mode (M-8 follow-up fix).
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "调用插件能力".to_string(),
            description_zh: "通过插件权限边界调用已启用能力。".to_string(),
            capability_labels_zh: vec!["插件".to_string(), "外部能力".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::System],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: true,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let plugin_id = required_str(&args, "pluginId")?;
        let capability_id = required_str(&args, "capabilityId")?;
        let input = args.get("input").cloned().unwrap_or_else(|| json!({}));
        let confirmed = args
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        invoke_plugin_capability(&self.db, &plugin_id, &capability_id, input, confirmed).await
    }
}

pub struct PluginCapabilityTool {
    db: LocalDb,
    plugin_id: String,
    capability: PluginCapabilityManifest,
    tool_name: String,
    metadata: ToolMetadata,
}

#[async_trait]
impl Tool for PluginCapabilityTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.capability.description
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self
                .capability
                .input_schema
                .clone()
                .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        self.metadata.clone()
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let confirmed = args
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        invoke_plugin_capability(
            &self.db,
            &self.plugin_id,
            &self.capability.id,
            args,
            confirmed,
        )
        .await
    }
}

pub fn register_installed_plugin_capabilities(registry: &mut ToolRegistry, db: LocalDb) {
    let packages = match db.list_enabled_plugin_packages() {
        Ok(packages) => packages,
        Err(error) => {
            eprintln!("Aura plugin capability load failed: {error}");
            return;
        }
    };
    for package in packages {
        for capability in capabilities_from_record(&package) {
            if !capability.enabled || !is_interpreted_capability(&capability.kind) {
                continue;
            }
            let tool_name = plugin_tool_name(&package.id, &capability.id);
            if tool_name == "plugin__" {
                continue;
            }
            registry.register(Box::new(PluginCapabilityTool {
                db: db.clone(),
                plugin_id: package.id.clone(),
                metadata: metadata_for_capability(&tool_name, &capability),
                tool_name,
                capability,
            }));
        }
    }
}

pub fn plugin_tool_name(plugin_id: &str, capability_id: &str) -> String {
    format!(
        "plugin_{}_{}",
        sanitize_tool_segment(plugin_id),
        sanitize_tool_segment(capability_id)
    )
}

async fn invoke_plugin_capability(
    db: &LocalDb,
    plugin_id: &str,
    capability_id: &str,
    input: Value,
    confirmed: bool,
) -> Result<ToolResult, AgentError> {
    let package = db
        .get_plugin_package(plugin_id)
        .map_err(|error| AgentError::Tool(error.to_string()))?;
    if !package.enabled {
        log_plugin_event(
            db,
            plugin_id,
            capability_id,
            "invoke",
            "blocked",
            &package.risk,
            "插件能力包已停用。",
            input,
            Value::Null,
        );
        return Ok(ToolResult::error(
            "插件能力包已停用。",
            vec!["启用插件包后再调用。".to_string()],
        ));
    }
    let capability = capabilities_from_record(&package)
        .into_iter()
        .find(|capability| capability.id == capability_id)
        .ok_or_else(|| AgentError::Tool(format!("插件能力不存在：{plugin_id}/{capability_id}")))?;
    if !capability.enabled {
        log_plugin_event(
            db,
            plugin_id,
            capability_id,
            "invoke",
            "blocked",
            &capability.risk,
            "插件能力已停用。",
            input,
            Value::Null,
        );
        return Ok(ToolResult::error(
            "插件能力已停用。",
            vec!["启用该能力后再调用。".to_string()],
        ));
    }
    let risk = normalize_risk(&capability.risk);
    if risk != "safe" && !confirmed {
        return Ok(ToolResult::warning(
            format!("插件能力需要确认：{plugin_id}/{capability_id}"),
            json!({
                "playbackState": "requires_confirmation",
                "pluginId": plugin_id,
                "capabilityId": capability_id,
                "risk": risk,
                "permissions": capability.permissions
            }),
            vec!["确认插件来源和权限后用 confirmed=true 重试。".to_string()],
        ));
    }
    let output = match normalize_kind(&capability.kind).as_str() {
        "skill" | "instruction" => invoke_text_capability(&package, &capability, &risk),
        "mcp" => {
            let server_id = capability.server_id.as_deref().unwrap_or_default();
            let tool_name = capability.tool_name.as_deref().unwrap_or_default();
            let result = invoke_mcp_tool(db, server_id, tool_name, input.clone(), confirmed)
                .await
                .map_err(AgentError::Tool)?;
            let bounded = output_limit::truncate_middle(
                &result.output.to_string(),
                output_limit::MAX_TOOL_OUTPUT_CHARS,
            );
            let mut data = serde_json::to_value(&result).unwrap_or_else(|_| json!({}));
            let truncation = bounded.meta();
            if bounded.truncated {
                data["output"] = Value::String(bounded.text);
            }
            data["truncation"] = truncation;
            data["adapter"] = json!("mcp");
            data
        }
        "script" => {
            let command = capability.command.as_deref().unwrap_or_default().trim();
            let mut args = json!({
                "command": command,
                "timeout_ms": input
                    .get("timeoutMs")
                    .or_else(|| input.get("timeout_ms"))
                    .and_then(Value::as_u64)
                    .unwrap_or(30_000)
            });
            if let Some(cwd) = input.get("cwd").and_then(Value::as_str) {
                args["cwd"] = Value::String(cwd.to_string());
            }
            let result = RunCommandTool::new(CommandIsolationPolicy::default_for_current_dir())
                .execute(args)
                .await?;
            json!({
                "adapter": "script",
                "pluginId": plugin_id,
                "capabilityId": capability_id,
                "result": result,
            })
        }
        "tool" | "connector" => invoke_http_adapter(&capability, input.clone())
            .await
            .map_err(AgentError::Tool)?,
        other => {
            return Err(AgentError::Tool(format!(
                "插件能力类型没有可执行适配器：{other}"
            )))
        }
    };
    log_plugin_event(
        db,
        plugin_id,
        capability_id,
        "invoke",
        "ok",
        &risk,
        "插件 skill/instruction 能力已按最小权限返回内容。",
        input,
        output.clone(),
    );
    Ok(ToolResult::success(
        format!("插件能力已调用：{plugin_id}/{capability_id}"),
        output,
    ))
}

fn invoke_text_capability(
    package: &PluginPackageRecord,
    capability: &PluginCapabilityManifest,
    risk: &str,
) -> Value {
    let content = capability
        .content
        .clone()
        .unwrap_or_default()
        .trim()
        .to_string();
    let bounded = output_limit::truncate_middle(&content, output_limit::MAX_TOOL_OUTPUT_CHARS);
    json!({
        "pluginId": package.id,
        "capabilityId": capability.id,
        "kind": capability.kind,
        "adapter": "text",
        "source": package.source,
        "trusted": package.trusted,
        "risk": risk,
        "permissions": capability.permissions,
        "content": bounded.text,
        "truncation": bounded.meta(),
        "untrustedExternal": !package.trusted
    })
}

async fn invoke_http_adapter(
    capability: &PluginCapabilityManifest,
    input: Value,
) -> Result<Value, String> {
    let endpoint = capability
        .endpoint
        .as_deref()
        .or(capability.content.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "tool/connector capability requires endpoint or content URL".to_string())?;
    let channel = if normalize_kind(&capability.kind) == "connector" {
        OutboundChannel::WebTool
    } else {
        OutboundChannel::McpServer
    };
    match active_policy().evaluate_url(channel, endpoint, &[]) {
        OutboundDecision::Allow => {}
        OutboundDecision::NeedsConsent { reason } | OutboundDecision::Deny { reason } => {
            return Err(reason)
        }
    }
    let screened = crate::tools::outbound::screen_egress(&input.to_string());
    let method = capability
        .method
        .as_deref()
        .unwrap_or("POST")
        .trim()
        .to_ascii_uppercase();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let response = if method == "GET" {
        client.get(endpoint).send().await
    } else {
        client
            .post(endpoint)
            .header("content-type", "application/json")
            .body(screened.masked.clone())
            .send()
            .await
    }
    .map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let text = response.text().await.map_err(|error| error.to_string())?;
    let bounded = output_limit::truncate_middle(&text, output_limit::MAX_TOOL_OUTPUT_CHARS);
    Ok(json!({
        "adapter": "http",
        "kind": capability.kind,
        "endpointHost": crate::tools::outbound::audit_target_host(endpoint),
        "method": method,
        "status": status,
        "secretHits": screened.secret_count,
        "output": bounded.text,
        "truncation": bounded.meta()
    }))
}

fn package_record_from_manifest(
    manifest: &PluginPackageManifest,
    manifest_value: Value,
    trusted: bool,
    enabled: bool,
) -> Result<PluginPackageRecord, AgentError> {
    validate_manifest(manifest)?;
    let capabilities_json = serde_json::to_value(&manifest.capabilities)
        .map_err(|error| AgentError::Tool(error.to_string()))?;
    let permissions = aggregate_permissions(&manifest.capabilities);
    Ok(PluginPackageRecord {
        id: normalize_id(&manifest.id),
        name: manifest.name.trim().to_string(),
        version: manifest.version.trim().to_string(),
        source: manifest.source.trim().to_string(),
        description: manifest.description.trim().to_string(),
        trusted,
        enabled,
        risk: aggregate_risk(&manifest.capabilities),
        permissions: json!(permissions),
        capabilities: capabilities_json,
        manifest: manifest_value,
        installed_at: 0,
        updated_at: 0,
    })
}

fn parse_manifest(value: Value) -> Result<PluginPackageManifest, AgentError> {
    serde_json::from_value(value).map_err(|error| AgentError::Tool(error.to_string()))
}

fn validate_manifest(manifest: &PluginPackageManifest) -> Result<(), AgentError> {
    if normalize_id(&manifest.id).is_empty() {
        return Err(AgentError::Tool("插件 id 不能为空。".to_string()));
    }
    if manifest.name.trim().is_empty() {
        return Err(AgentError::Tool("插件 name 不能为空。".to_string()));
    }
    if manifest.capabilities.is_empty() {
        return Err(AgentError::Tool("插件至少需要声明一个能力。".to_string()));
    }
    for capability in &manifest.capabilities {
        if normalize_id(&capability.id).is_empty() {
            return Err(AgentError::Tool("插件能力 id 不能为空。".to_string()));
        }
        if capability.name.trim().is_empty() {
            return Err(AgentError::Tool("插件能力 name 不能为空。".to_string()));
        }
        match normalize_kind(&capability.kind).as_str() {
            "skill" | "instruction" => {
                if capability
                    .content
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
                {
                    return Err(AgentError::Tool(
                        "skill/instruction 能力必须提供 content。".to_string(),
                    ));
                }
            }
            "mcp" => {
                if capability
                    .server_id
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                    || capability
                        .tool_name
                        .as_deref()
                        .unwrap_or("")
                        .trim()
                        .is_empty()
                {
                    return Err(AgentError::Tool(
                        "mcp 能力必须提供 serverId 和 toolName。".to_string(),
                    ));
                }
            }
            "script" => {
                if capability
                    .command
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
                    return Err(AgentError::Tool(
                        "script 能力必须提供 command。".to_string(),
                    ));
                }
            }
            "tool" | "connector" => {
                let has_endpoint = capability
                    .endpoint
                    .as_deref()
                    .or(capability.content.as_deref())
                    .map(str::trim)
                    .is_some_and(|value| !value.is_empty());
                if !has_endpoint {
                    return Err(AgentError::Tool(
                        "tool/connector 能力必须提供 endpoint 或 content URL。".to_string(),
                    ));
                }
            }
            other => return Err(AgentError::Tool(format!("不支持的插件能力类型：{other}"))),
        }
    }
    Ok(())
}

fn capabilities_from_record(record: &PluginPackageRecord) -> Vec<PluginCapabilityManifest> {
    serde_json::from_value(record.capabilities.clone()).unwrap_or_default()
}

fn metadata_for_capability(tool_name: &str, capability: &PluginCapabilityManifest) -> ToolMetadata {
    let risk = normalize_risk(&capability.risk);
    // Classify policy capabilities by kind so executable plugin capabilities are
    // gated like their first-party equivalents instead of leaking through as
    // read-only: `script` runs shell commands (System), `mcp`/`tool`/`connector`
    // reach the network. Marking these read-only let them execute in Plan mode
    // and skip the network gate (M-8 follow-up fix).
    let (capabilities, mutates_state) = match normalize_kind(&capability.kind).as_str() {
        "script" => (vec![ToolCapability::System], true),
        "mcp" | "tool" | "connector" => (vec![ToolCapability::Network], true),
        _ => (vec![ToolCapability::ReadOnly], false),
    };
    ToolMetadata {
        name: tool_name.to_string(),
        description: capability.description.clone(),
        label_zh: capability.name.clone(),
        description_zh: capability.description.clone(),
        capability_labels_zh: vec!["插件".to_string(), capability.kind.clone()],
        safety_label_zh: if risk == "safe" { "安全" } else { "敏感" }.to_string(),
        capabilities,
        safety_level: if risk == "safe" {
            ToolSafetyLevel::Safe
        } else {
            ToolSafetyLevel::Sensitive
        },
        mutates_state,
        requires_confirmation: risk != "safe",
    }
}

#[allow(clippy::too_many_arguments)]
fn log_plugin_event(
    db: &LocalDb,
    plugin_id: &str,
    capability_id: &str,
    action: &str,
    status: &str,
    risk: &str,
    reason: &str,
    input: Value,
    output: Value,
) {
    let _ = db.log_plugin_capability_event(LogPluginCapabilityEventPayload {
        plugin_id: plugin_id.to_string(),
        capability_id: capability_id.to_string(),
        action: action.to_string(),
        status: status.to_string(),
        risk: risk.to_string(),
        reason: reason.to_string(),
        input,
        output,
    });
}

fn aggregate_permissions(capabilities: &[PluginCapabilityManifest]) -> Vec<String> {
    let mut permissions = std::collections::BTreeSet::new();
    for capability in capabilities {
        permissions.insert(match normalize_kind(&capability.kind).as_str() {
            "mcp" => "network".to_string(),
            "script" => "command".to_string(),
            "tool" => "tool".to_string(),
            "connector" => "connector".to_string(),
            _ => "read".to_string(),
        });
        for permission in &capability.permissions {
            let normalized = permission.trim().to_ascii_lowercase();
            if !normalized.is_empty() {
                permissions.insert(normalized);
            }
        }
    }
    permissions.into_iter().collect()
}

fn aggregate_risk(capabilities: &[PluginCapabilityManifest]) -> String {
    if capabilities
        .iter()
        .any(|capability| normalize_risk(&capability.risk) == "destructive")
    {
        "destructive".to_string()
    } else if capabilities
        .iter()
        .any(|capability| normalize_risk(&capability.risk) == "sensitive")
    {
        "sensitive".to_string()
    } else {
        "safe".to_string()
    }
}

fn is_interpreted_capability(kind: &str) -> bool {
    matches!(
        normalize_kind(kind).as_str(),
        "skill" | "instruction" | "mcp" | "script" | "tool" | "connector"
    )
}

fn normalize_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "skill" | "instruction" | "mcp" | "script" | "tool" | "connector" => {
            kind.trim().to_ascii_lowercase()
        }
        _ => "unknown".to_string(),
    }
}

fn normalize_risk(risk: &str) -> String {
    match risk.trim().to_ascii_lowercase().as_str() {
        "safe" | "sensitive" | "destructive" => risk.trim().to_ascii_lowercase(),
        _ => "sensitive".to_string(),
    }
}

fn normalize_id(id: &str) -> String {
    id.trim()
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                Some(ch.to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect()
}

fn sanitize_tool_segment(segment: &str) -> String {
    let normalized = normalize_id(segment).replace(['-', '.'], "_");
    normalized
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>()
}

fn required_str(args: &Value, key: &str) -> Result<String, AgentError> {
    args.get(key)
        .or_else(|| {
            let snake = key
                .chars()
                .flat_map(|ch| {
                    if ch.is_ascii_uppercase() {
                        vec!['_', ch.to_ascii_lowercase()]
                    } else {
                        vec![ch]
                    }
                })
                .collect::<String>();
            args.get(snake.as_str())
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| AgentError::Tool(format!("缺少 {key} 参数。")))
}

fn default_version() -> String {
    "0.1.0".to_string()
}

fn default_source() -> String {
    "local".to_string()
}

fn default_safe_risk() -> String {
    "safe".to_string()
}

fn default_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Value {
        json!({
            "id": "docs-helper",
            "name": "Docs Helper",
            "version": "1.0.0",
            "source": "fixture://docs-helper",
            "description": "Adds a review checklist skill.",
            "capabilities": [{
                "id": "review-checklist",
                "kind": "skill",
                "name": "Review Checklist",
                "description": "Return a code review checklist.",
                "risk": "safe",
                "permissions": ["read"],
                "content": "Check scope, tests, rollback, and user-visible behavior."
            }]
        })
    }

    fn cap_manifest(kind: &str, risk: &str) -> PluginCapabilityManifest {
        serde_json::from_value(json!({
            "id": "cap",
            "kind": kind,
            "name": "Cap",
            "description": "desc",
            "risk": risk,
            "command": "echo hi",
            "endpoint": "https://example.com/run"
        }))
        .unwrap()
    }

    #[test]
    fn executable_plugin_capabilities_are_gated_like_first_party_tools() {
        use crate::tools::policy::{AgentPermissionMode, PolicyDecision, PolicyEngine};

        // `script` runs shell commands -> System + mutating: denied in Plan and
        // Default, never silently read-only.
        let script =
            metadata_for_capability("plugin_p_script", &cap_manifest("script", "sensitive"));
        assert!(script.capabilities.contains(&ToolCapability::System));
        assert!(script.mutates_state);
        assert!(matches!(
            PolicyEngine::new(AgentPermissionMode::Plan).evaluate_tool_execution(&script),
            PolicyDecision::Deny { .. }
        ));
        assert!(matches!(
            PolicyEngine::new(AgentPermissionMode::Default).evaluate_tool_execution(&script),
            PolicyDecision::Deny { .. }
        ));

        // `connector`/`mcp`/`tool` reach the network -> denied in Plan mode (no
        // silent outbound during planning).
        for kind in ["connector", "mcp", "tool"] {
            let net = metadata_for_capability("plugin_p_net", &cap_manifest(kind, "sensitive"));
            assert!(
                net.capabilities.contains(&ToolCapability::Network),
                "{kind}"
            );
            assert!(
                matches!(
                    PolicyEngine::new(AgentPermissionMode::Plan).evaluate_tool_execution(&net),
                    PolicyDecision::Deny { .. }
                ),
                "{kind} must be denied in plan mode"
            );
        }

        // `skill` is text only -> stays read-only and usable while planning.
        let skill = metadata_for_capability("plugin_p_skill", &cap_manifest("skill", "safe"));
        assert!(skill.capabilities.contains(&ToolCapability::ReadOnly));
        assert!(!skill.mutates_state);
        assert!(matches!(
            PolicyEngine::new(AgentPermissionMode::Plan).evaluate_tool_execution(&skill),
            PolicyDecision::Allow
        ));

        // The generic invoke tool can reach any capability -> also denied in Plan.
        let generic = InvokePluginCapabilityTool::new(temp_db()).metadata();
        assert!(matches!(
            PolicyEngine::new(AgentPermissionMode::Plan).evaluate_tool_execution(&generic),
            PolicyDecision::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn install_requires_confirmation_then_persists_source_risk_and_permissions() {
        let db = temp_db();
        let tool = InstallPluginPackageTool::new(db.clone());
        let preview = tool
            .execute(json!({ "manifest": manifest(), "enabled": true, "confirmed": false }))
            .await
            .unwrap();
        assert!(matches!(
            preview.status,
            crate::agent::ToolResultStatus::Warning
        ));
        assert!(db.list_plugin_packages().unwrap().is_empty());

        let installed = tool
            .execute(json!({
                "manifest": manifest(),
                "enabled": true,
                "trusted": false,
                "confirmed": true
            }))
            .await
            .unwrap();
        assert!(matches!(
            installed.status,
            crate::agent::ToolResultStatus::Success
        ));
        let packages = db.list_plugin_packages().unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].source, "fixture://docs-helper");
        assert_eq!(packages[0].risk, "safe");
        assert_eq!(packages[0].permissions, json!(["read"]));
        let events = db
            .list_plugin_capability_events(Some("docs-helper"), 10)
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "install");
        assert_eq!(events[0].status, "ok");
    }

    #[tokio::test]
    async fn enabled_skill_capability_registers_as_real_tool_and_is_audited() {
        let db = temp_db();
        let install = InstallPluginPackageTool::new(db.clone());
        install
            .execute(json!({
                "manifest": manifest(),
                "enabled": true,
                "confirmed": true
            }))
            .await
            .unwrap();

        let mut registry = ToolRegistry::new();
        register_installed_plugin_capabilities(&mut registry, db.clone());
        let tool_name = plugin_tool_name("docs-helper", "review-checklist");
        assert!(
            registry.metadata_for(&tool_name).is_some(),
            "enabled skill capability is registered as a callable tool"
        );
        let result = registry
            .execute(&crate::agent::ToolCall {
                id: "call-1".to_string(),
                name: tool_name,
                arguments: json!({}),
            })
            .await
            .unwrap();
        assert!(matches!(
            result.status,
            crate::agent::ToolResultStatus::Success
        ));
        assert!(result.data["content"]
            .as_str()
            .unwrap()
            .contains("Check scope"));
        let events = db
            .list_plugin_capability_events(Some("docs-helper"), 10)
            .unwrap();
        assert!(events
            .iter()
            .any(|event| event.action == "invoke" && event.status == "ok"));
    }

    #[tokio::test]
    async fn disabled_package_blocks_invocation_and_unregisters_dynamic_tool() {
        let db = temp_db();
        InstallPluginPackageTool::new(db.clone())
            .execute(json!({
                "manifest": manifest(),
                "enabled": true,
                "confirmed": true
            }))
            .await
            .unwrap();
        SetPluginPackageEnabledTool::new(db.clone())
            .execute(json!({
                "pluginId": "docs-helper",
                "enabled": false,
                "reason": "test disable",
                "confirmed": true
            }))
            .await
            .unwrap();

        let mut registry = ToolRegistry::new();
        register_installed_plugin_capabilities(&mut registry, db.clone());
        assert!(registry
            .metadata_for(&plugin_tool_name("docs-helper", "review-checklist"))
            .is_none());
        let result = InvokePluginCapabilityTool::new(db.clone())
            .execute(json!({
                "pluginId": "docs-helper",
                "capabilityId": "review-checklist"
            }))
            .await
            .unwrap();
        assert!(matches!(
            result.status,
            crate::agent::ToolResultStatus::Error
        ));
    }

    #[tokio::test]
    async fn mcp_capability_is_advertised_when_eval_gate_passes() {
        let db = temp_db();
        let mcp_manifest = json!({
            "id": "mcp-pack",
            "name": "MCP Pack",
            "source": "fixture://mcp",
            "eval": { "commands": [{ "command": "cargo test --lib mcp" }] },
            "capabilities": [{
                "id": "search",
                "kind": "mcp",
                "name": "Search",
                "risk": "sensitive",
                "permissions": ["network"],
                "serverId": "srv",
                "toolName": "search"
            }]
        });
        InstallPluginPackageTool::new(db.clone())
            .execute(json!({
                "manifest": mcp_manifest,
                "enabled": true,
                "trusted": true,
                "confirmed": true
            }))
            .await
            .unwrap();
        assert!(
            db.get_plugin_package("mcp-pack").unwrap().enabled,
            "quality gate allows executable MCP adapter after eval evidence is declared"
        );
        let mut registry = ToolRegistry::new();
        register_installed_plugin_capabilities(&mut registry, db.clone());
        assert!(
            registry
                .metadata_for(&plugin_tool_name("mcp-pack", "search"))
                .is_some(),
            "MCP capability is registered through the executable adapter boundary"
        );
    }

    fn temp_db() -> LocalDb {
        let path =
            std::env::temp_dir().join(format!("aura_plugin_test_{}.db", uuid::Uuid::new_v4()));
        LocalDb::open(path).unwrap()
    }
}
