# Architecture Map

## Runtime

`src-tauri/src/agent/core.rs` coordinates the model loop, tool calls, tool results, checkpoints, verification, and final audit behavior.

## Model Adapters

`src-tauri/src/agent/openai.rs`, `anthropic.rs`, `llm_client.rs`, `model_routing.rs`, and related files define provider boundaries, model selection, streaming/non-streaming parsing, usage accounting, and fallback behavior.

## Tools

`src-tauri/src/tools/*` contains the local tool system: file read/write/edit, command safety, search, web, MCP, browser automation, checkpoints, verification, and policy enforcement.

## Commands / Bridge

`src-tauri/src/commands/*` exposes selected backend capabilities to the desktop frontend through Tauri commands. `src/lib/invoke-bridge.ts` is the TypeScript bridge boundary.

## Storage

`src-tauri/src/storage/mod.rs` defines local persistence tables, migrations, sessions, projects, artifacts, activity, permissions, usage, and related records.

## Context And Memory

`long_term_memory.rs`, `working_memory.rs`, `token_budget.rs`, and the agent command context-window helpers describe how conversation context, memory, and token windows are represented.

## Security And Policy

`policy.rs`, `command_safety.rs`, `fs_scope.rs`, `secret_scan.rs`, and permission-related command code are the main safety boundaries.