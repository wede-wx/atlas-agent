# Atlas Agent Architecture Preview

This repository is an architecture-only preview extracted from the Atlas/Aura local agent project.

It is not the full desktop product. The current frontend, local runtime data, build output, logs, caches, user sessions, private configuration, and API keys are intentionally excluded.

## Included

- Agent runtime and model adapter code
- Tool registry and tool execution boundaries
- Tauri command bridge code for agent/storage/config/web paths
- Local storage schema and migration code
- Permission, policy, context, memory, model routing, verification, and observability architecture
- Minimal TypeScript bridge/state files needed to understand frontend-to-backend boundaries

## Excluded

- `.env`, local config, API keys, tokens, passwords, and credentials
- SQLite databases and user/session data
- Logs, screenshots, Playwright output, build output, target directories, caches, and dependency folders
- Current product UI implementation and design mockups
- External reference-source checkouts

## Status

Architecture preview only. Some files are copied from a larger application and may reference modules that are not included here. Treat this as source material for reviewing and open-sourcing the agent architecture, not as a ready-to-run product.