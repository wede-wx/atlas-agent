# Atlas Agent 事项记录

# Atlas Agent 工作区约定

- 当前默认工作区：`C:\Users\Administrator\Desktop\atlas-agent`。
- 暂不处理旧 Aura 工作区：`C:\Users\Administrator\Desktop\Aura-Codex-Transfer-CLEAN-2026-05-05`，除非用户明确要求。
- 后续用户提供新的架构或前端代码时：直接照搬代码；照搬后运行验证；只定位并汇报错误、冲突和未通过项；有问题先问/汇报，不擅自修改。
- 汇报时区分：已照搬、验证通过、验证失败、未验证；不要把复制代码当成目标完成。

## 2026-06-10 Atlas skills installation

- Installed Codex-compatible skills from C:\Users\Administrator\Desktop\atlas-skill into C:\Users\Administrator\.codex\skills:
  - tlas-contract
  - tlas-ledger
- Compatibility checks performed: SKILL.md exists, YAML frontmatter exists, 
ame: matches destination folder, installed content hash matches source.
- Existing installed skill folders were backed up under C:\Users\Administrator\.codex\skill-backups before replacement.
- Restart Codex to pick up the updated skills.


## 2026-06-10 Atlas skill Codex compatibility frontmatter fix

- User reported tlas-contract still did not appear after Codex restart.
- Compatibility fix applied: shortened YAML description: frontmatter for installed and source copies of tlas-contract and tlas-ledger; retained full skill bodies (Atlas Contract v6.1, Atlas Ledger v2.1).
- Reason: long frontmatter descriptions can be less reliable for Codex skill indexing; body length is preserved because it carries the important protocol.
- Restart Codex again to force skill re-indexing.


## 2026-06-11 React frontend implementation

**User goal**: Replace the framework-free TS shell with a React + TypeScript frontend for `atlas-agent`, keep `src/bridge.ts` as the single backend bridge, use real Tauri commands, follow `D:\Atlas.html` visual direction, and avoid fake data/interfaces.

**Actual changes**:
- Added React/Vite dependencies and scripts in `package.json`: `tauri:dev`, `tauri:build`; generated `package-lock.json` via `npm install`.
- Updated `vite.config.ts` for React and `src-tauri/tauri.conf.json` with `beforeDevCommand` / `beforeBuildCommand`.
- Replaced frontend entry with `src/main.tsx`, `src/App.tsx`, React Context state, feature components, and CSS-variable theme styling.
- Kept `src/bridge.ts` as the only Tauri API bridge and added typed wrappers for sessions, projects, messages, chat, run timeline/evidence, settings/app-state, MCP, knowledge, plugins, evals, memory, and data export/reset.
- Implemented main shell: custom title bar/window controls, left nav, project/session list, chat stream, composer, `agent_chat_v2`, `agent-event`, tool cards, Blocked/Unverified gate card, pause/resume/cancel.
- Implemented run drawer tabs: contract/audit evidence, run timeline, graph/evidence feeds, permission decisions with real `resolve_permission_confirmation` where records exist.
- Implemented settings page: model config/check/list, MCP list/test/trust, global Agent rules, UI preferences through `get_app_state/set_app_state`, data health/export/reset-app-state-only.
- Implemented knowledge page using `search_knowledge({ query: "" })` as first-pass listing with an explicit incomplete-list notice; add/delete use real commands.
- Implemented plugins page with real package list/enable toggle/capability events.
- Implemented defense/eval page with real eval suites and verifier run command.
- Implemented search overlay using real `search_sessions` and `search_knowledge`.

**UI app-state keys chosen**:
- `ui.theme = { mode: "dark" | "light" | "system" }`
- `ui.notifications = { runCompleted, blockedGate, permissionNeeded, sound }`
- `ui.general = { defaultAgentMode, autoCreateSession, openDrawerOnRun }`
- `ui.layout = { sidebarCollapsed, rightDrawerOpen, rightDrawerTab }`

