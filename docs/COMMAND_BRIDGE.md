# Atlas Command Bridge

This is the contract between any frontend and the Rust backend. If you are
building a UI, this is the page you need. This document covers only the
**core chat loop** the baseline frontend wires. The backend has more commands;
their source of truth remains the Rust `#[tauri::command]` definitions until a
real UI flow needs a typed wrapper in `src/bridge.ts`.

> Casing: backend structs carry no serde `rename_all`, so **JSON fields are
> snake_case** and **event tags are PascalCase**. Tauri command *argument* keys
> are camelCase on the JS side (Tauri converts them).

## Sessions

| Command | Args | Returns |
|---|---|---|
| `get_sessions` | — | `SessionRecord[]` |
| `create_session` | `{ title, projectId? }` | `SessionRecord` |
| `delete_session` | `{ id }` | `SessionRecord[]` (remaining) |

## Messages

| Command | Args | Returns |
|---|---|---|
| `get_messages` | `{ sessionId }` | `MessageRecord[]` |

## Chat (the main loop)

```ts
agent_chat_v2({
  sessionId: string,
  message: string,
  displayMessage?: string,   // what to persist/show if different from the model prompt
  mode?: string,             // "chat" | "agent" | ...
  attachments?: AgentAttachment[],
}) -> string                 // resolves with the final assistant text
```

`agent_chat_v2` does three things: persists the user message, runs the agent,
and **streams progress** on the `agent-event` channel while running. If a run is
already active for the session, a second call is queued as *guidance* (merged
into the running run) instead of starting a new run.

## The event channel — `agent-event`

Subscribe with Tauri's `listen("agent-event", …)`. Payload envelope:

```ts
interface AgentEventEnvelope {
  sessionId: string;
  runId: string;
  event: AgentEvent;   // tagged union, tag = "type"
}
```

> **Confirm the envelope.** `agent_chat_v2` emits exactly this shape for the
> guidance case. The streamed `AgentEvent`s come from an internal
> `Sender<AgentEvent>` forwarded to the window; the natural (and assumed) shape
> is the same `{ sessionId, runId, event }`. `bridge.ts::onAgentEvent` reads
> `payload.event` with a flat fallback. If your forwarder differs, adjust there.

### AgentEvent variants you will actually render

| `type` | Fields | Meaning |
|---|---|---|
| `ResponseStarted` | `message_id` | begin an assistant message |
| `ResponseDelta` | `message_id, content` | append a streamed chunk |
| `ResponseCompleted` / `Response` | `message_id, content` | authoritative final text |
| `Thinking` | `content` | transient status line |
| `OperationStarted` | `operation_id, tool_name, label, target?, command?` | a tool/operation began |
| `OperationOutput` | `operation_id, stream, content` | streamed tool output |
| `OperationFinished` | `operation_id, status, summary` | operation succeeded |
| `OperationFailed` | `operation_id, summary` | operation failed |
| `RunEvent` | `event: AgentRunEvent` | run lifecycle (see below) |

Lower-frequency diagnostics also exist (`ToolVisibilityDecision`,
`ModelToolParseDiagnostic`, `UnknownToolRequested`, `ToolNormalizationApplied`,
`SubAgent*`, `FinalAudit`) — render or ignore as you like.

### AgentRunEvent (inside `RunEvent`)

| `type` | Fields | Meaning |
|---|---|---|
| `Started` | `run` | run created |
| `Iteration` | `run_id, iteration` | model/tool loop tick |
| `Blocked` | `run_id, status, footer` | **the interception** — the audit refused to mark the response complete. `status` is `"blocked"` or `"unverified"`; `footer` is the audit summary. Render this distinctly. |
| `Finished` | `run_id` | run complete |
| `Failed` | `run_id, error, retryable` | run failed |
| `Paused` / `Resumed` / `Cancelled` | `run_id` | lifecycle |
| `GuidanceQueued` / `GuidanceMerged` | `run_id, count` | mid-run user message queued/merged |

The `Blocked` event is the one thing a good Atlas UI should make unmissable — it
is the whole point of the project (a model that won't fake "done"). The baseline
renders it as an interception card; do better.

## Minimal end-to-end (what the baseline does)

```ts
const unlisten = await onAgentEvent(({ event }) => render(event));
const sessions = await getSessions();
const msgs = await getMessages(sessions[0].id);
await agentChat({ sessionId: sessions[0].id, message: "hello", mode: "chat" });
// progress arrives via the listener; the promise resolves with the final text
```

## Beyond the core loop
Other commands cover runs/timeline/diff/terminal, evals, knowledge, teams and
handoffs, model routing/cost, workspaces, and external-agent protocols. Keep
those command contracts in Rust until a real frontend workflow needs them, then
add typed wrappers to `src/bridge.ts`. Keep `bridge.ts` the single source of
truth for frontend calls — and **no fake buttons**: if it is not wired, do not
show a control that pretends it is.
