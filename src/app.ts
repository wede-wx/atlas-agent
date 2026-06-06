// Atlas baseline UI controller (framework-free, on purpose). It wires the real
// command bridge and renders the streamed run — including the interception card
// for blocked/unverified completions, which is the part of Atlas worth showing
// off. Replace the styling freely; keep the event handling as a reference.

import {
  isTauri,
  getSessions,
  createSession,
  deleteSession,
  getMessages,
  agentChat,
  onAgentEvent,
} from "./bridge";
import type { SessionRecord, AgentEvent, AgentRunEvent } from "./types";

// ---- tiny DOM helper -------------------------------------------------------
type Attrs = Record<string, string | undefined>;
function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Attrs = {},
  children: (Node | string)[] = [],
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined) continue;
    if (k === "class") node.className = v;
    else node.setAttribute(k, v);
  }
  for (const c of children) node.append(typeof c === "string" ? document.createTextNode(c) : c);
  return node;
}

// ---- app state -------------------------------------------------------------
interface Live {
  bubbles: Map<string, HTMLElement>; // message_id -> assistant text node
  ops: Map<string, HTMLElement>; // operation_id -> chip
  opsWrap: HTMLElement | null;
  running: boolean;
}

const state = {
  sessions: [] as SessionRecord[],
  activeId: null as string | null,
};

let railEl: HTMLElement;
let threadEl: HTMLElement;
let statusEl: HTMLElement;
let gateDot: HTMLElement;
let inputEl: HTMLTextAreaElement;
let modeEl: HTMLSelectElement;
let sendBtn: HTMLButtonElement;
let live: Live = { bubbles: new Map(), ops: new Map(), opsWrap: null, running: false };

// ---- bootstrap -------------------------------------------------------------
export async function mount(root: HTMLElement) {
  root.append(buildShell());
  await onAgentEvent((env) => handleEvent(env.event));

  if (!isTauri()) {
    banner("Backend not connected — preview mode. Run inside Tauri (`tauri dev`) for a live agent.");
    return;
  }
  try {
    await refreshSessions();
    if (!state.activeId && state.sessions[0]) await selectSession(state.sessions[0].id);
  } catch (e) {
    banner(`Could not reach the backend: ${String(e)}`);
  }
}

// ---- shell -----------------------------------------------------------------
function buildShell(): HTMLElement {
  gateDot = el("span", { class: "gate-dot", title: "Goal-fidelity gate idle" });
  const brand = el("div", { class: "brand" }, [
    el("span", { class: "brand-mark" }, ["ATLAS"]),
    gateDot,
  ]);
  const newBtn = el("button", { class: "btn ghost", title: "New session" }, ["+ New"]);
  newBtn.addEventListener("click", onNewSession);

  railEl = el("nav", { class: "rail-list" });
  const rail = el("aside", { class: "rail" }, [
    el("div", { class: "rail-head" }, [el("span", { class: "label" }, ["Sessions"]), newBtn]),
    railEl,
  ]);

  threadEl = el("div", { class: "thread" }, [emptyState()]);
  statusEl = el("div", { class: "status" });

  inputEl = el("textarea", {
    class: "composer-input",
    rows: "1",
    placeholder: "Message Atlas…  (Enter to send, Shift+Enter for newline)",
  }) as HTMLTextAreaElement;
  inputEl.addEventListener("input", autosize);
  inputEl.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void onSend();
    }
  });

  modeEl = el("select", { class: "mode" }, [
    el("option", { value: "chat" }, ["chat"]),
    el("option", { value: "agent" }, ["agent"]),
  ]) as HTMLSelectElement;

  sendBtn = el("button", { class: "btn primary" }, ["Send"]) as HTMLButtonElement;
  sendBtn.addEventListener("click", () => void onSend());

  const composer = el("div", { class: "composer" }, [
    statusEl,
    el("div", { class: "composer-row" }, [modeEl, inputEl, sendBtn]),
  ]);

  const main = el("main", { class: "main" }, [
    el("header", { class: "topbar" }, [brand]),
    threadEl,
    composer,
  ]);

  return el("div", { class: "layout" }, [rail, main]);
}