**Validation results**:
- `npm install`: completed; npm reported 2 moderate vulnerabilities. No `npm audit fix --force` was run because it may introduce breaking upgrades.
- `npm run typecheck`: passed after frontend TS fixes.
- `npm run build`: passed; Vite production bundle generated.
- `npm run tauri:dev`: attempted, but timed out after 60s with no captured output; dev-window path is therefore not fully verified through that command.
- `npm run tauri:build`: passed; Rust release build and MSI/NSIS bundles completed.
- Built executable launched: `src-tauri\target\release\atlas.exe`, process id `13568`.

**Not verified / risks**:
- Real chat send path was not manually exercised inside the opened app window by this agent.
- Page navigation was not manually clicked in the opened app window by this agent.
- `search_knowledge` empty query may not be a complete list; page discloses this.
- Some settings fields are generic app-state or config projections, not specialized backend settings commands.
- No Rust command signatures were changed.

**Honest status**: React frontend implementation and build-level/native-bundle validation completed; real interactive smoke inside the launched window remains unverified.

## 2026-06-11 Static visual replication note: iframe is temporary

- Current static visual replication uses `iframe` / `srcdoc` for secondary pages (`knowledge`, `plugins`, `defense`, `settings`) to faithfully preserve `D:\Atlas.html` template styles and isolate `__embed_hide` from the main shell.
- This is explicitly a temporary solution for the static visual-only phase.
- When entering the backend integration phase, these secondary pages must be rewritten as normal React components using the same Atlas.html class names and DOM structure, but without iframe/srcdoc, so they can call `src/bridge.ts`, consume real backend data, share app state, and support real interactions.
- Do not treat the iframe/static template implementation as suitable for the real connected frontend phase.

## 2026-06-11 Backend integration B1: React component shell

**Phase goal**: prepare the static visual shell for real backend wiring while preserving the approved Atlas.html visual structure.

**Actual changes**:
- Added `src/AtlasMainMarkup.tsx`, generated from the approved `D:\Atlas.html` main UI DOM as JSX instead of rendering the main shell through `dangerouslySetInnerHTML`.
- Reworked `src/App.tsx` so the main shell and search overlay are React-rendered components. Search overlay now renders static React rows instead of injecting the search HTML string.
- Kept the current page-open / settings-mode behavior and the existing iframe/srcdoc secondary pages for the later page-specific rewrite phases; this is still temporary and not suitable for final backend integration.
- Preserved current `src/styles.css` from the Atlas.html-derived CSS.

**Validation**:
- `npm run typecheck`: passed.
- `npm run build`: passed.

**Not completed / risks**:
- B2 backend wiring has not started yet.
- Main shell is React JSX but still contains static sample content; real sessions/projects/messages/chat events are not connected yet.
- Secondary pages still use iframe/srcdoc and must be rewritten as normal React components in their backend integration phases.
- Visual runtime after JSX conversion has not been manually re-confirmed in the app window in this phase.

## 2026-06-11 B2 主聊天后端接线 + Aura 误写纠偏记录

