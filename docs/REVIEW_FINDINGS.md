# Atlas Review Findings

This document records the open-source hardening findings that are relevant to
the current repository state.

## Fixed

### Preserve Path Matching Bypass

`ContractGate` now uses `path_match::path_matches_glob` instead of raw string
matching. The matcher normalizes `./`, `..`, backslashes, and absolute path
forms so preserve globs are harder to bypass accidentally.

### Unknown Write Tool Fail-Closed

`ProposedAction::is_mutating()` now fails closed when a tool name is unknown but
the arguments look like a write operation, such as path plus content or a shell
command.

### Command Matching False Positives

Command matching no longer relies on bare substring checks for words such as
`skip` or `disable`. This avoids blocking benign flags such as `skipLibCheck`
while still detecting verification-defeating patterns.

### Mass Deletion Detection

`glue.rs` extracts prior content from `old_str`, `old_string`, or `old_content`.
`ContractGate` can now compare prior and replacement content for edit actions,
instead of relying only on unified diff markers.

### Mutex Poison Recovery

The Atlas Harness mutex path in `core.rs` now recovers poisoned locks with
`unwrap_or_else(|poisoned| poisoned.into_inner())` instead of panicking.

### Baseline Frontend

A minimal runnable frontend baseline was added under `src/`, with
`src/bridge.ts` as the command boundary and `docs/COMMAND_BRIDGE.md` as the
frontend/backend contract for the core chat loop.

## Remaining Follow-Ups

### Contract Block Extraction

`glue.rs::extract_contract_block` still extracts Goal Contract text from
assistant prose. This is brittle. A future design should use a structured
channel, such as a dedicated tool call or a fenced block with a parser.

### CompletionGate

`CompletionGate` exists in the harness module but is not wired into the runtime
path yet. The current repository must not describe it as an active `done`
interceptor.

### Verifier

`Verifier` exists in the harness module but is not wired into `team_runtime` or
an `atlas-verifier` reviewer path yet.

### Goal Contract Persistence

Goal Contract installation is runtime-local. Storage persistence for contracts
has not been implemented.

## Current Runtime Harness Boundary

Runtime-wired today:

- `ContractGate`
- `ImpactEvidenceGate`

Not runtime-wired today:

- `CompletionGate`
- `Verifier`
- `atlas-verifier` / `team_runtime` reviewer
- Goal Contract storage persistence
