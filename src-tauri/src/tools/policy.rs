use crate::tools::{ToolCapability, ToolMetadata, ToolSafetyLevel};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPermissionMode {
    Plan,
    Default,
    FullAccess,
}

impl AgentPermissionMode {
    pub fn normalize(raw: Option<&str>) -> Self {
        match raw.map(str::trim).filter(|value| !value.is_empty()) {
            Some("plan") | Some("safe") | Some("suggest") => Self::Plan,
            Some("full_access") | Some("full") | Some("full_auto") => Self::FullAccess,
            Some("default") | Some("workspace") | Some("auto_edit") => Self::Default,
            _ => Self::Default,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Default => "default",
            Self::FullAccess => "full_access",
        }
    }

    pub fn label_zh(&self) -> &'static str {
        match self {
            Self::Plan => "计划模式",
            Self::Default => "默认模式",
            Self::FullAccess => "完全访问模式",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyAction {
    Read,
    Write,
    Command,
    Delete,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyRisk {
    Safe,
    Sensitive,
    Destructive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    RequireApproval { reason: String },
    Deny { reason: String },
}

impl PolicyDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    pub fn is_visible_without_runtime_approval(&self) -> bool {
        matches!(self, Self::Allow)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Allow => None,
            Self::RequireApproval { reason } | Self::Deny { reason } => Some(reason),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PolicyEngine {
    mode: AgentPermissionMode,
}

impl PolicyEngine {
    pub fn new(mode: AgentPermissionMode) -> Self {
        Self { mode }
    }

    pub fn mode(&self) -> &AgentPermissionMode {
        &self.mode
    }

    pub fn evaluate(&self, action: PolicyAction, risk: PolicyRisk) -> PolicyDecision {
        if risk == PolicyRisk::Destructive {
            return match self.mode {
                AgentPermissionMode::Plan => PolicyDecision::Deny {
                    reason: "计划模式不会执行破坏性操作。".to_string(),
                },
                AgentPermissionMode::Default => PolicyDecision::Deny {
                    reason: "默认模式不会执行破坏性操作。".to_string(),
                },
                // 行为变更：完全访问模式不再逐动作审批，破坏性动作直接放行。
                // 被移除的只是“权限层审批”这一道；以下边界全部保留：
                //   1) Atlas 目标保真闸（ContractGate 等四道）照常拦截；
                //   2) command_safety 的 Denied 级硬拒绝（rm -rf /、强制 push 等）
                //      在任何模式下仍然拒绝；
                //   3) 子代理角色收紧（SubAgentRole::restrict）不受会话模式影响。
                AgentPermissionMode::FullAccess => PolicyDecision::Allow,
            };
        }

        match self.mode {
            AgentPermissionMode::Plan => match action {
                PolicyAction::Read => PolicyDecision::Allow,
                PolicyAction::Write => PolicyDecision::Deny {
                    reason: "当前是计划模式，只能读取和整理方案，不会修改文件。".to_string(),
                },
                PolicyAction::Command => PolicyDecision::Deny {
                    reason: "当前是计划模式，只能读取和整理方案，不会运行命令。".to_string(),
                },
                PolicyAction::Delete | PolicyAction::System => PolicyDecision::Deny {
                    reason: "当前是计划模式，不会执行会改变系统或数据的操作。".to_string(),
                },
            },
            AgentPermissionMode::Default => match action {
                PolicyAction::Read | PolicyAction::Write => PolicyDecision::Allow,
                PolicyAction::Command => PolicyDecision::RequireApproval {
                    reason: "默认模式运行命令前需要你确认。".to_string(),
                },
                PolicyAction::Delete => PolicyDecision::RequireApproval {
                    reason: "删除或覆盖类操作需要你确认。".to_string(),
                },
                PolicyAction::System => PolicyDecision::Deny {
                    reason: "默认模式不会执行系统级操作。".to_string(),
                },
            },
            // 行为变更：完全访问模式对所有动作类型直接放行（含 Delete/System），
            // 不再要求逐动作审批。目标保真与命令安全的硬性边界另行保留。
            AgentPermissionMode::FullAccess => PolicyDecision::Allow,
        }
    }

    pub fn evaluate_tool_execution(&self, metadata: &ToolMetadata) -> PolicyDecision {
        if is_plan_tasks_tool_name(&metadata.name) {
            return PolicyDecision::Allow;
        }
        if metadata.name == "prepare_command" {
            return match self.mode {
                AgentPermissionMode::Default => PolicyDecision::Allow,
                AgentPermissionMode::Plan => PolicyDecision::Deny {
                    reason: "计划模式不会准备或运行命令。".to_string(),
                },
                AgentPermissionMode::FullAccess => PolicyDecision::Deny {
                    reason: "完全访问模式下普通命令应直接执行，不需要命令预览工具。".to_string(),
                },
            };
        }
        if metadata.name == "stop_run" {
            return match self.mode {
                AgentPermissionMode::Plan => PolicyDecision::Deny {
                    reason: "计划模式没有正在执行的任务可停止。".to_string(),
                },
                AgentPermissionMode::Default | AgentPermissionMode::FullAccess => {
                    PolicyDecision::Allow
                }
            };
        }
        if self.mode == AgentPermissionMode::Plan && is_network_tool(metadata) {
            return PolicyDecision::Deny {
                reason: "计划模式不会主动联网。".to_string(),
            };
        }
        self.evaluate(infer_tool_action(metadata), infer_tool_risk(metadata))
    }

    pub fn evaluate_tool_visibility(&self, metadata: &ToolMetadata) -> PolicyDecision {
        if is_plan_tasks_tool_name(&metadata.name) {
            return PolicyDecision::Allow;
        }
        if metadata.name == "stop_run" {
            return match self.mode {
                AgentPermissionMode::Plan => PolicyDecision::Deny {
                    reason: "计划模式不需要停止任务工具。".to_string(),
                },
                AgentPermissionMode::Default | AgentPermissionMode::FullAccess => {
                    PolicyDecision::Allow
                }
            };
        }
        let action = infer_tool_action(metadata);
        let risk = infer_tool_risk(metadata);

        match self.mode {
            AgentPermissionMode::Plan => {
                if is_network_tool(metadata) {
                    return PolicyDecision::Deny {
                        reason: "计划模式不会主动联网。".to_string(),
                    };
                }
                self.evaluate(action, risk)
            }
            AgentPermissionMode::Default => {
                if metadata.name == "prepare_command" {
                    return PolicyDecision::Allow;
                }
                if metadata.name == "run_command" {
                    return PolicyDecision::Deny {
                        reason: "默认模式运行命令前需要确认卡片。".to_string(),
                    };
                }
                self.evaluate(action, risk)
            }
            AgentPermissionMode::FullAccess => {
                if metadata.name == "prepare_command" {
                    return PolicyDecision::Deny {
                        reason: "完全访问模式会直接运行普通命令，不需要命令预览工具。".to_string(),
                    };
                }
                self.evaluate(action, risk)
            }
        }
    }
}

pub fn infer_tool_action(metadata: &ToolMetadata) -> PolicyAction {
    let name = metadata.name.as_str();
    if name.contains("delete") || name.contains("remove") {
        return PolicyAction::Delete;
    }
    if matches!(name, "prepare_command" | "run_command") {
        return PolicyAction::Command;
    }
    if metadata.capabilities.contains(&ToolCapability::System) {
        return PolicyAction::System;
    }
    if metadata.mutates_state {
        return PolicyAction::Write;
    }
    PolicyAction::Read
}

pub fn infer_tool_risk(metadata: &ToolMetadata) -> PolicyRisk {
    match metadata.safety_level {
        ToolSafetyLevel::Safe => PolicyRisk::Safe,
        ToolSafetyLevel::Sensitive => PolicyRisk::Sensitive,
        ToolSafetyLevel::Destructive => PolicyRisk::Destructive,
    }
}

fn is_network_tool(metadata: &ToolMetadata) -> bool {
    metadata.capabilities.contains(&ToolCapability::Network)
}

fn is_plan_tasks_tool_name(name: &str) -> bool {
    matches!(
        name,
        "create_plan"
            | "create_plan_task"
            | "update_plan_task"
            | "list_plan_tasks"
            | "set_active_plan_task"
    )
}

/// P3-2: a subagent runs as a constrained *role*, not an unlimited worker.
/// The role is an extra gate that *tightens* (never loosens) the mode-based
/// tool decision, so a reviewer/planner cannot mutate state even inside a
/// full-access session. The main agent reviews subagent output before
/// integrating it; the subagent itself only gets the tools its role needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubAgentRole {
    Planner,
    Reviewer,
    Executor,
    Tester,
    Researcher,
}

impl SubAgentRole {
    /// Infer a role from the subagent profile name/description. Least-privilege
    /// default: an unrecognized subagent is treated as a read-only reviewer.
    pub fn classify(name: &str, description: &str) -> Self {
        let haystack = format!("{name} {description}").to_ascii_lowercase();
        let has = |needles: &[&str]| needles.iter().any(|needle| haystack.contains(needle));
        if has(&[
            "review", "审查", "审阅", "复核", "审计", "audit", "critic", "lint",
        ]) {
            Self::Reviewer
        } else if has(&[
            "plan",
            "规划",
            "计划",
            "architect",
            "架构",
            "design",
            "设计",
        ]) {
            Self::Planner
        } else if has(&["test", "测试", "qa", "verify", "验证", "coverage"]) {
            Self::Tester
        } else if has(&[
            "research", "调研", "搜索", "search", "explore", "探索", "调查", "情报",
        ]) {
            Self::Researcher
        } else if has(&[
            "exec",
            "执行",
            "implement",
            "实现",
            "build",
            "coder",
            "编码",
            "开发",
            "fix",
            "修复",
            "refactor",
            "重构",
        ]) {
            Self::Executor
        } else {
            // Unknown subagent: least privilege = read-only.
            Self::Reviewer
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planner => "planner",
            Self::Reviewer => "reviewer",
            Self::Executor => "executor",
            Self::Tester => "tester",
            Self::Researcher => "researcher",
        }
    }

    pub fn label_zh(&self) -> &'static str {
        match self {
            Self::Planner => "规划子代理（只读）",
            Self::Reviewer => "审查子代理（只读）",
            Self::Executor => "执行子代理（按会话模式）",
            Self::Tester => "测试子代理（命令需确认）",
            Self::Researcher => "调研子代理（联网需确认）",
        }
    }

    /// Read-only roles cannot mutate state at all (file write / command / delete).
    pub fn is_read_only(self) -> bool {
        matches!(self, Self::Planner | Self::Reviewer)
    }

    /// Tighten a base policy decision for this role + tool. Never loosens:
    /// `Allow` may become `RequireApproval`/`Deny`; `RequireApproval` may become
    /// `Deny`; `Deny` always stays `Deny`.
    pub fn restrict(self, base: PolicyDecision, metadata: &ToolMetadata) -> PolicyDecision {
        match self {
            // Executor inherits the session mode unchanged.
            Self::Executor => base,
            Self::Planner | Self::Reviewer => {
                // Planning artifacts (the planner's whole job) and pure reads stay
                // allowed; anything that mutates files / runs commands / deletes is
                // denied regardless of session mode.
                if is_plan_tasks_tool_name(&metadata.name)
                    || infer_tool_action(metadata) == PolicyAction::Read
                {
                    base
                } else {
                    PolicyDecision::Deny {
                        reason: format!(
                            "{}：只读角色不能写入文件、运行命令或删除数据，只能读取并产出结论交主 Agent 整合。",
                            self.label_zh()
                        ),
                    }
                }
            }
            Self::Tester => {
                if infer_tool_action(metadata) == PolicyAction::Command {
                    tighten_to_approval(base, "测试子代理运行命令前需要确认，不会自动执行命令。")
                } else {
                    base
                }
            }
            Self::Researcher => {
                if is_network_tool(metadata) {
                    tighten_to_approval(base, "调研子代理联网前需要确认，不会自动发起网络请求。")
                } else {
                    base
                }
            }
        }
    }
}

/// Tighten a decision to at least `RequireApproval`; an existing `Deny` stays denied.
fn tighten_to_approval(base: PolicyDecision, reason: &str) -> PolicyDecision {
    match base {
        PolicyDecision::Deny { .. } => base,
        _ => PolicyDecision::RequireApproval {
            reason: reason.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(
        name: &str,
        capabilities: Vec<ToolCapability>,
        safety_level: ToolSafetyLevel,
        mutates_state: bool,
    ) -> ToolMetadata {
        ToolMetadata {
            name: name.to_string(),
            description: name.to_string(),
            label_zh: name.to_string(),
            description_zh: name.to_string(),
            capability_labels_zh: vec![],
            safety_label_zh: "安全".to_string(),
            capabilities,
            safety_level,
            mutates_state,
            requires_confirmation: false,
        }
    }

    #[test]
    fn normalizes_old_permission_aliases() {
        assert_eq!(
            AgentPermissionMode::normalize(Some("safe")).as_str(),
            "plan"
        );
        assert_eq!(
            AgentPermissionMode::normalize(Some("workspace")).as_str(),
            "default"
        );
        assert_eq!(
            AgentPermissionMode::normalize(Some("full")).as_str(),
            "full_access"
        );
        assert_eq!(AgentPermissionMode::normalize(None).as_str(), "default");
    }

    #[test]
    fn plan_mode_allows_read_but_denies_write_and_command() {
        let engine = PolicyEngine::new(AgentPermissionMode::Plan);
        assert!(engine
            .evaluate(PolicyAction::Read, PolicyRisk::Safe)
            .is_allowed());
        assert!(matches!(
            engine.evaluate(PolicyAction::Write, PolicyRisk::Sensitive),
            PolicyDecision::Deny { .. }
        ));
        assert!(matches!(
            engine.evaluate(PolicyAction::Command, PolicyRisk::Sensitive),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn default_mode_hides_direct_command_but_keeps_command_preview() {
        let engine = PolicyEngine::new(AgentPermissionMode::Default);
        let prepare = metadata(
            "prepare_command",
            vec![ToolCapability::System],
            ToolSafetyLevel::Sensitive,
            false,
        );
        let run = metadata(
            "run_command",
            vec![ToolCapability::System],
            ToolSafetyLevel::Sensitive,
            true,
        );

        assert!(engine.evaluate_tool_visibility(&prepare).is_allowed());
        assert!(matches!(
            engine.evaluate_tool_visibility(&run),
            PolicyDecision::Deny { .. }
        ));
        assert!(matches!(
            engine.evaluate_tool_execution(&run),
            PolicyDecision::RequireApproval { .. }
        ));
    }

    #[test]
    fn full_access_allows_everything_without_approval() {
        // 行为变更：完全访问模式不再逐动作审批。命令安全网关的 Denied 级
        // 硬拒绝与 Atlas 目标保真闸在工具内部 / 分发点另行兜底。
        let engine = PolicyEngine::new(AgentPermissionMode::FullAccess);
        for action in [
            PolicyAction::Read,
            PolicyAction::Write,
            PolicyAction::Command,
            PolicyAction::Delete,
            PolicyAction::System,
        ] {
            for risk in [
                PolicyRisk::Safe,
                PolicyRisk::Sensitive,
                PolicyRisk::Destructive,
            ] {
                assert!(
                    engine.evaluate(action, risk).is_allowed(),
                    "full_access should allow {action:?}/{risk:?} without approval"
                );
            }
        }
    }

    #[test]
    fn full_access_keeps_subagent_role_tightening() {
        // 完全访问会话里，只读子代理依然不能写——角色收紧与模式放行正交。
        let engine = PolicyEngine::new(AgentPermissionMode::FullAccess);
        let base = engine.evaluate_tool_execution(&write_tool());
        assert!(base.is_allowed());
        assert!(matches!(
            SubAgentRole::Reviewer.restrict(base, &write_tool()),
            PolicyDecision::Deny { .. }
        ));
    }

    fn write_tool() -> ToolMetadata {
        metadata(
            "write_file",
            vec![ToolCapability::Filesystem],
            ToolSafetyLevel::Sensitive,
            true,
        )
    }

    fn read_tool() -> ToolMetadata {
        metadata(
            "read_file",
            vec![ToolCapability::ReadOnly],
            ToolSafetyLevel::Safe,
            false,
        )
    }

    #[test]
    fn subagent_role_classifies_from_name_and_description() {
        assert_eq!(
            SubAgentRole::classify("code-reviewer", "reviews code"),
            SubAgentRole::Reviewer
        );
        assert_eq!(
            SubAgentRole::classify("审查子代理", ""),
            SubAgentRole::Reviewer
        );
        assert_eq!(
            SubAgentRole::classify("planner", "makes a plan"),
            SubAgentRole::Planner
        );
        assert_eq!(SubAgentRole::classify("架构师", ""), SubAgentRole::Planner);
        assert_eq!(
            SubAgentRole::classify("tester", "writes tests"),
            SubAgentRole::Tester
        );
        assert_eq!(
            SubAgentRole::classify("researcher", "searches the web"),
            SubAgentRole::Researcher
        );
        assert_eq!(
            SubAgentRole::classify("executor", "实现功能"),
            SubAgentRole::Executor
        );
        // Unknown subagent → least privilege (read-only reviewer).
        assert_eq!(
            SubAgentRole::classify("mystery", "does things"),
            SubAgentRole::Reviewer
        );
    }

    #[test]
    fn read_only_roles_deny_writes_but_keep_reads_and_plans() {
        let write = write_tool();
        let read = read_tool();
        let plan = metadata("create_plan", vec![], ToolSafetyLevel::Safe, true);
        for role in [SubAgentRole::Reviewer, SubAgentRole::Planner] {
            // Write denied even though the session mode would allow it.
            assert!(matches!(
                role.restrict(PolicyDecision::Allow, &write),
                PolicyDecision::Deny { .. }
            ));
            // Pure reads stay allowed.
            assert!(role.restrict(PolicyDecision::Allow, &read).is_allowed());
            // Planning artifacts stay allowed (the planner's whole job).
            assert!(role.restrict(PolicyDecision::Allow, &plan).is_allowed());
        }
    }

    #[test]
    fn executor_role_inherits_session_decision_unchanged() {
        let write = write_tool();
        assert!(SubAgentRole::Executor
            .restrict(PolicyDecision::Allow, &write)
            .is_allowed());
        assert!(matches!(
            SubAgentRole::Executor.restrict(
                PolicyDecision::Deny {
                    reason: "mode".to_string()
                },
                &write,
            ),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn tester_role_requires_approval_for_commands_only() {
        let run = metadata(
            "run_command",
            vec![ToolCapability::System],
            ToolSafetyLevel::Sensitive,
            true,
        );
        assert!(matches!(
            SubAgentRole::Tester.restrict(PolicyDecision::Allow, &run),
            PolicyDecision::RequireApproval { .. }
        ));
        // A non-command write (e.g. writing a test file) is unaffected.
        assert!(SubAgentRole::Tester
            .restrict(PolicyDecision::Allow, &write_tool())
            .is_allowed());
    }

    #[test]
    fn researcher_role_requires_approval_for_network_only() {
        let net = metadata(
            "fetch_web_page",
            vec![ToolCapability::Network],
            ToolSafetyLevel::Sensitive,
            false,
        );
        assert!(matches!(
            SubAgentRole::Researcher.restrict(PolicyDecision::Allow, &net),
            PolicyDecision::RequireApproval { .. }
        ));
        assert!(SubAgentRole::Researcher
            .restrict(PolicyDecision::Allow, &read_tool())
            .is_allowed());
    }

    #[test]
    fn role_restrict_never_loosens_a_denied_decision() {
        // A denied base stays denied even for a role that would otherwise allow reads.
        let denied = PolicyDecision::Deny {
            reason: "plan mode".to_string(),
        };
        assert!(matches!(
            SubAgentRole::Reviewer.restrict(denied, &read_tool()),
            PolicyDecision::Deny { .. }
        ));
    }
}