- 目标阶段：B2，仅接主聊天真实路径；未进入 B3 右抽屉真实数据阶段。
- Aura 误写范围查实：相对路径 patch 曾误写 `C:\Users\Administrator\Desktop\Aura-Codex-Transfer-CLEAN-2026-05-05\src\App.tsx`，并新增 `C:\Users\Administrator\Desktop\Aura-Codex-Transfer-CLEAN-2026-05-05\src\features\chat\BackendChatLayer.tsx`；相对 `src\styles.css` patch 在 Aura 因文件不存在失败，未写入。
- Aura 纠偏：已删除误新增的 `src\features\chat\BackendChatLayer.tsx`，删除残留空目录 `src\features\chat`，并按用户批准从 Aura 仓库 `HEAD` 恢复 `src\App.tsx`。复查证据：误写路径 `git status --short -- src/App.tsx src/features/chat/BackendChatLayer.tsx src/features/chat` 无输出；`git diff --quiet -- src/App.tsx` 通过；`git hash-object src\App.tsx` 与 `git rev-parse HEAD:src/App.tsx` 一致。Aura 全仓库仍有大量既有 dirty 项，不属于本次误写纠偏范围。
- atlas-agent 写入约束：后续所有 atlas-agent 写入均改用绝对路径 `C:\Users\Administrator\Desktop\atlas-agent\...`，避免默认 CWD 再误写 Aura。
- atlas-agent B2 改动：新增 `src/features/chat/BackendChatLayer.tsx`，主聊天通过 `bridge.ts` 调用真实 `get_sessions`、`list_projects`、`get_messages`、`agent_chat_v2`、`agent-event`、`pause_agent_chat`、`resume_agent_chat`、`cancel_agent_chat`；`App.tsx` 改为保留 Atlas 静态壳并把导航/对话流/composer 三个区域交给后端接线层；`styles.css` 增加仅 B2 动态区域所需的隐藏/空态/错误态/mode 菜单/运行控制样式。
- mode 接线：composer 的“默认模式”标签现在可展开选择 `chat`/`plan`/`review`，发送时把所选 mode 传给 `agent_chat_v2`，不是固定 `chat`。
- 错误态：发送失败会显示可见错误卡；模型/连接/配置/密钥相关错误会提示“没有可用的模型连接，请先在设置里配置模型连接”，并带后端原始错误。
- 验证：`npm run typecheck` 通过；`npm run build` 通过，Vite production build 成功。
- 未验证：未启动 Tauri 窗口做人工点击；未真实发送消息验证模型调用；未验证暂停/恢复/取消在真实运行中的后端行为；B3 右抽屉真实契约/时间线/运行图仍未接入。

## 2026-06-11 B2 验收修复：设置返回与窗口控制

- 用户验收反馈：设置页无法返回主对话；自绘窗口按钮（最小化/最大化/关闭）无功能；需一并确认导航栏开关、主题切换、抽屉开关保持可用。
- 修复范围：仅修改 `src/App.tsx`。未进入 B3，未接设置模型连接表单，未改 Rust command。
- 设置页返回：由于二级页当前仍是静态阶段遗留的 `iframe/srcdoc`，iframe 内部点击不会冒泡给 React 父层；已在 `viewFrame` load 后安装同源 iframe click bridge。设置页如果缺少返回按钮，会在左侧导航候选容器顶部注入 `← 返回`；点击任意文本含“返回”的设置页按钮会调用父层 `setActivePage(null)`，退出 `settings-mode` 回到主对话界面。
- 窗口控制：新增 `@tauri-apps/api/window` 动态导入，优先使用 Tauri v2 `getCurrentWindow()`，兼容 `appWindow` 兜底；右上角自绘按钮按 `aria-label/title` 分别调用 `minimize()`、`toggleMaximize()`、`close()`。
- 已保留：顶栏左侧导航开关仍切 `nav-collapsed`；主题按钮仍切 `data-theme`；抽屉按钮仍切 `drawer-open`。
- 验证：`npm run typecheck` 通过；`npm run build` 通过。
- 未验证：未在真实 Tauri 窗口手点设置返回和窗口控制按钮；需要用户在已运行窗口里复验。下一步建议按用户要求先接设置页“模型连接”真实配置路径，再回到 B3。

## B4-P2 窗口控制修复记录（2026-06-11）

