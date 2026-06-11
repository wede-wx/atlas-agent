import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import {
  agentChat,
  cancelAgentChat,
  createSession,
  getMessages,
  getSessions,
  listProjects,
  onAgentEvent,
  pauseAgentChat,
  resumeAgentChat,
} from "../../bridge";
import type { AgentEventEnvelope, MessageRecord, ProjectRecord, SessionRecord } from "../../types";

export type PageKey = "knowledge" | "plugins" | "evals" | "settings";

type AgentMode = "chat" | "plan" | "review";
type MessageRole = "user" | "assistant" | "system" | "tool";

type UiMessage = {
  id: string;
  role: MessageRole;
  content: string;
  createdAt?: string | number;
  error?: boolean;
};

type ToolActivity = {
  id: string;
  title: string;
  detail?: string;
  output?: string;
  status: "running" | "success" | "failed";
};

type GateNotice = {
  id: string;
  title: string;
  message: string;
  detail?: string;
  kind?: "blocked" | "failed" | "cancelled";
};

type NormalizedEvent = {
  kind: string;
  data: Record<string, unknown>;
};

const modeLabels: Record<AgentMode, string> = {
  chat: "默认模式",
  plan: "计划模式",
  review: "审查模式",
};

const modeHints: Record<AgentMode, string> = {
  chat: "常规执行",
  plan: "先规划再执行",
  review: "偏审查/复核",
};

function id(prefix: string): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `${prefix}-${crypto.randomUUID()}`;
  }
  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

function friendlyChatError(error: unknown): string {
  const raw = errorText(error);
  const lower = raw.toLowerCase();
  const configLike =
    lower.includes("model") ||
    lower.includes("provider") ||
    lower.includes("api key") ||
    lower.includes("apikey") ||
    lower.includes("connection") ||
    lower.includes("config") ||
    raw.includes("模型") ||
    raw.includes("连接") ||
    raw.includes("配置") ||
    raw.includes("密钥");

  if (configLike) {
    return `没有可用的模型连接，请先在设置里配置模型连接。\n后端返回：${raw}`;
  }
  return `发送失败：${raw}`;
}

function firstText(value: unknown): string {
  if (typeof value === "string") return value;
  if (value == null) return "";
  if (Array.isArray(value)) return value.map(firstText).filter(Boolean).join("\n");
  if (typeof value === "object") {
    const record = value as Record<string, unknown>;
    return firstText(record.content ?? record.text ?? record.message ?? record.delta ?? record.output ?? record.error);
  }
  return String(value);
}

function asRecord(value: unknown): Record<string, unknown> {
  if (value && typeof value === "object" && !Array.isArray(value)) return value as Record<string, unknown>;
  return { value };
}

function normalizeRole(role: string): MessageRole {
  return role === "user" || role === "assistant" || role === "system" || role === "tool" ? role : "assistant";
}

function toUiMessages(records: MessageRecord[]): UiMessage[] {
  return records.map((message) => ({
    id: message.id,
    role: normalizeRole(message.role),
    content: message.content ?? "",
    createdAt: message.created_at,
  }));
}

function normalizeEvent(payload: unknown): NormalizedEvent {
  if (!payload || typeof payload !== "object") return { kind: "event", data: { value: payload } };

  const record = payload as Record<string, unknown>;
  if (typeof record.type === "string") return { kind: record.type, data: asRecord(record.payload ?? record.data ?? record) };
  if (typeof record.kind === "string") return { kind: record.kind, data: asRecord(record.payload ?? record.data ?? record) };
  if (typeof record.event === "string") return { kind: record.event, data: asRecord(record.payload ?? record.data ?? record) };

  const keys = Object.keys(record);
  if (keys.length === 1) return { kind: keys[0], data: asRecord(record[keys[0]]) };

  return { kind: "event", data: record };
}

function eventText(data: Record<string, unknown>): string {
  return firstText(data.delta ?? data.content ?? data.message ?? data.text ?? data.output ?? data.error ?? data.value);
}