function emptyState(): HTMLElement {
  return el("div", { class: "empty" }, [
    el("div", { class: "empty-mark" }, ["◇"]),
    el("p", {}, ["Start a session, or send a message to begin."]),
  ]);
}

function banner(text: string) {
  statusEl.replaceChildren(el("div", { class: "warn" }, [text]));
}

function autosize() {
  inputEl.style.height = "auto";
  inputEl.style.height = `${Math.min(inputEl.scrollHeight, 200)}px`;
}

// ---- sessions --------------------------------------------------------------
async function refreshSessions() {
  state.sessions = await getSessions();
  renderRail();
}

function renderRail() {
  railEl.replaceChildren(
    ...state.sessions.map((s) => {
      const item = el("button", {
        class: `rail-item${s.id === state.activeId ? " active" : ""}`,
      }, [el("span", { class: "rail-title" }, [s.title || "Untitled"])]);
      item.addEventListener("click", () => void selectSession(s.id));

      const del = el("span", { class: "rail-del", title: "Delete" }, ["×"]);
      del.addEventListener("click", async (e) => {
        e.stopPropagation();
        state.sessions = await deleteSession(s.id);
        if (state.activeId === s.id) {
          state.activeId = state.sessions[0]?.id ?? null;
          if (state.activeId) await selectSession(state.activeId);
          else threadEl.replaceChildren(emptyState());
        }
        renderRail();
      });
      item.append(del);
      return item;
    }),
  );
}

async function onNewSession() {
  const s = await createSession("New session");
  await refreshSessions();
  await selectSession(s.id);
  inputEl.focus();
}

async function selectSession(id: string) {
  state.activeId = id;
  renderRail();
  resetLive();
  const msgs = await getMessages(id);
  threadEl.replaceChildren();
  if (msgs.length === 0) threadEl.append(emptyState());
  for (const m of msgs) appendMessage(m.role === "user" ? "user" : "assistant", m.content);
  scrollThread();
}

// ---- sending ---------------------------------------------------------------
async function onSend() {
  const text = inputEl.value.trim();
  if (!text || live.running) return;

  if (!state.activeId) {
    const s = await createSession(text.slice(0, 40));
    await refreshSessions();
    state.activeId = s.id;
    renderRail();
    resetLive();
    threadEl.replaceChildren();
  }
  const sessionId = state.activeId!;

  // drop empty-state if present
  threadEl.querySelector(".empty")?.remove();
  appendMessage("user", text);
  inputEl.value = "";
  autosize();
  setRunning(true);

  // live run scaffold (ops appear here, above the assistant bubble)
  live.opsWrap = el("div", { class: "ops" });
  threadEl.append(live.opsWrap);
  scrollThread();

  try {
    await agentChat({ sessionId, message: text, mode: modeEl.value });
  } catch (e) {
    appendInterceptOrError("error", `Run failed: ${String(e)}`);
  } finally {
    setRunning(false);
  }
}

function setRunning(on: boolean) {
  live.running = on;
  sendBtn.disabled = on;
  sendBtn.textContent = on ? "Working…" : "Send";
  gateDot.classList.toggle("live", on);
  gateDot.title = on ? "Run active — gate enforcing" : "Goal-fidelity gate idle";
  if (!on) statusEl.replaceChildren();
}

function resetLive() {
  live = { bubbles: new Map(), ops: new Map(), opsWrap: null, running: false };
}

