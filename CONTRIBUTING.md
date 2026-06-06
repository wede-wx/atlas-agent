# Contributing to Atlas

Thanks for considering a contribution. Atlas is a local-first desktop AI agent
built with Rust and Tauri.

## Current Priority

The biggest contribution area is UI.

The current frontend in `src/` is a minimal baseline. It is intentionally small
and only covers the real core chat loop. It should be improved or replaced by a
real product-quality interface over time, but it must remain honest about which
backend commands are actually connected.

Start with:

- `src/bridge.ts`
- `docs/COMMAND_BRIDGE.md`

`src/bridge.ts` is the frontend entry point for backend commands. Keep that file
or an equivalent bridge as the single source of truth for UI/backend calls.

## No Fake Buttons

Do not add controls that look usable unless they call a real backend command.

Acceptable:

- A button wired to a real command and verified through the UI path.
- A clearly disabled control with text explaining that it is not available yet.
- A documented issue for a future workflow.

Not acceptable:

- A button that only logs to the console.
- A button that pretends to run a command but does nothing.
- Placeholder data presented as real local data.

## Development

Install dependencies:

```bash
npm install
```

Run the frontend:

```bash
npm run dev
```

Run the Tauri shell in another terminal:

```bash
cd src-tauri
cargo tauri dev
```

Common checks:

```bash
npm run typecheck
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

## Code Style

- New code comments should be in English.
- Keep pull requests focused.
- Prefer fail-closed behavior in safety-sensitive paths.
- Do not weaken tests to make a change pass.
- Avoid unrelated formatting churn.

## Pull Request Expectations

Each PR should include:

- What changed.
- Why it changed.
- What user-visible behavior is affected.
- Test commands run and results.
- Any unverified paths or known limitations.

For UI changes, include the real command path you wired or explain why the
control is visibly disabled.