- 实际改动：自绘窗口按钮改为通过 `src/bridge.ts` 的 `minimizeWindow`、`toggleMaximizeWindow`、`closeWindow` 调用 Tauri window API，移除 `App.tsx` 内直接动态导入 `@tauri-apps/api/window` 的本地 helper。
- 实际改动：窗口按钮点击分支增加 `preventDefault()` 与 `stopPropagation()`，避免顶栏点击代理或拖拽区域干扰。
- 实际改动：`AtlasMainMarkup.tsx` 只给非交互的 `tb-session` 区域添加 `data-tauri-drag-region`，窗口按钮自身和父级 `win-ctrls/tb-right/topbar` 不作为拖拽区。
- 实际改动：新增 `src-tauri/capabilities/default.json`，授权 `core:window:allow-minimize`、`core:window:allow-maximize`、`core:window:allow-unmaximize`、`core:window:allow-toggle-maximize`、`core:window:allow-close`，并保留 `core:default`。
- 验证：`npm run typecheck` 通过。
- 验证：`npm run build` 通过。
- 未验证：尚未在真实 Tauri 窗口中手点最小化、最大化/还原、关闭；需要用户或后续真实窗口验证。
- 后续注意：B4-P3 仍需把设置页“模型”板块从 iframe/srcdoc 改写为真实 React 组件，并接 `get_config/save_config/check_model_settings/list_models`。

## B4-P2 用户实测收尾（2026-06-11）

- 用户在真实 Tauri 窗口中手点验证：最小化、最大化/还原、关闭按钮通过。
- B4-P2 状态：窗口控制修复经用户真实路径确认通过。

## B4-P3 设置页模型板块 React 化与后端接线（2026-06-11）

- 目标阶段：B4-P3，仅处理设置页「模型」板块从 iframe/srcdoc 迁出并接真实后端；未进入 B3，未迁移知识库/插件/防线测试，未接右抽屉真实数据。
- 实际改动：`src/App.tsx` 中 `activePage === "settings"` 不再写入 `viewFrame.srcdoc`，而是在主 React 树中渲染 `ModelSettingsPage`；其他二级页仍保留当前 iframe/srcdoc 临时方案。
- 实际改动：移除了设置页 iframe 注入返回按钮/iframe click bridge；设置页返回现在是 `ModelSettingsPage` 内真实 React 按钮，调用父层 `setActivePage(null)` 返回主对话界面。
- 实际改动：新增 `src/features/settings/ModelSettingsPage.tsx`，接入 `getConfig`、`listModels`、`checkModelSettings`、`saveConfig`，所有调用均通过 `src/bridge.ts` wrapper，没有在组件内直接调用 Tauri `invoke`。
- 实际改动：模型页从 `get_config` 读取已保存连接；无连接时显示真实未配置空态；模型列表只显示 `list_models` / `check_model_settings` 的真实返回，不填假模型。
- 实际改动：`save_config` 提交表单字段；API Key 留空时传 `null`，不把空字符串当新 key；`theme` 和 `sound_enabled` 仅从 `get_config` 现有值透传，若缺失则显示错误并不保存。
- 实际改动：`src/styles.css` 追加 `.settings-react-page` 作用域样式，保留 Atlas.html 设置页视觉语义，避免污染主聊天和其他 iframe 页面。
- 验证：`npm run typecheck` 通过。
- 验证：`npm run build` 通过。
- 未验证：尚未在真实 Tauri 窗口点击设置页返回、填写配置、调用 `list_models`、`check_model_settings` 或 `save_config`。
- 未验证：真实模型连通性需要用户提供有效 provider/API 地址/API key/model 后才能验证。
- 后续注意：其余六个设置板块仍未 React 化/后端接线；知识库/插件/防线测试仍是 iframe/srcdoc 临时方案，进入对应后端阶段时必须迁出为 React 组件。

## B4-P4 模型设置体验修复与删除连接 command（2026-06-11）

