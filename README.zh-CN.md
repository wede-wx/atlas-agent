# Atlas

Atlas 是一个本地优先的桌面 AI 代理(Rust + [Tauri](https://v2.tauri.app/)),围绕一个目标构建:**一个不会"假装完成"的代理。**

代理最常见的失败方式不是"做不到"——而是*悄悄漂移*:它悄悄缩小任务范围、把函数打桩、注释掉没通过的测试、改了被要求保留的文件,然后汇报"成功"。靠问模型"你诚实吗?"解决不了这个问题——做出判断的正是那个已经漂移的器官。

Atlas 把**声明**和**执行**分开。一个请求会被冻结成**目标合同**(必须做什么、不能做什么、要保留什么、范围边界、每一项如何验证),然后一组闸门在每次工具调用**执行前**,把这个动作和合同做机械比对——而不是让模型给自己打分。

> 本仓库以架构为主,附带一个**最小化的可运行前端基线**。它不是一个完整的产品,也**不是所有的闸门都已接入运行时**(见下文)。**尤其欢迎做前端的贡献者。**

[English → README.md](./README.md)

## 它是怎么工作的

Harness 有四道闸。我们明确区分"今天运行时里已接通的"和"代码里有但还没接进主循环的":

| 闸门 | 作用 | 状态 |
|---|---|---|
| **ContractGate** | 在工具动作执行**前**,把它和目标合同做结构比对(保留路径、禁止模式、范围边界)。 | ✅ 已接通 |
| **ImpactEvidenceGate** | 记录读取/搜索证据,在高风险的越界修改前要求提供证据或发起披露。 | ✅ 已接通 |
| **Verifier** | 任务允许标记为完成前,进行一次独立的、只读的对抗式审查。 | ⏳ 代码已有,未接通 |
| **CompletionGate** | `done` 必须绑定真实的验证产物,而不是模型的自我评估。 | ⏳ 代码已有,未接通 |

同样存在于代码中但尚未接入运行时:`atlas-verifier` / `team_runtime` 审查集成,以及目标合同的持久化存储。

**Atlas Harness 是纵深防御层,不是证明。** 它提高了目标悄悄漂移的成本,并在高风险动作前强制留下证据或发起披露,但它不能保证代理永远不会漂移。诚实的边界说明见 [SECURITY.md](./SECURITY.md) 和 [docs/REVIEW_FINDINGS.md](./docs/REVIEW_FINDINGS.md)。

## 快速开始

前置条件:稳定版 **Rust**、**Node.js 18+**,以及你操作系统对应的 [Tauri v2 环境要求](https://v2.tauri.app/start/prerequisites/)。

```bash
npm install        # 安装前端依赖

npm run dev        # 终端 A — Vite 开发服务器 (http://localhost:1420)
cd src-tauri && cargo tauri dev   # 终端 B — Tauri 外壳
```

本地模型(ollama、LM Studio)开箱可用——CSP 已放行 `localhost` / `127.0.0.1`。

检查和正式构建:

```bash
npm run typecheck
cargo test
npm run build
```

## 仓库地图

```
src/                          最小化前端基线(TypeScript + Vite)
  bridge.ts                   对 Tauri 命令的类型化封装 — UI 通过这里调后端
  types.ts                    从 Rust 侧镜像过来的类型
  app.ts                      无框架的聊天 UI 控制器
  main.ts / styles.css        入口 + 座舱风格主题
src-tauri/src/
  agent/                      代理运行时和模型/工具主循环
  agent/atlas_harness/        目标合同 + 各道闸(contract_gate、impact_evidence、
                              completion_gate、verifier、goal_contract、path_match、glue)
  commands/                   Tauri 命令面
  storage/                    本地优先的会话/消息存储
scripts/                      辅助脚本(如浏览器自动化)
docs/
  ARCHITECTURE.md             系统架构概览
  COMMAND_BRIDGE.md           前端 <-> 后端命令 + 事件契约
  REVIEW_FINDINGS.md          已修复的问题 + 待跟进项
ATLAS_CODE_REVIEW_COMMAND.md  项目自身的代码审查清单(用 Atlas 审查 Atlas)
```

## 参与贡献 — 前端是最大的空缺

目前最大的贡献空间是**前端**。`src/` 里的基线是一个真实可接的起点,不是最终产品。你可以扩展它、重新设计风格,或用你熟悉的框架替换;只要保留 `bridge.ts`(或等效文件)作为调用后端的唯一入口就行。

从这两个文档开始:

- [CONTRIBUTING.md](./CONTRIBUTING.md)
- [docs/COMMAND_BRIDGE.md](./docs/COMMAND_BRIDGE.md) — UI 对接后端所需的一切(核心命令 + `agent-event` 事件流)。

有一条铁律:**不准假按钮。** 一个控件如果没有真实调用某个后端命令,就不能看起来像能用的。这是一个关于"不假装完成"的项目,UI 也应该守同一条线。

## 文档

- [README.md](./README.md)(英文)
- [CONTRIBUTING.md](./CONTRIBUTING.md)
- [SECURITY.md](./SECURITY.md)
- [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md)
- [docs/COMMAND_BRIDGE.md](./docs/COMMAND_BRIDGE.md)
- [docs/REVIEW_FINDINGS.md](./docs/REVIEW_FINDINGS.md)

## 许可证

[Apache-2.0](./LICENSE)。注意:Apache-2.0 是允许商用的。如果你想限制商用,需要换一个不同的许可证(比如 AGPL-3.0——保持开源,但要求修改版/网络服务版也必须开放源码;或者非商用的源码可见许可证——后者不属于"开源"范畴)。
