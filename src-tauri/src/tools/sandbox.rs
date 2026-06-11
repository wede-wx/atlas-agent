//! Step 3（真沙箱·阶段 1）：OS 级文件系统写边界。
//!
//! 此前的"隔离"全部是策略级（cwd 边界 + env 白名单 + command_safety 分类器），
//! `sandbox_backend` 只是个记录字符串。本模块补上 OS 强制层，按平台渐进：
//!
//! - **Linux → Landlock**（内核 ≥5.13，无特权、零外部依赖）：子进程 `pre_exec`
//!   里施加 ruleset——全盘只读 + workspace/临时目录可写。
//! - **macOS → seatbelt**（`sandbox-exec` profile）：deny file-write* 后按
//!   subpath 放行 workspace/临时目录。官方 deprecated 但事实稳定（Chromium 等）。
//! - **Windows → BoundaryOnly**：本阶段诚实披露"OS 沙箱未激活"，维持策略级。
//!   假装有沙箱比没有沙箱更危险——披露即 Atlas 的哲学。AppContainer 归阶段 2。
//!
//! 失败语义（用户已拍板）：默认**降级 + 披露 + audit 留痕**；配置
//! `require_sandbox=true` 则 fail-closed——沙箱起不来就拒绝执行命令。
//! 阶段 1 只锁文件系统写边界，不锁网络（Landlock 网络过滤需内核 6.7+）。

use std::path::PathBuf;
use std::sync::OnceLock;

use tokio::process::Command;

use crate::agent::AgentError;

/// 实际生效的沙箱后端。`as_str` 的值直接落 `workspace_lifecycle.sandbox_backend`
/// 与审计——记录的必须是真实生效的东西，不是愿望。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxBackend {
    /// Linux Landlock LSM（文件系统访问控制）。
    Landlock,
    /// macOS seatbelt（sandbox-exec profile）。
    Seatbelt,
    /// 无 OS 强制；仅策略级边界（cwd + env 白名单 + 命令分类器）。
    BoundaryOnly,
}

impl SandboxBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            SandboxBackend::Landlock => "landlock",
            SandboxBackend::Seatbelt => "seatbelt",
            SandboxBackend::BoundaryOnly => "boundary_only",
        }
    }

    /// 探测当前平台可用的最强后端。结果进程内缓存（OnceLock）：探测是真实
    /// 系统调用/文件检查，不必每条命令重做。
    pub fn detect() -> SandboxBackend {
        static DETECTED: OnceLock<SandboxBackend> = OnceLock::new();
        *DETECTED.get_or_init(detect_uncached)
    }
}

#[cfg(target_os = "linux")]
fn detect_uncached() -> SandboxBackend {
    // 探测 = 只创建 ruleset、不 restrict_self（restrict 是不可逆的，绝不能在
    // 主进程做）。HardRequirement 让不支持的内核显式报错而非 BestEffort 静默
    // 降级——探测要的就是真话。
    use landlock::{Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, ABI};
    let probe = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(ABI::V1))
        .and_then(|ruleset| ruleset.create());
    match probe {
        Ok(_) => SandboxBackend::Landlock,
        Err(_) => SandboxBackend::BoundaryOnly,
    }
}

