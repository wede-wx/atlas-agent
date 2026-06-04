use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::storage::{LocalDb, LogPluginCapabilityEventPayload, PluginPackageRecord};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginQualityGateRequest {
    pub plugin_id: String,
    #[serde(default)]
    pub dev_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginQualityGate {
    pub plugin_id: String,
    pub status: String,
    pub can_enable: bool,
    pub risk: String,
    pub model_tier_hint: String,
    pub required_eval: bool,
    pub reasons: Vec<String>,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginEvalRegistryEntry {
    pub plugin_id: String,
    pub version: String,
    pub required: bool,
    pub suite_id: Option<String>,
    pub commands: Vec<String>,
    pub status: String,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillVersionRecord {
    pub plugin_id: String,
    pub skill_id: String,
    pub version: String,
    pub source: String,
    pub risk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamPresetRolePermission {
    pub role: String,
    pub allowed_capabilities: Vec<String>,
    pub denied_permissions: Vec<String>,
    pub risk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamPresetPermissionReport {
    pub plugin_id: String,
    pub can_bind: bool,
    pub roles: Vec<TeamPresetRolePermission>,
    pub reasons: Vec<String>,
}

pub fn evaluate_installed_plugin_quality_gate(
    db: &LocalDb,
    request: PluginQualityGateRequest,
) -> Result<PluginQualityGate, String> {
    let package = db
        .list_plugin_packages()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|package| package.id == request.plugin_id)
        .ok_or_else(|| format!("plugin package not found: {}", request.plugin_id))?;
    let gate = evaluate_plugin_quality_gate(&package, request.dev_mode);
    let _ = db.log_plugin_capability_event(LogPluginCapabilityEventPayload {
        plugin_id: package.id,
        capability_id: "*".to_string(),
        action: "quality_gate".to_string(),
        status: gate.status.clone(),
        risk: gate.risk.clone(),
        reason: gate.reasons.join("; "),
        input: json!({ "devMode": request.dev_mode }),
        output: serde_json::to_value(&gate).unwrap_or(Value::Null),
    });
    Ok(gate)
}

pub fn plugin_eval_registry_entry(package: &PluginPackageRecord) -> PluginEvalRegistryEntry {
    let risk = normalize_risk(&package.risk);
    let capabilities = package.capabilities.as_array().cloned().unwrap_or_default();
    let required = package
        .manifest
        .get("eval")
        .and_then(|value| value.get("required"))
        .and_then(Value::as_bool)
        .unwrap_or(risk != "safe" || capabilities.iter().any(is_adapter_pending_kind));
    let suite_id = package
        .manifest
        .get("eval")
        .and_then(|value| value.get("suiteId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let commands = package
        .manifest
        .get("eval")
        .and_then(|value| value.get("commands"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .take(20)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut reasons = Vec::new();
    if required && suite_id.is_none() && commands.is_empty() {
        reasons.push("required eval suite or command is missing".to_string());
    }
    if commands.iter().any(|command| command.len() > 500) {
        reasons.push("eval command is too long".to_string());
    }
    let status = if reasons.is_empty() {
        "registered"
    } else {
        "missing_required_eval"
    }
    .to_string();
    PluginEvalRegistryEntry {
        plugin_id: package.id.clone(),
        version: package.version.clone(),
        required,
        suite_id,
        commands,
        status,
        reasons,
    }
}

pub fn skill_version_registry(package: &PluginPackageRecord) -> Vec<SkillVersionRecord> {
    package
        .capabilities
        .as_array()
        .into_iter()
        .flatten()
        .filter(|capability| {
            matches!(
                capability
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
                "skill" | "instruction"
            )
        })
        .map(|capability| SkillVersionRecord {
            plugin_id: package.id.clone(),
            skill_id: capability
                .get("id")
                .or_else(|| capability.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("unnamed-skill")
                .to_string(),
            version: capability
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or(&package.version)
                .to_string(),
            source: package.source.clone(),
            risk: normalize_risk(
                capability
                    .get("risk")
                    .and_then(Value::as_str)
                    .unwrap_or(&package.risk),
            ),
        })
        .collect()
}

pub fn team_preset_permission_report(package: &PluginPackageRecord) -> TeamPresetPermissionReport {
    let capabilities = package.capabilities.as_array().cloned().unwrap_or_default();
    let roles = package
        .manifest
        .get("teamPreset")
        .and_then(|value| value.get("roles"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![json!({ "role": "main" })]);
    let mut reasons = Vec::new();
    let mut role_permissions = Vec::new();
    for role in roles {
        let role_name = role
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("reviewer")
            .to_string();
        let allowed = role
            .get("capabilities")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| {
                capabilities
                    .iter()
                    .filter_map(|capability| capability.get("id").cloned())
                    .collect()
            })
            .into_iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect::<Vec<_>>();
        let denied = denied_permissions_for_role(&role_name, &capabilities, &allowed);
        if !denied.is_empty() {
            reasons.push(format!(
                "{role_name} cannot bind denied permissions: {}",
                denied.join(",")
            ));
        }
        role_permissions.push(TeamPresetRolePermission {
            role: role_name,
            allowed_capabilities: allowed,
            denied_permissions: denied,
            risk: normalize_risk(&package.risk),
        });
    }
    TeamPresetPermissionReport {
        plugin_id: package.id.clone(),
        can_bind: reasons.is_empty(),
        roles: role_permissions,
        reasons,
    }
}

pub fn evaluate_plugin_quality_gate(
    package: &PluginPackageRecord,
    dev_mode: bool,
) -> PluginQualityGate {
    let mut reasons = Vec::new();
    let risk = normalize_risk(&package.risk);
    let capabilities = package.capabilities.as_array().cloned().unwrap_or_default();
    if capabilities.is_empty() {
        reasons.push("manifest has no capabilities".to_string());
    }
    let model_tier_hint = package
        .manifest
        .get("modelTierHint")
        .and_then(Value::as_str)
        .map(normalize_model_tier)
        .unwrap_or_else(|| infer_model_tier(&risk, &capabilities));
    let required_eval = package
        .manifest
        .get("eval")
        .and_then(|value| value.get("required"))
        .and_then(Value::as_bool)
        .unwrap_or(risk != "safe" || capabilities.iter().any(is_adapter_pending_kind));
    let has_eval = package
        .manifest
        .get("eval")
        .map(|value| {
            value
                .get("commands")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty())
                || value
                    .get("suiteId")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
        })
        .unwrap_or(false);
    if required_eval && !has_eval {
        reasons.push("required eval is missing".to_string());
    }
    if !package.trusted && risk != "safe" && !dev_mode {
        reasons.push("non-safe plugin from untrusted source".to_string());
    }
    for capability in &capabilities {
        let kind = capability
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let permissions = capability
            .get("permissions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if is_adapter_pending_kind(capability) && permissions.is_empty() {
            reasons.push(format!("{kind} capability missing explicit permissions"));
        }
        if normalize_risk(
            capability
                .get("risk")
                .and_then(Value::as_str)
                .unwrap_or("sensitive"),
        ) == "destructive"
            && !has_eval
        {
            reasons.push(format!("{kind} destructive capability has no eval"));
        }
    }
    reasons.sort();
    reasons.dedup();
    let can_enable = reasons.is_empty() || (dev_mode && reasons.iter().all(|r| r.contains("eval")));
    PluginQualityGate {
        plugin_id: package.id.clone(),
        status: if can_enable {
            "passed".to_string()
        } else if dev_mode {
            "needs_review".to_string()
        } else {
            "blocked".to_string()
        },
        can_enable,
        risk,
        model_tier_hint,
        required_eval,
        reasons,
        checked_at: chrono::Utc::now().timestamp_millis(),
    }
}

fn denied_permissions_for_role(
    role: &str,
    capabilities: &[Value],
    allowed_capabilities: &[String],
) -> Vec<String> {
    let mut denied = Vec::new();
    for capability in capabilities {
        let id = capability
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if !allowed_capabilities.iter().any(|allowed| allowed == id) {
            continue;
        }
        let risk = normalize_risk(
            capability
                .get("risk")
                .and_then(Value::as_str)
                .unwrap_or("sensitive"),
        );
        let permissions = capability
            .get("permissions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if role != "main" && risk == "destructive" {
            denied.push(format!("{id}:destructive"));
        }
        if matches!(role, "verifier" | "reviewer" | "tester")
            && permissions.iter().any(|permission| permission == "write")
        {
            denied.push(format!("{id}:write"));
        }
    }
    denied.sort();
    denied.dedup();
    denied
}

fn is_adapter_pending_kind(capability: &Value) -> bool {
    !matches!(
        capability
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        "skill" | "instruction"
    )
}

fn normalize_risk(risk: &str) -> String {
    match risk.trim().to_ascii_lowercase().as_str() {
        "safe" | "sensitive" | "destructive" => risk.trim().to_ascii_lowercase(),
        _ => "sensitive".to_string(),
    }
}

fn normalize_model_tier(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "cheap" | "standard" | "strong" | "vision" => value.trim().to_ascii_lowercase(),
        _ => "standard".to_string(),
    }
}

fn infer_model_tier(risk: &str, capabilities: &[Value]) -> String {
    if capabilities.iter().any(|capability| {
        capability
            .get("permissions")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|item| item == "network"))
    }) {
        "strong".to_string()
    } else if risk == "safe" {
        "cheap".to_string()
    } else {
        "standard".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(
        capabilities: Value,
        risk: &str,
        trusted: bool,
        manifest_extra: Value,
    ) -> PluginPackageRecord {
        let mut manifest = json!({ "capabilities": capabilities });
        if let Some(object) = manifest.as_object_mut() {
            if let Some(extra) = manifest_extra.as_object() {
                for (key, value) in extra {
                    object.insert(key.clone(), value.clone());
                }
            }
        }
        PluginPackageRecord {
            id: "plugin-a".to_string(),
            name: "Plugin A".to_string(),
            version: "1.0.0".to_string(),
            source: "local".to_string(),
            description: "".to_string(),
            trusted,
            enabled: false,
            risk: risk.to_string(),
            permissions: json!([]),
            capabilities: manifest["capabilities"].clone(),
            manifest,
            installed_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn safe_skill_package_passes_without_eval() {
        let gate = evaluate_plugin_quality_gate(
            &package(
                json!([{ "id": "skill", "kind": "skill", "risk": "safe", "permissions": ["read"] }]),
                "safe",
                false,
                json!({}),
            ),
            false,
        );
        assert!(gate.can_enable);
        assert_eq!(gate.model_tier_hint, "cheap");
    }

    #[test]
    fn untrusted_sensitive_adapter_without_eval_is_blocked() {
        let gate = evaluate_plugin_quality_gate(
            &package(
                json!([{ "id": "mcp", "kind": "mcp", "risk": "sensitive", "permissions": ["network"] }]),
                "sensitive",
                false,
                json!({}),
            ),
            false,
        );
        assert!(!gate.can_enable);
        assert_eq!(gate.status, "blocked");
        assert!(gate.reasons.iter().any(|reason| reason.contains("eval")));
    }

    #[test]
    fn eval_registry_requires_suite_for_sensitive_plugin() {
        let entry = plugin_eval_registry_entry(&package(
            json!([{ "id": "mcp", "kind": "mcp", "risk": "sensitive", "permissions": ["network"] }]),
            "sensitive",
            true,
            json!({}),
        ));
        assert_eq!(entry.status, "missing_required_eval");
        assert!(entry.required);
    }

    #[test]
    fn skill_registry_and_team_preset_permissions_are_derived_from_manifest() {
        let package = package(
            json!([
                { "id": "safe-skill", "kind": "skill", "risk": "safe", "permissions": ["read"], "version": "1.2.0" },
                { "id": "write-tool", "kind": "mcp", "risk": "destructive", "permissions": ["write"] }
            ]),
            "destructive",
            true,
            json!({
                "eval": { "required": true, "suiteId": "plugin-smoke" },
                "teamPreset": {
                    "roles": [
                        { "role": "verifier", "capabilities": ["write-tool"] },
                        { "role": "main", "capabilities": ["write-tool"] }
                    ]
                }
            }),
        );
        let skills = skill_version_registry(&package);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].version, "1.2.0");
        let report = team_preset_permission_report(&package);
        assert!(!report.can_bind);
        assert!(report
            .roles
            .iter()
            .any(|role| role.role == "verifier" && !role.denied_permissions.is_empty()));
        let eval = plugin_eval_registry_entry(&package);
        assert_eq!(eval.status, "registered");
    }
}
