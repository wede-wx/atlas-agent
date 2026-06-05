# Atlas Agent Architecture Preview

This repository is an architecture-only preview extracted from the Atlas local agent project.

It is not the full desktop product. The current frontend, local runtime data, build output, logs, caches, user sessions, private configuration, and API keys are intentionally excluded.

## Included

- Agent runtime and model adapter code
- Tool registry and tool execution boundaries
- Tauri command bridge code for agent/storage/config/web paths
- Local storage schema and migration code
- Permission, policy, context, memory, model routing, verification, and observability architecture
- Minimal compile-time compatibility inputs required by the Rust architecture modules

## Excluded

- `.env`, local config, API keys, tokens, passwords, and credentials
- SQLite databases and user/session data
- Logs, screenshots, Playwright output, build output, target directories, caches, and dependency folders
- Current product UI implementation and design mockups
- Full desktop verification scripts, island probes, browser smoke tests, local runtime data, and product UI
- External reference-source checkouts

## Status

Architecture preview only. Some files are copied from a larger application and may reference modules that are not included here. Treat this as source material for reviewing and open-sourcing the agent architecture, not as a ready-to-run product.

## Running and Verification Notes

This repository is an architecture-only preview. Full desktop verification scripts, island probes, browser smoke tests, local runtime data, and product UI are intentionally excluded.

For this preview, prefer compile/static review of the included runtime modules. The frontend bridge boundary is documented in `docs/ARCHITECTURE.md`; the full TypeScript application shell is not included.
