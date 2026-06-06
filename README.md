# Atlas

Atlas is a local-first desktop AI agent (Rust + [Tauri](https://v2.tauri.app/))
built around one goal: **an agent that won't fake "done."**

Most agent failures aren't "it couldn't do the task" — they're *silent goal
drift*: the agent quietly narrows the goal, stubs a function, comments out a
failing test, edits a file it was told to preserve, and then reports success.
You can't fix that by asking the model "are you being honest?" — that's the same
judgment that already drifted.

Atlas separates **declaration** from **enforcement**. A request is frozen into a
**Goal Contract** (what must happen, what must not, what to preserve, scope, and
how each item is verified), and a set of gates check every tool action against
that contract *mechanically* — not by asking the model to grade itself.

> This repository is architecture-focused with a **minimal, runnable frontend
> baseline**. It is not a finished product, and **not every gate is wired into
> the runtime yet** (see below). **UI contributors are especially welcome.**

[中文说明 → README.zh-CN.md](./README.zh-CN.md)

## How it works

The harness has four gates. We're explicit about what is enforced at runtime
today versus what exists in the code but isn't wired into the run loop yet:

| Gate | What it does | Status |
|---|---|---|
| **ContractGate** | Structurally compares each proposed tool action to the contract (preserve paths, forbidden patterns, scope) **before** it runs. | ✅ wired |
| **ImpactEvidenceGate** | Records read/search evidence and requires evidence or disclosure before risky out-of-scope actions. | ✅ wired |
| **Verifier** | Independent, read-only adversarial review before a task may complete. | ⏳ present, not wired |
| **CompletionGate** | `done` must bind a real verification artifact, not the model's self-assessment. | ⏳ present, not wired |

Also present-but-not-yet-wired: the `atlas-verifier` / `team_runtime` reviewer
integration and Goal Contract storage persistence.

**The Atlas Harness is a defense-in-depth layer, not a proof.** It raises the
cost of silent goal drift and forces evidence or disclosure around risky
actions; it does not guarantee an agent can never drift. See
[SECURITY.md](./SECURITY.md) and [docs/REVIEW_FINDINGS.md](./docs/REVIEW_FINDINGS.md)
for an honest account of its limits.

## Quick start

Prerequisites: stable **Rust**, **Node.js 18+**, and the
[Tauri v2 system prerequisites](https://v2.tauri.app/start/prerequisites/) for
your OS.

```bash
npm install        # frontend deps

npm run dev        # terminal A — Vite dev server (http://localhost:1420)
cd src-tauri && cargo tauri dev   # terminal B — the Tauri shell
```

Local model providers (ollama, LM Studio) work out of the box — the CSP already
allows `localhost` / `127.0.0.1`.

Checks and a release-style build:

```bash
npm run typecheck
cargo test
npm run build
```

## Repository map

```
src/                          minimal frontend baseline (TypeScript + Vite)
  bridge.ts                   typed wrappers over Tauri commands — the UI talks to the backend here
  types.ts                    types mirrored from the Rust side
  app.ts                      framework-free chat UI controller
  main.ts / styles.css        entry point + cockpit-style theme
src-tauri/src/
  agent/                      agent runtime and the model/tool loop
  agent/atlas_harness/        Goal Contract + gates (contract_gate, impact_evidence,
                              completion_gate, verifier, goal_contract, path_match, glue)
  commands/                   Tauri command surface
  storage/                    local-first session/message storage
scripts/                      helper scripts (e.g. browser automation)
docs/
  ARCHITECTURE.md             system overview
  COMMAND_BRIDGE.md           frontend <-> backend command + event contract
  REVIEW_FINDINGS.md          fixed issues + remaining follow-ups
ATLAS_CODE_REVIEW_COMMAND.md  the project's own code-review checklist (used to dogfood Atlas)
```

## Contributing — the UI is the biggest open area

The largest contribution area is the **frontend**. The baseline in `src/` is a
real, connected starting point — not the final experience. Extend it, restyle
it, or replace it with your framework of choice; keep `bridge.ts` (or an
equivalent) as the single source of truth for backend calls.

Start here:

- [CONTRIBUTING.md](./CONTRIBUTING.md)
- [docs/COMMAND_BRIDGE.md](./docs/COMMAND_BRIDGE.md) — everything a UI needs to
  talk to the backend (the core commands + the `agent-event` stream).

## Documentation

- [README.zh-CN.md](./README.zh-CN.md)
- [CONTRIBUTING.md](./CONTRIBUTING.md)
- [SECURITY.md](./SECURITY.md)
- [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md)
- [docs/COMMAND_BRIDGE.md](./docs/COMMAND_BRIDGE.md)
- [docs/REVIEW_FINDINGS.md](./docs/REVIEW_FINDINGS.md)

## License

[Apache-2.0](./LICENSE). Note: Apache-2.0 permits commercial use. If you'd
rather restrict that, you'd need a different license (e.g. AGPL-3.0 to keep it
open while requiring source disclosure for modified/network use, or a
non-commercial source-available license — which is not "open source").