function eventSessionId(data: Record<string, unknown>): string | null {
  const direct = data.session_id ?? data.sessionId;
  if (typeof direct === "string") return direct;
  const run = asRecord(data.run);
  const nested = run.session_id ?? run.sessionId;
  return typeof nested === "string" ? nested : null;
}

function formatStamp(value?: string | number | null): string {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function relativeTime(value?: string | number | null): string {
  if (!value) return "";
  const time = new Date(value).getTime();
  if (Number.isNaN(time)) return "";
  const diff = Date.now() - time;
  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;
  if (diff < minute) return "刚刚";
  if (diff < hour) return `${Math.max(1, Math.floor(diff / minute))}分钟前`;
  if (diff < day) return `${Math.floor(diff / hour)}小时前`;
  if (diff < 2 * day) return "昨天";
  return `${Math.floor(diff / day)}天前`;
}

function renderParagraphs(content: string): ReactNode {
  if (!content.trim()) return <p className="muted-line">正在思考...</p>;
  return content.split(/\n{2,}/g).map((chunk, index) => (
    <p className="backend-preserve-lines" key={`${index}-${chunk.slice(0, 16)}`}>
      {chunk}
    </p>
  ));
}

function Icon({ children, small = false }: { children: ReactNode; small?: boolean }) {
  return (
    <svg className={`ico${small ? " ico-sm" : ""}`} viewBox="0 0 20 20" aria-hidden="true">
      {children}
    </svg>
  );
}

export function BackendChatLayer({
  navTarget,
  threadTarget,
  composerTarget,
  onActiveSessionChange,
  onOpenSearch,
  onOpenPage,
}: {
  navTarget: HTMLElement | null;
  threadTarget: HTMLElement | null;
  composerTarget: HTMLElement | null;
  onActiveSessionChange?: (sessionId: string | null) => void;
  onOpenSearch: () => void;
  onOpenPage: (page: PageKey) => void;
}) {
  const [sessions, setSessions] = useState<SessionRecord[]>([]);
  const [projects, setProjects] = useState<ProjectRecord[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<UiMessage[]>([]);
  const [tools, setTools] = useState<ToolActivity[]>([]);
  const [gates, setGates] = useState<GateNotice[]>([]);
  const [input, setInput] = useState("");
  const [mode, setMode] = useState<AgentMode>("chat");
  const [modeMenuOpen, setModeMenuOpen] = useState(false);
  const [loading, setLoading] = useState(true);
  const [messagesLoading, setMessagesLoading] = useState(false);
  const [sending, setSending] = useState(false);
  const [paused, setPaused] = useState(false);
  const [status, setStatus] = useState("");
  const [error, setError] = useState<string | null>(null);

  const activeSessionRef = useRef<string | null>(null);
  const streamingMessageRef = useRef<string | null>(null);
  const loadMessagesRef = useRef<(sessionId: string) => Promise<void>>(async () => undefined);

  const activeSession = useMemo(
    () => sessions.find((session) => session.id === activeSessionId) ?? null,
    [activeSessionId, sessions],
  );

  useEffect(() => {
    activeSessionRef.current = activeSessionId;
  }, [activeSessionId]);

  useEffect(() => {
    onActiveSessionChange?.(activeSessionId);
  }, [activeSessionId, onActiveSessionChange]);

  useEffect(() => {
    const title = activeSession?.title?.trim() || "Atlas";
    const topbarSession = document.querySelector(".tb-session");
    if (topbarSession) topbarSession.textContent = title;
  }, [activeSession?.title]);

  const loadMessagesForSession = useCallback(async (sessionId: string) => {
    setMessagesLoading(true);
    setError(null);
    try {
      const records = await getMessages(sessionId);
      setMessages(toUiMessages(records));
      setTools([]);
      setGates([]);
      streamingMessageRef.current = null;
    } catch (err) {
      setError(`读取消息失败：${errorText(err)}`);
      setMessages([]);
    } finally {
      setMessagesLoading(false);
    }
  }, []);

  useEffect(() => {
    loadMessagesRef.current = loadMessagesForSession;
  }, [loadMessagesForSession]);

  const refreshSessions = useCallback(async () => {
    const records = await getSessions();
    const visible = records.filter((session) => !session.archived_at);
    setSessions(visible);
    return visible;
  }, []);

  useEffect(() => {
    let cancelled = false;

    async function bootstrap() {
      setLoading(true);
      setError(null);
      const [sessionResult, projectResult] = await Promise.allSettled([getSessions(), listProjects()]);

      if (cancelled) return;

      if (projectResult.status === "fulfilled") {
        setProjects(projectResult.value);
      } else {
        setProjects([]);
        setError(`读取项目失败：${errorText(projectResult.reason)}`);
      }

      if (sessionResult.status === "fulfilled") {
        const visible = sessionResult.value.filter((session) => !session.archived_at);
        setSessions(visible);
        const first = visible[0] ?? null;
        setActiveSessionId(first?.id ?? null);
        if (first) await loadMessagesForSession(first.id);
      } else {
        setSessions([]);
        setActiveSessionId(null);
        setMessages([]);
        setError(`读取会话失败：${errorText(sessionResult.reason)}`);
      }

      if (!cancelled) setLoading(false);
    }

    void bootstrap();
    return () => {
      cancelled = true;
    };
  }, [loadMessagesForSession]);

  const handleAgentEvent = useCallback((env: AgentEventEnvelope) => {
    const payload = (env as unknown as Record<string, unknown>).payload ?? env;
    const normalized = normalizeEvent(payload);
    const nested = normalized.kind === "RunEvent" ? normalizeEvent(normalized.data.event ?? normalized.data.value ?? normalized.data) : normalized;
    const kind = nested.kind.toLowerCase();
    const data = nested.data;
    const sid = eventSessionId(data);
    const activeSid = activeSessionRef.current;

    if (sid && activeSid && sid !== activeSid) return;

    if (kind.includes("thinking")) {
      setStatus(eventText(data) || "Atlas 正在思考...");
      return;
    }

    if (kind.includes("response_started")) {
      const messageId = id("assistant");
      streamingMessageRef.current = messageId;
      setMessages((current) => [...current, { id: messageId, role: "assistant", content: "" }]);
      setStatus("Atlas 正在回复...");
      return;
    }

    if (kind.includes("response_delta")) {
      const delta = eventText(data);
      if (!delta) return;
      setMessages((current) => {
        const streamingId = streamingMessageRef.current;
        if (!streamingId || !current.some((message) => message.id === streamingId)) {
          const messageId = id("assistant");
          streamingMessageRef.current = messageId;
          return [...current, { id: messageId, role: "assistant", content: delta }];
        }
        return current.map((message) =>
          message.id === streamingId ? { ...message, content: `${message.content}${delta}` } : message,
        );
      });
      setStatus("Atlas 正在回复...");
      return;
    }

    if (kind.includes("response_completed") || kind === "response" || kind.includes("responsefinished")) {
      const finalText = eventText(data);
      const streamingId = streamingMessageRef.current;
      setMessages((current) => {
        if (streamingId && current.some((message) => message.id === streamingId)) {
          return current.map((message) => (message.id === streamingId ? { ...message, content: finalText || message.content } : message));
        }
        return finalText ? [...current, { id: id("assistant"), role: "assistant", content: finalText }] : current;
      });
      streamingMessageRef.current = null;
      setStatus("");
      return;
    }

    if (kind.includes("operation_started") || kind.includes("tool_started") || kind.includes("subagent_started")) {
      const activityId = String(data.id ?? data.operation_id ?? data.name ?? id("tool"));
      setTools((current) => [
        ...current,
        {
          id: activityId,
          title: firstText(data.name ?? data.tool ?? data.command ?? data.role ?? "正在运行工具"),
          detail: firstText(data.detail ?? data.task ?? data.args ?? data.input),
          status: "running",
        },
      ]);
      setStatus("正在执行工具调用...");
      return;
    }

    if (kind.includes("operation_output") || kind.includes("tool_output")) {
      const activityId = String(data.id ?? data.operation_id ?? data.name ?? "");
      const output = eventText(data);
      setTools((current) =>
        current.map((tool) => (activityId && tool.id === activityId ? { ...tool, output: `${tool.output ?? ""}${output}` } : tool)),
      );
      return;
    }

    if (kind.includes("operation_finished") || kind.includes("tool_finished") || kind.includes("subagent_finished")) {
      const activityId = String(data.id ?? data.operation_id ?? data.name ?? "");
      setTools((current) => current.map((tool) => (activityId && tool.id === activityId ? { ...tool, status: "success" } : tool)));
      setStatus("");
      return;
    }

    if (kind.includes("operation_failed") || kind.includes("tool_failed") || kind.includes("subagent_failed")) {
      const activityId = String(data.id ?? data.operation_id ?? data.name ?? "");
      const message = eventText(data) || "工具调用失败";
      setTools((current) =>
        current.map((tool) => (activityId && tool.id === activityId ? { ...tool, status: "failed", output: message } : tool)),
      );
      setError(message);
      return;
    }

    if (kind.includes("blocked")) {
      setGates((current) => [
        ...current,
        {
          id: id("gate"),
          title: firstText(data.title ?? data.reason ?? "运行被防线拦截"),
          message: firstText(data.message ?? data.reason ?? data.value ?? "后端返回了拦截事件。"),
          detail: firstText(data.detail ?? data.contract_item ?? data.contractItem),
          kind: "blocked",
        },
      ]);
      setSending(false);
      setStatus("运行已被拦截");
      return;
    }

    if (kind.includes("paused")) {
      setPaused(true);
      setStatus("运行已暂停");
      return;
    }

    if (kind.includes("resumed")) {
      setPaused(false);
      setStatus("运行已恢复");
      return;
    }

    if (kind.includes("cancelled") || kind.includes("canceled")) {
      setSending(false);
      setPaused(false);
      setStatus("运行已取消");
      setGates((current) => [
        ...current,
        { id: id("gate"), title: "运行已取消", message: eventText(data) || "后端已确认取消请求。", kind: "cancelled" },
      ]);
      return;
    }

    if (kind.includes("failed")) {
      const message = eventText(data) || "运行失败";
      setSending(false);
      setPaused(false);
      setError(message);
      setGates((current) => [...current, { id: id("gate"), title: "运行失败", message, kind: "failed" }]);
      return;
    }

    if (kind.includes("finished") || kind.includes("completed")) {
      setSending(false);
      setPaused(false);
      setStatus("");
      const sidToRefresh = activeSessionRef.current;
      if (sidToRefresh) void loadMessagesRef.current(sidToRefresh);
    }
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let closed = false;

    async function subscribe() {
      try {
        unlisten = await onAgentEvent((env) => {
          if (!closed) handleAgentEvent(env);
        });
      } catch (err) {
        setError(`订阅 agent-event 失败：${errorText(err)}`);
      }
    }

    void subscribe();
    return () => {
      closed = true;
      if (unlisten) unlisten();
    };
  }, [handleAgentEvent]);

  useEffect(() => {
    const stream = document.querySelector(".stream");
    if (stream) stream.scrollTo({ top: stream.scrollHeight, behavior: "smooth" });
  }, [messages, tools, gates, status, error]);

  const openSession = useCallback(
    async (sessionId: string) => {
      if (sessionId === activeSessionId) return;
      setActiveSessionId(sessionId);
      setStatus("");
      setError(null);
      setPaused(false);
      await loadMessagesForSession(sessionId);
    },
    [activeSessionId, loadMessagesForSession],
  );

  const newSession = useCallback(async () => {
    setError(null);
    setStatus("正在创建新对话...");
    try {
      const session = await createSession("新对话", null);
      setSessions((current) => [session, ...current]);
      setActiveSessionId(session.id);
      setMessages([]);
      setTools([]);
      setGates([]);
      setStatus("");
    } catch (err) {
      setStatus("");
      setError(`创建会话失败：${errorText(err)}`);
    }
  }, []);

  const ensureSession = useCallback(
    async (message: string) => {
      if (activeSessionId) return activeSessionId;
      const session = await createSession(message.slice(0, 28) || "新对话", null);
      setSessions((current) => [session, ...current]);
      setActiveSessionId(session.id);
      return session.id;
    },
    [activeSessionId],
  );

  const send = useCallback(async () => {
    const text = input.trim();
    if (!text || sending) return;

    setInput("");
    setModeMenuOpen(false);
    setError(null);
    setStatus("正在提交给 Atlas...");
    setSending(true);
    setPaused(false);

    try {
      const sessionId = await ensureSession(text);
      setMessages((current) => [...current, { id: id("user"), role: "user", content: text, createdAt: new Date().toISOString() }]);
      await agentChat({ sessionId, message: text, mode });
      await refreshSessions();
      await loadMessagesForSession(sessionId);
      setStatus("");
    } catch (err) {
      const friendly = friendlyChatError(err);
      setError(friendly);
      setMessages((current) => [
        ...current,
        { id: id("assistant-error"), role: "assistant", content: friendly, error: true, createdAt: new Date().toISOString() },
      ]);
      setStatus("");
    } finally {
      setSending(false);
      setPaused(false);
    }
  }, [ensureSession, input, loadMessagesForSession, mode, refreshSessions, sending]);

  const controlRun = useCallback(async (action: "pause" | "resume" | "cancel") => {
    const sessionId = activeSessionRef.current;
    if (!sessionId) {
      setError("当前没有可控制的会话。请先创建或选择一个会话。");
      return;
    }

    setError(null);
    try {
      if (action === "pause") {
        await pauseAgentChat(sessionId);
        setPaused(true);
        setStatus("已发送暂停请求");
      } else if (action === "resume") {
        await resumeAgentChat(sessionId);
        setPaused(false);
        setSending(true);
        setStatus("已发送恢复请求");
      } else {
        await cancelAgentChat(sessionId);
        setPaused(false);
        setSending(false);
        setStatus("已发送取消请求");
      }
    } catch (err) {
      setError(`${action === "pause" ? "暂停" : action === "resume" ? "恢复" : "取消"}失败：${errorText(err)}`);
    }
  }, []);

  const keyDown = useCallback(
    (event: KeyboardEvent<HTMLTextAreaElement>) => {
      if (event.key === "Enter" && !event.shiftKey) {
        event.preventDefault();
        void send();
      }
    },
    [send],
  );

  return (
    <>
      {navTarget &&
        createPortal(
          <BackendNav
            activeSessionId={activeSessionId}
            loading={loading}
            projects={projects}
            sessions={sessions}
            onNewSession={newSession}
            onOpenPage={onOpenPage}
            onOpenSearch={onOpenSearch}
            onOpenSession={openSession}
          />,
          navTarget,
        )}
      {threadTarget &&
        createPortal(
          <ConversationThread
            error={error}
            gates={gates}
            loading={loading || messagesLoading}
            messages={messages}
            status={status}
            tools={tools}
          />,
          threadTarget,
        )}
      {composerTarget &&
        createPortal(
          <Composer
            canControl={Boolean(activeSessionId)}
            input={input}
            mode={mode}
            modeMenuOpen={modeMenuOpen}
            paused={paused}
            sending={sending}
            status={status}
            onControl={controlRun}
            onInput={setInput}
            onKeyDown={keyDown}
            onSend={send}
            onSetMode={(next) => {
              setMode(next);
              setModeMenuOpen(false);
            }}
            onToggleModeMenu={() => setModeMenuOpen((current) => !current)}
          />,
          composerTarget,
        )}
    </>
  );
}

function BackendNav({
  sessions,
  projects,
  activeSessionId,
  loading,
  onNewSession,
  onOpenSearch,
  onOpenPage,
  onOpenSession,
}: {
  sessions: SessionRecord[];
  projects: ProjectRecord[];
  activeSessionId: string | null;
  loading: boolean;
  onNewSession: () => void;
  onOpenSearch: () => void;
  onOpenPage: (page: PageKey) => void;
  onOpenSession: (sessionId: string) => void;
}) {
  const sessionsByProject = useMemo(() => {
    const map = new Map<string, SessionRecord[]>();
    for (const session of sessions) {
      if (!session.project_id) continue;
      const list = map.get(session.project_id) ?? [];
      list.push(session);
      map.set(session.project_id, list);
    }
    return map;
  }, [sessions]);
  const looseSessions = sessions.filter((session) => !session.project_id);

  return (
    <div className="react-nav-content">
      <div className="nav-section">
        <button className="nav-item" onClick={onNewSession}>
          <Icon><path d="M4 6.5A2.5 2.5 0 0 1 6.5 4h7A2.5 2.5 0 0 1 16 6.5v4A2.5 2.5 0 0 1 13.5 13H8l-3.2 2.6a.5.5 0 0 1-.8-.4Z" /><line x1="10" y1="6.6" x2="10" y2="10.4" /><line x1="8.1" y1="8.5" x2="11.9" y2="8.5" /></Icon>
          <span className="label">新对话</span>
        </button>
        <button className="nav-item" onClick={onOpenSearch}>
          <Icon><circle cx="9" cy="9" r="5" /><line x1="12.8" y1="12.8" x2="16" y2="16" /></Icon>
          <span className="label">搜索</span>
        </button>
      </div>

      <div className="nav-section">
        <button className="nav-item" onClick={() => onOpenPage("knowledge")}>
          <Icon><path d="M5 4.5h7l3 3v8a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5.5a1 1 0 0 1 1-1Z" /><path d="M11.5 4.5V8H15" /></Icon>
          <span className="label">知识库</span>
        </button>
        <button className="nav-item" onClick={() => onOpenPage("plugins")}>
          <Icon><path d="M8 4h4v2.2a1 1 0 0 0 1.5.9 2 2 0 1 1 0 3.4 1 1 0 0 0-1.5.9V16H8v-2.4a1 1 0 0 0-1.5-.9 2 2 0 1 1 0-3.4A1 1 0 0 0 8 8.4Z" /></Icon>
          <span className="label">插件</span>
        </button>
        <button className="nav-item" onClick={() => onOpenPage("evals")}>
          <Icon><path d="M4 16V9" /><path d="M8 16V5" /><path d="M12 16v-5" /><path d="M16 16V7" /></Icon>
          <span className="label">防线测试</span>
        </button>
      </div>

      <div className="nav-group">项目 <span className="add"><Icon small><line x1="10" y1="5" x2="10" y2="15" /><line x1="5" y1="10" x2="15" y2="10" /></Icon></span></div>
      <div className="nav-section">
        {loading && <div className="nav-empty">正在读取项目...</div>}
        {!loading && projects.length === 0 && <div className="nav-empty">暂无项目</div>}
        {projects.map((project) => {
          const projectSessions = sessionsByProject.get(project.id) ?? [];
          return (
            <div className="proj open" data-proj key={project.id}>
              <button className="nav-item proj-toggle">
                <Icon small><path d="M8 6l4 4-4 4" /></Icon>
                <Icon><path d="M3.5 6.5A1.5 1.5 0 0 1 5 5h2.6l1.3 1.4H15a1.5 1.5 0 0 1 1.5 1.5v6A1.5 1.5 0 0 1 15 15.5H5a1.5 1.5 0 0 1-1.5-1.5Z" /></Icon>
                <span className="label">{project.title || project.root_path || "未命名项目"}</span>
              </button>
              <div className="proj-children">
                {projectSessions.length === 0 && <div className="nav-empty nested">暂无对话</div>}
                {projectSessions.map((session) => (
                  <button
                    className={`nav-item ${session.id === activeSessionId ? "selected" : ""}`}
                    key={session.id}
                    onClick={() => onOpenSession(session.id)}
                  >
                    <span className="dot" />
                    <span className="label">{session.title || "未命名对话"}</span>
                  </button>
                ))}
              </div>
            </div>
          );
        })}
      </div>

      <div className="nav-group">对话</div>
      <div className="nav-section">
        {loading && <div className="nav-empty">正在读取会话...</div>}
        {!loading && looseSessions.length === 0 && <div className="nav-empty">暂无独立对话</div>}
        {looseSessions.map((session) => (
          <button
            className={`nav-item ${session.id === activeSessionId ? "selected" : ""}`}
            key={session.id}
            onClick={() => onOpenSession(session.id)}
          >
            <span className="label">{session.title || "未命名对话"}</span>
            <span className="time">{relativeTime(session.last_active_at ?? session.updated_at ?? session.created_at)}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function ConversationThread({
  messages,
  tools,
  gates,
  loading,
  error,
  status,
}: {
  messages: UiMessage[];
  tools: ToolActivity[];
  gates: GateNotice[];
  loading: boolean;
  error: string | null;
  status: string;
}) {
  const hasContent = messages.length > 0 || tools.length > 0 || gates.length > 0 || error;

  return (
    <div className="react-thread-content">
      {loading && (
        <div className="msg-ai">
          <div className="ai-body"><p className="muted-line">正在读取真实会话数据...</p></div>
        </div>
      )}

      {!loading && !hasContent && (
        <div className="msg-ai backend-empty-state">
          <div className="ai-body">
            <p>当前会话还没有消息。</p>
            <p className="muted-line">在下方输入指令后，Atlas 会通过真实 <code className="inl">agent_chat_v2</code> 路径发送。</p>
          </div>
        </div>
      )}

      {messages.map((message) =>
        message.role === "user" ? (
          <div className="msg-user" key={message.id}>
            <div className="bubble">{message.content}</div>
            <div className="msg-stamp">{formatStamp(message.createdAt)}</div>
          </div>
        ) : (
          <div className={`msg-ai ${message.error ? "backend-message-error" : ""}`} key={message.id}>
            <div className="ai-body">{renderParagraphs(message.content)}</div>
          </div>
        ),
      )}

      {tools.map((tool) => (
        <div className={`tool open backend-tool-${tool.status}`} data-tool key={tool.id}>
          <button className="tool-bar">
            <Icon small><path d="M5 7l3 3-3 3" /><line x1="10" y1="14" x2="15" y2="14" /></Icon>
            <span>{tool.status === "running" ? "正在运行" : tool.status === "success" ? "已运行" : "运行失败"} · {tool.title}</span>
          </button>
          <div className="tool-panel">
            <div className="tool-head"><span className="tag">Tool</span><span>{tool.detail || "真实 agent-event"}</span></div>
            {tool.output && <div className="tool-out">{tool.output}</div>}
            <div className="tool-foot"><span className={tool.status === "failed" ? "fail" : "ok"}>{tool.status === "running" ? "运行中" : tool.status === "success" ? "成功" : "失败"}</span></div>
          </div>
        </div>
      ))}

      {gates.map((gate) => (
        <div className={`gate backend-gate backend-gate-${gate.kind ?? "blocked"}`} key={gate.id}>
          <div className="gate-inner">
            <div className="gate-top">
              <span className="badge">
                <Icon><circle cx="10" cy="10" r="6.5" /><line x1="5.4" y1="5.4" x2="14.6" y2="14.6" /></Icon>
                {gate.kind === "failed" ? "失败" : gate.kind === "cancelled" ? "已取消" : "已拦截"}
              </span>
              <span className="gate-title">{gate.title}</span>
            </div>
            <div className="gate-body">
              <div>
                <div className="gate-rule"><span className="ctab">event</span><span className="rule-text">{gate.message}</span></div>
                {gate.detail && <div className="gate-why">{gate.detail}</div>}
              </div>
            </div>
          </div>
        </div>
      ))}

      {error && (
        <div className="gate backend-gate backend-gate-error">
          <div className="gate-inner">
            <div className="gate-top">
              <span className="badge">
                <Icon><circle cx="10" cy="10" r="6.5" /><line x1="5.4" y1="5.4" x2="14.6" y2="14.6" /></Icon>
                错误
              </span>
              <span className="gate-title">主路径调用失败</span>
            </div>
            <div className="gate-body">
              <div className="gate-rule"><span className="ctab">backend</span><span className="rule-text backend-preserve-lines">{error}</span></div>
            </div>
          </div>
        </div>
      )}

      {status && !error && (
        <div className="msg-ai backend-status-line">
          <div className="ai-body"><p className="muted-line">{status}</p></div>
        </div>
      )}
    </div>
  );
}

function Composer({
  input,
  mode,
  modeMenuOpen,
  sending,
  paused,
  status,
  canControl,
  onInput,
  onSend,
  onKeyDown,
  onToggleModeMenu,
  onSetMode,
  onControl,
}: {
  input: string;
  mode: AgentMode;
  modeMenuOpen: boolean;
  sending: boolean;
  paused: boolean;
  status: string;
  canControl: boolean;
  onInput: (value: string) => void;
  onSend: () => void;
  onKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  onToggleModeMenu: () => void;
  onSetMode: (mode: AgentMode) => void;
  onControl: (action: "pause" | "resume" | "cancel") => void;
}) {
  return (
    <div className="react-composer-content">
      <div className="composer">
        {(sending || paused || status) && (
          <div className="backend-runbar">
            <span className="runbar-status"><span className="dot" />{status || (paused ? "已暂停" : "运行中")}</span>
            <span className="runbar-spacer" />
            <button className="btn btn-ghost btn-sm" disabled={!canControl || paused} onClick={() => onControl("pause")}>暂停</button>
            <button className="btn btn-ghost btn-sm" disabled={!canControl || !paused} onClick={() => onControl("resume")}>恢复</button>
            <button className="btn btn-block btn-sm" disabled={!canControl} onClick={() => onControl("cancel")}>取消</button>
          </div>
        )}
        <textarea
          className="composer-input"
          rows={1}
          placeholder="给 Atlas 指令..."
          value={input}
          onChange={(event) => onInput(event.target.value)}
          onKeyDown={onKeyDown}
        />
        <div className="composer-bar">
          <button className="comp-btn" title="附加" aria-label="附加" type="button">
            <Icon><line x1="10" y1="5" x2="10" y2="15" /><line x1="5" y1="10" x2="15" y2="10" /></Icon>
          </button>
          <span className="comp-spacer" />
          <div className="mode-wrap">
            <button className="mode" type="button" onClick={onToggleModeMenu} title={`当前：${modeLabels[mode]} · ${mode}`}>
              <span className="dot" />{modeLabels[mode]}
              <Icon small><path d="M6 8l4 4 4-4" /></Icon>
            </button>
            {modeMenuOpen && (
              <div className="mode-menu">
                {(["chat", "plan", "review"] as AgentMode[]).map((item) => (
                  <button className={`mode-choice ${item === mode ? "active" : ""}`} key={item} type="button" onClick={() => onSetMode(item)}>
                    <span>{modeLabels[item]}</span>
                    <small>{modeHints[item]} · {item}</small>
                  </button>
                ))}
              </div>
            )}
          </div>
          <button className="send" title="发送" aria-label="发送" type="button" disabled={!input.trim() || sending} onClick={onSend}>
            <svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
              <line x1="10" y1="15.5" x2="10" y2="5" />
              <path d="M5.5 9.5 10 5l4.5 4.5" />
            </svg>
          </button>
        </div>
      </div>
    </div>
  );
}