- 用户确认：B4-P3 已在窗口点验通过，模型能连上。
- 目标阶段：B4-P4，仅处理模型设置页体验修复和真实删除模型连接后端缺口；未进入 B3，未改其余六个设置板块。
- 前端改动：API Key 输入框增加右侧眼睛按钮，默认掩码，可切换明文/掩码。
- 前端改动：保存成功后才清空 API Key 输入框并回到“已保存密钥，留空保持不变”的占位语义；用户输入过程中不主动清空。
- 前端改动：连接参数主区域精简为连接名称、服务商、API 地址、API Key、模型；Provider ID、Route ID、协议、认证 Header 移入默认折叠的“高级选项”，字段仍参与提交，留空交给后端推导。
- 前端改动：设置页内容滚动条改为细样式，默认透明，悬停/聚焦时显示，减少常驻粗滚动条占位感。
- 前端改动：每张已保存模型连接卡增加删除按钮；删除前使用二次确认；删除调用 bridge 的真实后端命令，不做前端假删除。
- Bridge 改动：新增 `deleteModelConnection(connectionId)` wrapper，调用 Tauri command `delete_model_connection`，组件仍不直接调用 invoke。
- 后端改动：新增 `delete_model_connection(connection_id: String)` command，对 `config.llm.connections` 做真实 retain 删除；删除默认连接时切换默认连接；删除 provider 的最后一条连接时清理对应 legacy slot；无剩余连接时清空 legacy slots，避免保存时重新生成；保存 config 后更新内存 state 并返回 redacted config。
- 后端改动：`lib.rs` 的 `tauri::generate_handler!` 注册 `commands::delete_model_connection`。
- 验证：`npm run typecheck` 通过。
- 验证：`npm run build` 通过。
- 验证：`cargo build` 在 `src-tauri` 下通过。
- 未验证：尚未在真实 Tauri 窗口手点 API Key 显示切换、高级选项折叠/展开、删除二次确认、真实删除后配置刷新、保存后 API Key 输入框行为。
- 后续注意：删除 command 是真实持久化操作；手测建议先用可恢复的测试连接验证删除路径。

## B4-P4 白屏排查与 dev server 重启（2026-06-11）

- 现象：用户反馈 Tauri 窗口白屏；同一 `http://localhost:1420` 在 Chrome 截图中也白屏。
- 根因：运行中的 Vite dev server 缓存了旧版 `src/bridge.ts`，浏览器控制台报错：`The requested module '/src/bridge.ts' does not provide an export named 'deleteModelConnection'`。磁盘源码已包含该 export，属于 dev server/HMR 缓存未更新，不是源码缺少 wrapper。
- 处理：重启 atlas-agent dev 栈，停止旧 `node/vite/cargo/atlas.exe` 进程后重新启动 `npm run tauri:dev`。
- 验证：重启后 `http://localhost:1420/src/bridge.ts` 已包含 `deleteModelConnection`；Chrome headless 截图显示主界面恢复渲染，不再白屏。
- 当前残留真实错误：主界面显示 `订阅 agent-event 失败：Cannot read properties of undefined (reading 'transformCallback')`，这是后续需要单独处理的 Tauri event/浏览器环境兼容问题，不是白屏根因。

## B4-P5 按需查看已保存 API Key 明文（2026-06-11）

