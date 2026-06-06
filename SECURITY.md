# Security Policy

## Reporting a Vulnerability

Please report security issues privately.

Please use GitHub Security Advisories when available. If advisories are not
enabled yet, open a minimal public issue asking for a private contact without
disclosing vulnerability details.

Include a concise description, reproduction steps, expected behavior, actual
behavior, and any relevant logs with secrets removed.

## Scope

In scope:

- Rust backend in `src-tauri/`
- Atlas Harness
- Tool and permission system
- Local data store
- Baseline frontend in `src/`

Out of scope:

- Third-party model providers
- User-configured MCP servers
- Commands or tools the user explicitly configures

## Atlas Harness Boundary

Atlas Harness is a defense-in-depth mechanism for goal fidelity. The currently
runtime-wired gates are `ContractGate` and `ImpactEvidenceGate`.

The repository also contains `Verifier` and `CompletionGate`, but they are not
yet wired into the runtime path. Do not treat those modules as active runtime
protection until that integration is implemented and verified.

Atlas Harness is not an execution-isolation boundary. It cannot replace OS
permissions, process isolation, tool permission prompts, model-provider
security, or review of user-configured commands. If you find a way to silently
violate a frozen Goal Contract item, report it.

## Local Data and Secrets

Atlas is local-first. Sessions, messages, and run data are stored locally. Do not
include real provider credentials, personal session data, database files, logs,
or `.env` contents in issues or pull requests.