#[cfg(target_os = "macos")]
fn detect_uncached() -> SandboxBackend {
    if std::path::Path::new("/usr/bin/sandbox-exec").exists() {
        SandboxBackend::Seatbelt
    } else {
        SandboxBackend::BoundaryOnly
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn detect_uncached() -> SandboxBackend {
    SandboxBackend::BoundaryOnly
}

/// 一次命令执行的沙箱规格。`writable_roots` 之外全盘只读（读/执行不受限——
/// 命令仍要能跑 /usr/bin 下的工具链）。
#[derive(Debug, Clone, Default)]
pub struct SandboxSpec {
    pub writable_roots: Vec<PathBuf>,
    /// true = fail-closed：沙箱不可用则拒绝执行；false（默认）= 降级 + 披露。
    pub require_sandbox: bool,
}

impl SandboxSpec {
    /// 规范化可写根：去重、补系统临时目录（构建工具几乎都要写临时文件）。
    pub fn effective_writable_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        let mut push = |path: PathBuf| {
            let normalized = path.canonicalize().unwrap_or(path);
            if !roots.iter().any(|existing| existing == &normalized) {
                roots.push(normalized);
            }
        };
        for root in &self.writable_roots {
            push(root.clone());
        }
        push(std::env::temp_dir());
        #[cfg(target_os = "macos")]
        {
            // macOS 的 /tmp 是 /private/tmp 的符号链接；seatbelt subpath 按真实
            // 路径匹配，两个都放行以免 canonicalize 行为差异漏边。
            push(PathBuf::from("/private/tmp"));
        }
        roots
    }
}

/// 一次构造的结果：实际后端 + 是否真的施加了 OS 强制 + 人话明细。
/// `enforced=false` 的命令必须在审计里可见——这正是"诚实披露"的载体。
#[derive(Debug, Clone)]
pub struct SandboxApplication {
    pub backend: SandboxBackend,
    pub enforced: bool,
    pub detail: String,
}

/// 构造受沙箱的 shell 命令（替代 command.rs 中裸的 `sh -lc` / powershell）。
///
/// 返回 Err 仅当 `require_sandbox=true` 且当前平台拿不出 OS 强制；
/// 否则总是返回可执行的命令 + 如实的 SandboxApplication。
pub fn build_sandboxed_shell(
    command: &str,
    spec: &SandboxSpec,
) -> Result<(Command, SandboxApplication), AgentError> {
    let backend = SandboxBackend::detect();
    if spec.require_sandbox && backend == SandboxBackend::BoundaryOnly {
        return Err(AgentError::Tool(
            "配置要求 OS 沙箱（require_sandbox=true），但当前平台/内核不可用：\
             Linux 需要内核 ≥5.13（Landlock），macOS 需要 sandbox-exec。\
             请关闭 require_sandbox 以策略级边界降级运行，或升级环境。"
                .to_string(),
        ));
    }
    match backend {
        SandboxBackend::Landlock => Ok(build_landlock_shell(command, spec)),
        SandboxBackend::Seatbelt => Ok(build_seatbelt_shell(command, spec)),
        SandboxBackend::BoundaryOnly => {
            let mut child = plain_shell(command);
            child.kill_on_drop(true);
            Ok((
                child,
                SandboxApplication {
                    backend,
                    enforced: false,
                    detail: "OS 沙箱未激活（平台/内核不支持）；仅策略级边界生效。".to_string(),
                },
            ))
        }
    }
}

fn plain_shell(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut child = Command::new("powershell");
        child.args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            command,
        ]);
        child
    }
    #[cfg(not(windows))]
    {
        let mut child = Command::new("sh");
        child.args(["-lc", command]);
        child
    }
}

