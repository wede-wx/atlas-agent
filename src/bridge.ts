// The command bridge. Every function here maps to a real `#[tauri::command]`
// in src-tauri. This file is the single source of truth for what the UI can
// call; see docs/COMMAND_BRIDGE.md for the baseline surface. This frontend
// wires only the core chat loop and leaves everything else to the Rust command
// definitions until a real UI flow needs it.

import type {
  SessionRecord,
  MessageRecord,
  AgentEventEnvelope,
} from "./types";

// Tauri v2 entry points. Imported lazily so a plain `vite dev` in a browser
// (no Tauri runtime) does not hard-crash on module load.
type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
type ListenFn = (event: string, cb: (e: { payload: unknown }) => void) => Promise<() => void>;

let _invoke: InvokeFn | null = null;
let _listen: ListenFn | null = null;

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function api(): Promise<{ invoke: InvokeFn; listen: ListenFn }> {
  if (_invoke && _listen) return { invoke: _invoke, listen: _listen };
  const core = await import("@tauri-apps/api/core");
  const evt = await import("@tauri-apps/api/event");
  _invoke = core.invoke as unknown as InvokeFn;
  _listen = evt.listen as unknown as ListenFn;
  return { invoke: _invoke, listen: _listen };
}

// ---------------------------------------------------------------------------
// WIRED — sessions
// ---------------------------------------------------------------------------

export async function getSessions(): Promise<SessionRecord[]> {
  const { invoke } = await api();
  return invoke<SessionRecord[]>("get_sessions");
}

export async function createSession(
  title: string,
  projectId?: string,
): Promise<SessionRecord> {
  const { invoke } = await api();
  return invoke<SessionRecord>("create_session", {
    title,
    projectId: projectId ?? null,
  });
}

export async function deleteSession(id: string): Promise<SessionRecord[]> {
  const { invoke } = await api();
  return invoke<SessionRecord[]>("delete_session", { id });
}

// ---------------------------------------------------------------------------
// WIRED — messages + chat
// ---------------------------------------------------------------------------

export async function getMessages(sessionId: string): Promise<MessageRecord[]> {
  const { invoke } = await api();
  return invoke<MessageRecord[]>("get_messages", { sessionId });
}

/**
 * Send a message and run the agent. Streams progress on the "agent-event"
 * channel (subscribe via {@link onAgentEvent}); resolves with the final
 * assistant text. `mode` is the agent mode string ("chat", "agent", ...).
 */
export async function agentChat(opts: {
  sessionId: string;
  message: string;
  displayMessage?: string;
  mode?: string;
}): Promise<string> {
  const { invoke } = await api();
  return invoke<string>("agent_chat_v2", {
    sessionId: opts.sessionId,
    message: opts.message,
    displayMessage: opts.displayMessage ?? null,
    mode: opts.mode ?? "chat",
    attachments: null,
  });
}

/**
 * Subscribe to streamed run events. Returns an unlisten function.
 *
 * Envelope note: agent_chat_v2 emits `{ sessionId, runId, event }` on
 * "agent-event". We read `payload.event`; if your event forwarder emits the
 * AgentEvent flat (i.e. `payload.type` directly), the fallback below handles
 * it. Confirm the forwarder shape in core.rs and simplify if desired.
 */
export async function onAgentEvent(
  handler: (env: AgentEventEnvelope) => void,
): Promise<() => void> {
  const { listen } = await api();
  return listen("agent-event", ({ payload }) => {
    const p = payload as Record<string, unknown>;
    if (p && typeof p === "object" && "event" in p) {
      handler(p as unknown as AgentEventEnvelope);
    } else if (p && typeof p === "object" && "type" in p) {
      // Flat fallback: wrap a bare AgentEvent.
      handler({
        sessionId: (p.sessionId as string) ?? "",
        runId: (p.runId as string) ?? "",
        event: p as never,
      });
    }
  });
}