- 目标阶段：B4-P5，仅补模型设置页“用户显式点击眼睛查看已保存 API Key 明文”的真实后端能力；未改 `get_config` / `redacted_for_client` 默认脱敏行为，未进入 B3，未动其余六个设置板块。
- 后端改动：新增 Tauri command `reveal_model_connection_key(connection_id: String) -> Result<String, String>`，按 `connection_id` 查找 `config.llm.connections` 并返回对应连接的明文 `api_key`；找不到或空 id 返回错误；不修改配置、不保存文件、不打印密钥。
- 后端改动：`lib.rs` 的 `tauri::generate_handler!` 注册 `commands::reveal_model_connection_key`。
- Bridge 改动：新增 `revealModelConnectionKey(connectionId)` wrapper，调用 `reveal_model_connection_key`，组件仍不直接调用 invoke。
- 前端改动：API Key 眼睛按钮改为按需 reveal。若输入框已有用户新输入内容，只本地切换明文/掩码，不调用后端；若输入框为空且已有保存密钥，点击显示时调用 `revealModelConnectionKey` 临时展示明文。
- 前端安全语义：切回隐藏、切换连接、保存成功、删除连接时立即清空临时明文；`buildSavePayload`、`listModels`、`checkModelSettings` 仍只读取 `form.apiKey`，不会把 reveal 出来的旧密钥随其他 payload 上送，除非用户编辑输入框使其成为新输入值。
- 保留：保存成功后 Key 框清空、留空表示不变；配置列表默认不带明文；删除连接和模型连接保存路径不回退。
- 验证：`npm run typecheck` 通过。
- 验证：`npm run build` 通过。
- 验证：`cargo build` 在 `src-tauri` 下通过。
- 未验证：尚未重启真实 Tauri 窗口手点 reveal command；新增 Rust command 需要重启 Tauri 进程后才会在运行窗口中可用。
- 需用户手测：已保存 key 点眼睛是否显示明文；切回隐藏是否清掉；新输入 key 时眼睛是否仅本地切换；保存后是否仍清空输入框并保持“留空不变”。

## B3 右抽屉真实运行数据接线（2026-06-11）

- 目标阶段：B3，仅把右抽屉三个标签（目标契约 / 运行时间线 / 运行图）从静态占位改为真实 React 组件并接真实后端 run 数据；未改 B2 主聊天路径，未改 B4 模型设置，未新增后端命令。
- 当前 run 串联：`BackendChatLayer` 增加 `onActiveSessionChange`，把当前 `activeSessionId` 抬升到 `App`；`RunDrawer` 用 `get_agent_runs(sessionId, limit)` 读取当前会话最近 run，并取最新一条作为右抽屉当前 run。
- 可见右抽屉：新增 `src/features/runs/RunDrawer.tsx`，通过 portal 接管 `.drawer` 可见区域；旧 Atlas 静态 drawer DOM 仅隐藏，不再作为可见数据来源。
- 目标契约标签：接 `get_agent_permission_decisions(runId, limit)` 显示权限审批账本；接 `get_agent_run_audit(runId, limit, offset)` 显示审计流，保留 raw/detail 展示；pending/needs_confirm 记录提供批准/拒绝按钮并走 `resolve_permission_confirmation` bridge wrapper。
- 运行时间线标签：接 `get_agent_run_progress(runId)` 显示进度概览；接 `get_agent_run_timeline(runId, limit, offset)` 显示 entries，并按 `total` 提供“加载更多”；接 `get_agent_run_diff(runId, limit)` 与 `get_agent_run_terminal(runId, limit, offset)`，只展示后端返回内容，不补齐终端行。
- 运行图标签：新增 bridge wrapper `getAgentGraphSnapshot(graphRunId)` / `getAgentGraphNodeTraces(graphRunId)`，对应后端 `get_agent_graph_snapshot` / `get_agent_graph_node_traces`；仅当当前 run 记录中存在真实 `graph_run_id` / `graphRunId` 等字段时拉取图数据。
- graphRunId 来源结论：本轮只读搜索找到按 graphRunId 获取 snapshot/traces 的命令，但未找到按 session/source_run 列出 graph run 或从普通 run 自动换取 graphRunId 的已暴露命令；因此当前 run 没有 graphRunId 字段时，运行图显示真实空态，不用普通 run 数据伪造图。
- 空态：无 session、当前会话无 run、run 无 timeline/audit/permission/graph 数据时均显示真实空态，不填静态样例。
- Bridge/类型：`src/bridge.ts` 新增 graph 读取 wrapper；`src/types.ts` 增加 `RunAuditFeed`、`AgentGraphSnapshot`、`WorkflowTraceReport` 等宽松类型。
- 验证：`npm run typecheck` 通过。
- 验证：`npm run build` 通过，Vite production build 成功。
- 已发现并修复：首次验证暴露 `GraphTab` props 的 TS 窄化错误，已改为以 `currentRun` 做显式非空判断后重新验证通过。
- 未验证：尚未在真实 Tauri 窗口发送消息后手点右抽屉三个标签；尚未验证真实 run 是否包含 graphRunId；尚未验证真实 `needs_confirm` 记录的批准/拒绝在后端账本中的写入效果。
- 需用户手测：发一条会产生 run 的消息后打开右抽屉，检查最近 run 是否加载、时间线是否分页、契约账本是否显示、运行图在无 graphRunId 时是否真实空态。