// ---- event handling --------------------------------------------------------
function handleEvent(ev: AgentEvent) {
  switch (ev.type) {
    case "Thinking":
      setStatus(ev.content);
      break;

    case "ResponseStarted":
      live.bubbles.set(ev.message_id, beginAssistant());
      break;
    case "ResponseDelta": {
      const b = live.bubbles.get(ev.message_id) ?? beginAssistant(ev.message_id);
      b.textContent = (b.textContent ?? "") + ev.content;
      scrollThread();
      break;
    }
    case "ResponseCompleted":
    case "Response": {
      const b = live.bubbles.get(ev.message_id) ?? beginAssistant(ev.message_id);
      b.textContent = ev.content; // authoritative final text
      scrollThread();
      break;
    }
    case "ResponseFallbackStarted":
      setStatus(`Falling back to another provider: ${ev.reason}`);
      break;

    case "OperationStarted":
      addOp(ev.operation_id, ev.tool_name, ev.label, ev.target ?? ev.command ?? undefined);
      break;
    case "OperationFinished":
      finishOp(ev.operation_id, "ok", ev.summary);
      break;
    case "OperationFailed":
      finishOp(ev.operation_id, "fail", ev.summary);
      break;

    case "RunEvent":
      handleRunEvent(ev.event);
      break;

    // Quiet diagnostics — surfaced as a status line, not noisy cards.
    case "UnknownToolRequested":
      setStatus(`Unknown tool requested: ${ev.requested}`);
      break;
    default:
      break;
  }
}

function handleRunEvent(re: AgentRunEvent) {
  switch (re.type) {
    case "Blocked":
      // The interception — a completion the audit refused to call done.
      appendInterceptOrError(re.status === "unverified" ? "unverified" : "blocked", re.footer);
      gateDot.classList.add("tripped");
      break;
    case "Failed":
      appendInterceptOrError("error", re.error ?? "run failed");
      break;
    case "Finished":
      setStatus("");
      break;
    case "Paused":
      setStatus("Run paused at a tool boundary.");
      break;
    case "Cancelled":
      setStatus("Run cancelled.");
      break;
    default:
      break;
  }
}

// ---- rendering primitives --------------------------------------------------
function appendMessage(role: "user" | "assistant", text: string): HTMLElement {
  const body = el("div", { class: "bubble-body" });
  body.textContent = text;
  const row = el("div", { class: `msg ${role}` }, [
    el("div", { class: "bubble-role" }, [role === "user" ? "You" : "Atlas"]),
    body,
  ]);
  threadEl.append(row);
  scrollThread();
  return body;
}

function beginAssistant(id?: string): HTMLElement {
  const body = appendMessage("assistant", "");
  if (id) live.bubbles.set(id, body);
  return body;
}

function setStatus(text: string) {
  if (!text) {
    statusEl.replaceChildren();
    return;
  }
  statusEl.replaceChildren(el("div", { class: "thinking" }, [el("span", { class: "pulse" }), text]));
}

function addOp(id: string, tool: string, label: string, detail?: string) {
  if (!live.opsWrap) {
    live.opsWrap = el("div", { class: "ops" });
    threadEl.append(live.opsWrap);
  }
  const chip = el("div", { class: "op running" }, [
    el("span", { class: "op-tool" }, [tool]),
    el("span", { class: "op-label" }, [label]),
    detail ? el("span", { class: "op-detail" }, [detail]) : el("span"),
  ]);
  live.ops.set(id, chip);
  live.opsWrap.append(chip);
  scrollThread();
}

function finishOp(id: string, kind: "ok" | "fail", summary: string) {
  const chip = live.ops.get(id);
  if (!chip) return;
  chip.classList.remove("running");
  chip.classList.add(kind === "ok" ? "done" : "failed");
  chip.querySelector(".op-detail")?.remove();
  chip.append(el("span", { class: "op-summary" }, [summary]));
}

function appendInterceptOrError(kind: "blocked" | "unverified" | "error", footer: string) {
  const labels = { blocked: "INTERCEPTED · BLOCKED", unverified: "INTERCEPTED · UNVERIFIED", error: "RUN ERROR" };
  const card = el("div", { class: `intercept ${kind}` }, [
    el("div", { class: "intercept-head" }, [labels[kind]]),
    el("div", { class: "intercept-body" }, [footer || "(no detail)"]),
  ]);
  threadEl.append(card);
  scrollThread();
}

function scrollThread() {
  threadEl.scrollTop = threadEl.scrollHeight;
}