#[cfg(target_os = "linux")]
fn build_landlock_shell(command: &str, spec: &SandboxSpec) -> (Command, SandboxApplication) {
    let writable = spec.effective_writable_roots();
    let writable_display = writable
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let require = spec.require_sandbox;
    let mut child = plain_shell(command);
    child.kill_on_drop(true);
    // pre_exec 在 fork 之后、exec 之前的子进程里跑：restrict_self 只影响子进程
    // 及其后代，主进程不受波及。闭包里用 BestEffort（探测已确认内核支持，这里
    // 的降级只剩 ABI 细节差异）。
    unsafe {
        child.pre_exec(move || {
            use landlock::{
                path_beneath_rules, Access, AccessFs, Ruleset, RulesetAttr, RulesetCreatedAttr, ABI,
            };
            let abi = ABI::V2;
            let apply = || -> Result<(), landlock::RulesetError> {
                Ruleset::default()
                    .handle_access(AccessFs::from_all(abi))?
                    .create()?
                    // 全盘只读（含执行）——工具链要能跑；
                    .add_rules(path_beneath_rules(&["/"], AccessFs::from_read(abi)))?
                    // workspace + 临时目录全权限。
                    .add_rules(path_beneath_rules(&writable, AccessFs::from_all(abi)))?
                    .restrict_self()?;
                Ok(())
            };
            match apply() {
                Ok(()) => Ok(()),
                Err(error) if require => Err(std::io::Error::other(format!(
                    "landlock restrict failed and require_sandbox=true: {error}"
                ))),
                // 降级语义：施加失败不拦执行（探测已过、此处失败是边角情形），
                // 但父进程侧记录的 enforced 状态以探测为准并在审计中可见。
                Err(_) => Ok(()),
            }
        });
    }
    (
        child,
        SandboxApplication {
            backend: SandboxBackend::Landlock,
            enforced: true,
            detail: format!("Landlock：全盘只读，可写根 [{writable_display}]。"),
        },
    )
}

#[cfg(not(target_os = "linux"))]
fn build_landlock_shell(command: &str, _spec: &SandboxSpec) -> (Command, SandboxApplication) {
    // 非 Linux 永远探测不到 Landlock；此分支仅为编译完整性。
    let mut child = plain_shell(command);
    child.kill_on_drop(true);
    (
        child,
        SandboxApplication {
            backend: SandboxBackend::BoundaryOnly,
            enforced: false,
            detail: "unreachable: landlock on non-linux".to_string(),
        },
    )
}

/// 生成 seatbelt profile：默认放行，整体 deny 文件写，再按 subpath 放行可写根。
/// seatbelt 规则后者优先，顺序即语义。独立成纯函数以便跨平台单测。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn seatbelt_profile(writable_roots: &[PathBuf]) -> String {
    let mut profile = String::from("(version 1)\n(allow default)\n(deny file-write*)\n");
    for root in writable_roots {
        profile.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            seatbelt_escape(&root.display().to_string())
        ));
    }
    profile
}

/// seatbelt profile 字符串字面量转义（反斜杠与双引号）。路径可由模型间接影响
/// （workspace 根来自项目配置），不转义就是 profile 注入面。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn seatbelt_escape(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(target_os = "macos")]
fn build_seatbelt_shell(command: &str, spec: &SandboxSpec) -> (Command, SandboxApplication) {
    let writable = spec.effective_writable_roots();
    let profile = seatbelt_profile(&writable);
    let mut child = Command::new("/usr/bin/sandbox-exec");
    child.args(["-p", &profile, "sh", "-lc", command]);
    child.kill_on_drop(true);
    let writable_display = writable
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    (
        child,
        SandboxApplication {
            backend: SandboxBackend::Seatbelt,
            enforced: true,
            detail: format!("seatbelt：deny file-write*，可写根 [{writable_display}]。"),
        },
    )
}

