# Atlas

Atlas 是一个 local-first 桌面 AI agent，使用 Rust 和 Tauri 构建。

这个仓库现在的定位不是纯 architecture-only 导出，而是：

```text
architecture + minimal runnable frontend baseline
```

也就是说：仓库保留核心 agent 架构，并提供一个可以最小运行的前端基线。当前前端只覆盖真实 chat loop，不是完整产品 UI。我们欢迎 UI 贡献者在这个真实后端边界上继续建设。

[English README](./README.md)

## 当前状态

当前已经接入真实运行链路：

- `ContractGate`：工具执行前，把 proposed action 与当前 Goal Contract 做结构化比对。
- `ImpactEvidenceGate`：记录 read/search 证据，并要求高风险越界动作留下证据或披露。
- `src/` 下的极简前端基线。
- sessions、messages、chat、`agent-event` stream 的 command bridge。
- `hooks.rs` 输出处理硬化，覆盖高输出命令和 UTF-8 tail。

代码中已经存在但尚未接入真实运行链路：

- `CompletionGate`
- `Verifier`
- `atlas-verifier` / `team_runtime` reviewer 集成
- Goal Contract 持久化

Atlas Harness 是纵深防御层。它可以降低静默目标漂移的概率，并强制高风险动作留下证据或披露，但它不是“agent 永远不会漂移”的证明。

## 快速开始

前置要求：

- 稳定版 Rust
- Node.js 18+
- 当前系统对应的 Tauri v2 依赖

安装依赖：

```bash
npm install
```

启动前端 dev server：

```bash
npm run dev
```

另开一个终端启动 Tauri shell：

```bash
cd src-tauri
cargo tauri dev
```

常用检查：

```bash
npm run typecheck
cargo test
```

前端构建：

```bash
npm run build
```

## 目录概览

```text
src/
  bridge.ts                  前端调用后端 command 的唯一入口
  app.ts                     极简聊天 UI 控制器
  types.ts                   前端侧 command/event 类型
src-tauri/src/
  agent/                     agent runtime 和模型/工具循环
  agent/atlas_harness/       Goal Contract gates 和 harness 模块
  commands/                  Tauri command surface
  storage/                   本地 session/message 存储
docs/
  COMMAND_BRIDGE.md          当前前端/后端命令桥接说明
  REVIEW_FINDINGS.md         已修复问题和后续项
```

## UI 贡献方向

当前最大贡献方向是 UI。`src/` 里的前端是接通真实后端的起点，不是最终产品体验。

请先阅读：

- [CONTRIBUTING.md](./CONTRIBUTING.md)
- [docs/COMMAND_BRIDGE.md](./docs/COMMAND_BRIDGE.md)

硬性原则：不要做假按钮。没有接真实 command 的控件，不能伪装成可用功能。

## 文档

- [README.md](./README.md)
- [CONTRIBUTING.md](./CONTRIBUTING.md)
- [SECURITY.md](./SECURITY.md)
- [docs/COMMAND_BRIDGE.md](./docs/COMMAND_BRIDGE.md)
- [docs/REVIEW_FINDINGS.md](./docs/REVIEW_FINDINGS.md)

## 许可证

Apache-2.0。见 [LICENSE](./LICENSE)。
