# Atlas

Atlas is a local-first desktop AI agent built with Rust and Tauri.

This repository is no longer a pure architecture export. It is an
architecture-focused repository with a minimal runnable frontend baseline. The
frontend is intentionally small: it proves the core chat loop can connect to the
backend, but it is not a completed product UI. UI contributors are welcome.

[中文说明](./README.zh-CN.md)

## Current Status

Runtime-wired today:

- `ContractGate`: checks proposed tool actions against the active Goal Contract
  before execution.
- `ImpactEvidenceGate`: records read/search evidence and requires evidence or
  disclosure before risky out-of-scope actions.
- Minimal frontend baseline in `src/`.
- Core command bridge for sessions, messages, chat, and `agent-event` streaming.
- Hook output handling hardening for high-output commands and UTF-8 tails.

Present in the codebase but not wired into the runtime path yet:

- `CompletionGate`
- `Verifier`
- `atlas-verifier` / `team_runtime` reviewer integration
- Goal Contract storage persistence

Atlas Harness is a defense-in-depth layer. It reduces silent goal drift and
forces evidence or disclosure around risky actions, but it is not a proof that an
agent can never drift.

## Quick Start

Prerequisites:

- Stable Rust
- Node.js 18+
- Tauri v2 system prerequisites for your operating system

Install dependencies:

```bash
npm install
```

Run the frontend dev server:

```bash
npm run dev
```

In a second terminal, run the Tauri shell:

```bash
cd src-tauri
cargo tauri dev
```

Useful checks:

```bash
npm run typecheck
cargo test
```

For a release-style frontend build:

```bash
npm run build
```

## Repository Map

```text
src/
  bridge.ts                  typed frontend entry point for backend commands
  app.ts                     minimal chat UI controller
  types.ts                   frontend-facing command and event types
src-tauri/src/
  agent/                     agent runtime and model/tool loop
  agent/atlas_harness/       Goal Contract gates and harness modules
  commands/                  Tauri command surface
  storage/                   local session/message storage
docs/
  COMMAND_BRIDGE.md          current frontend/backend command bridge
  REVIEW_FINDINGS.md         fixed issues and remaining follow-ups
```

## UI Contributors

The largest open contribution area is the UI. The baseline frontend should be
treated as a real, connected starting point, not as the final experience.

Start here:

- [CONTRIBUTING.md](./CONTRIBUTING.md)
- [docs/COMMAND_BRIDGE.md](./docs/COMMAND_BRIDGE.md)

One rule is non-negotiable: no fake buttons. If a control does not call a real
backend command, it must not look usable.

## Documentation

- [README.zh-CN.md](./README.zh-CN.md)
- [CONTRIBUTING.md](./CONTRIBUTING.md)
- [SECURITY.md](./SECURITY.md)
- [docs/COMMAND_BRIDGE.md](./docs/COMMAND_BRIDGE.md)
- [docs/REVIEW_FINDINGS.md](./docs/REVIEW_FINDINGS.md)

## License

Apache-2.0. See [LICENSE](./LICENSE).