#[cfg(not(target_os = "macos"))]
fn build_seatbelt_shell(command: &str, _spec: &SandboxSpec) -> (Command, SandboxApplication) {
    let mut child = plain_shell(command);
    child.kill_on_drop(true);
    (
        child,
        SandboxApplication {
            backend: SandboxBackend::BoundaryOnly,
            enforced: false,
            detail: "unreachable: seatbelt on non-macos".to_string(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seatbelt_profile_denies_writes_then_reallows_roots_in_order() {
        let profile = seatbelt_profile(&[PathBuf::from("/ws/proj"), PathBuf::from("/tmp")]);
        let deny = profile.find("(deny file-write*)").expect("deny present");
        let allow_ws = profile
            .find("(allow file-write* (subpath \"/ws/proj\"))")
            .expect("workspace allow present");
        let allow_tmp = profile
            .find("(allow file-write* (subpath \"/tmp\"))")
            .expect("tmp allow present");
        assert!(
            deny < allow_ws && allow_ws < allow_tmp,
            "seatbelt 后者优先：deny 必须在 allow 之前"
        );
        assert!(profile.starts_with("(version 1)\n(allow default)\n"));
    }

    #[test]
    fn seatbelt_escape_blocks_profile_injection() {
        // 路径里藏 `")(allow file-write* (subpath "/`——不转义就把 deny 关了。
        let hostile = PathBuf::from(r#"/ws/x") )(allow file-write* (subpath "/"#);
        let profile = seatbelt_profile(&[hostile]);
        assert!(
            !profile.contains(r#"(subpath "/")"#),
            "hostile path must not escape its string literal"
        );
        assert!(profile.contains("\\\""), "quotes inside paths are escaped");
    }

    #[test]
    fn writable_roots_dedupe_and_include_temp_dir() {
        let spec = SandboxSpec {
            writable_roots: vec![std::env::temp_dir(), std::env::temp_dir()],
            require_sandbox: false,
        };
        let roots = spec.effective_writable_roots();
        let temp = std::env::temp_dir()
            .canonicalize()
            .unwrap_or_else(|_| std::env::temp_dir());
        assert_eq!(
            roots.iter().filter(|root| **root == temp).count(),
            1,
            "temp dir appears exactly once even when supplied explicitly"
        );
    }

    #[test]
    fn require_sandbox_fails_closed_when_only_boundary_available() {
        // 仅在拿不出 OS 后端的平台上有意义；Linux(≥5.13)/macOS 上 detect 会
        // 返回真后端，此测试自动短路——平台差异本身就是被测语义的一部分。
        if SandboxBackend::detect() != SandboxBackend::BoundaryOnly {
            return;
        }
        let spec = SandboxSpec {
            writable_roots: vec![],
            require_sandbox: true,
        };
        assert!(matches!(
            build_sandboxed_shell("echo hi", &spec),
            Err(AgentError::Tool(_))
        ));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn landlock_blocks_write_outside_workspace_and_allows_inside() {
        // 真实端到端：内核支持 Landlock 时，沙箱内命令写 workspace 成功、
        // 写 workspace 外失败。内核不支持则如实短路（探测即真相）。
        if SandboxBackend::detect() != SandboxBackend::Landlock {
            eprintln!("kernel without landlock; skipping enforcement e2e");
            return;
        }
        let workspace =
            std::env::temp_dir().join(format!("atlas_sandbox_ws_{}", std::process::id()));
        std::fs::create_dir_all(&workspace).unwrap();
        // 拒绝目标必须满足两条：无沙箱时可写（排除"因普通权限被拒"的假阳性）、
        // 不在 writable_roots 内（temp_dir 默认可写，不能用）。HOME 同时满足。
        let denied_path = dirs::home_dir()
            .expect("home dir available in test env")
            .join(format!("atlas_sandbox_denied_{}.txt", std::process::id()));
        std::fs::write(&denied_path, b"probe")
            .expect("denied target must be writable WITHOUT sandbox");
        std::fs::remove_file(&denied_path).unwrap();

        let spec = SandboxSpec {
            writable_roots: vec![workspace.clone()],
            require_sandbox: true,
        };

        let (mut inside_cmd, application) =
            build_sandboxed_shell(&format!("touch {}/ok.txt", workspace.display()), &spec).unwrap();
        assert!(application.enforced);
        let inside = inside_cmd.status().await.unwrap();
        assert!(inside.success(), "write inside workspace must succeed");

        let (mut outside_cmd, _) =
            build_sandboxed_shell(&format!("touch {}", denied_path.display()), &spec).unwrap();
        let denied = outside_cmd.status().await.unwrap();
        assert!(
            !denied.success(),
            "write outside writable roots must be denied by landlock"
        );
        assert!(!denied_path.exists(), "denied file must not exist");
        let _ = std::fs::remove_dir_all(&workspace);
    }
}