## Backend hardening branch attempt (2026-06-11)

- User selected baseline commit + isolated branch flow.
- Baseline commit on `main`: `f40727f chore: snapshot atlas frontend baseline`.
- Created and switched to isolated branch: `backend-harden`.
- Replaced the 10 requested backend files from pasted attachments on `backend-harden` only:
  - `src-tauri/src/agent/atlas_harness/glue.rs`
  - `src-tauri/src/agent/atlas_harness/contract_gate.rs`
  - `src-tauri/src/agent/atlas_harness/path_match.rs`
  - `src-tauri/src/agent/atlas_harness/impact_evidence.rs`
  - `src-tauri/src/agent/atlas_harness/completion_gate.rs`
  - `src-tauri/src/tools/mcp.rs`
  - `src-tauri/src/tools/policy.rs`
  - `src-tauri/src/tools/command_safety.rs`
  - `src-tauri/src/tools/plan_tasks.rs`
  - `src-tauri/src/agent/core.rs`
- `src-tauri/src/agent/atlas_harness/mod.rs` already had `pub mod path_match;`, so no module declaration change was needed.
- Validation stopped at `cargo build`: failed before tests. `cargo test` was not run because the contract required stopping on compile failure.
- Primary build failure starts at `src/agent/core.rs:60-61`: `pub type PostCommandVerifyHook = Arc` followed by `dyn Fn(...)`, causing `expected one of ..., found keyword dyn`. Subsequent unresolved imports (`Agent`, `ContextBuilder`, `AgentUsageEvent`) appear downstream from `core.rs` failing to parse/export.
- No business/security logic in the pasted backend files was modified to make it compile.
- Honest status: isolated branch has pasted hardening files applied, but backend hardening is blocked by compile failure; main baseline remains available at commit `f40727f`.

## Backend hardening branch validation passed after syntax-level paste repair (2026-06-11)

- Branch: `backend-harden`.
- Baseline available on `main`: commit `f40727f chore: snapshot atlas frontend baseline`.
- User authorized strictly limited paste-damage repair after first `cargo build` failure.
- Only repaired confirmed transmission damage in `src-tauri/src/agent/core.rs`: restored missing generic angle brackets in `PostCommandVerifyHook` (`Arc<...>`, `Pin<...>`, `Box<...>`). No function body logic, harness gate logic, policy semantics, command safety semantics, or plan task semantics were changed.
- Re-ran `cargo build`: passed.
- Re-ran `cargo test`: passed. Summary: `616 passed; 0 failed; 2 ignored`; lib/main/doc-test harnesses with 0 tests also passed.
- Required new tests observed green:
  - `agent::atlas_harness::glue::tests::mcp_invocation_unwraps_nested_arguments_and_is_gated`
  - `agent::atlas_harness::contract_gate::tests::quoted_no_verify_is_still_a_hard_block`
  - `tools::plan_tasks::tests::explicit_verified_without_artifact_is_blocked`
  - `tools::policy::tests::full_access_allows_everything_without_approval`
  - `agent::atlas_harness::contract_gate::tests::hookspath_rewire_is_a_hard_block`
- Honest status: backend hardening replacement is validated on isolated branch but not merged to `main` yet. Branch still contains uncommitted hardening file changes unless explicitly committed/merged later.
