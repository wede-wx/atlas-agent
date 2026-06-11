import { useEffect, useMemo, useRef, useState } from "react";
import {
  agentChat,
  getMessages,
  onAgentEvent,
} from "../../bridge";
import { EmptyState } from "../../components/EmptyState";
import { useAppState } from "../../state/AppState";
import type { AgentEvent, AgentRunEvent, MessageRecord } from "../../types";

interface ChatViewProps {
  onRunChange: (runId: string | null) => void;
  onOpenDrawer: () => void;
}

interface ChatItem {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  createdAt: number;
  live?: boolean;
}

interface OperationItem {
  id: string;
  tool: string;
  label: string;
  detail?: string;
  output: string[];
  status: "running" | "done" | "failed";
  summary?: string;
  open: boolean;
}

interface GateItem {
  id: string;
  kind: "blocked" | "unverified" | "failed";
  title: string;
  body: string;
}

export function ChatView({ onRunChange, onOpenDrawer }: ChatViewProps) {
  const { activeSessionId, createNewSession, refreshSessions, prefs } = useAppState();
  const [messages, setMessages] = useState<ChatItem[]>([]);
  const [input, setInput] = useState("");
  const [status, setStatus] = useState("");
  const [running, setRunning] = useState(false);
  const [operations, setOperations] = useState<OperationItem[]>([]);
  const [gates, setGates] = useState<GateItem[]>([]);
  const [error, setError] = useState<string | null>(null);
  const messageMap = useRef<Map<string, string>>(new Map());
  const activeSessionRef = useRef<string | null>(activeSessionId);

  useEffect(() => {
    activeSessionRef.current = activeSessionId;
  }, [activeSessionId]);

  useEffect(() => {
    let mounted = true;
    async function load() {
      setError(null);
      setOperations([]);
      setGates([]);
      messageMap.current.clear();
      if (!activeSessionId) {
        setMessages([]);
        return;
      }
      try {
        const rows = await getMessages(activeSessionId);
        if (!mounted) return;
        setMessages(rows.map(toChatItem));
      } catch (err) {
        if (mounted) setError(String(err));
      }
    }
    void load();
    return () => {
      mounted = false;
    };
  }, [activeSessionId]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    void onAgentEvent((env) => {
      if (env.sessionId && activeSessionRef.current && env.sessionId !== activeSessionRef.current) return;
      if (env.runId) onRunChange(env.runId);
      handleAgentEvent(env.event);
    }).then((off) => {
      unlisten = off;
    }).catch((err) => setError(String(err)));
    return () => {
      if (unlisten) unlisten();
    };
  }, [onRunChange]);

  function handleAgentEvent(event: AgentEvent) {
    switch (event.type) {
      case "Thinking":
        setStatus(event.content);
        break;
      case "ResponseStarted":
        messageMap.current.set(event.message_id, event.message_id);
        setMessages((rows) => [...rows, { id: event.message_id, role: "assistant", content: "", createdAt: Date.now(), live: true }]);
        break;
      case "ResponseDelta":
        appendAssistantDelta(event.message_id, event.content);
        break;
      case "ResponseCompleted":
      case "Response":
        setAssistantContent(event.message_id, event.content, false);
        break;
      case "ResponseFallbackStarted":
        setStatus(`正在切换模型：${event.reason}`);
        break;
      case "OperationStarted":
        setOperations((rows) => [...rows, { id: event.operation_id, tool: event.tool_name, label: event.label, detail: event.target ?? event.command ?? event.detail ?? undefined, output: [], status: "running", open: false }]);
        break;
      case "OperationOutput":
        setOperations((rows) => rows.map((op) => op.id === event.operation_id ? { ...op, output: [...op.output, event.content] } : op));
        break;
      case "OperationFinished":
        setOperations((rows) => rows.map((op) => op.id === event.operation_id ? { ...op, status: "done", summary: event.summary } : op));
        break;
      case "OperationFailed":
        setOperations((rows) => rows.map((op) => op.id === event.operation_id ? { ...op, status: "failed", summary: event.summary, open: true } : op));
        break;
      case "SubAgentStarted":
        setOperations((rows) => [...rows, { id: event.subagent_id, tool: "subagent", label: `已创建子代理 ${event.name}`, detail: event.task, output: [event.description], status: "running", open: true }]);
        break;
      case "SubAgentFinished":
        setOperations((rows) => rows.map((op) => op.id === event.subagent_id ? { ...op, status: "done", summary: event.summary } : op));
        break;
      case "SubAgentFailed":
        setOperations((rows) => rows.map((op) => op.id === event.subagent_id ? { ...op, status: "failed", summary: event.error, open: true } : op));
        break;
      case "UnknownToolRequested":
        setStatus(`后端拒绝未知工具：${event.requested}`);
        break;
      case "FinalAudit":
        setStatus("最终审计已返回，详见运行抽屉。");
        break;
      case "RunEvent":
        handleRunEvent(event.event);
        break;
      default:
        break;
    }
  }

  function handleRunEvent(event: AgentRunEvent) {
    if ("run_id" in event && event.run_id) onRunChange(event.run_id);
    switch (event.type) {
      case "Started":
        setRunning(true);
        if (prefs.general.openDrawerOnRun) onOpenDrawer();
        break;
      case "Blocked":
        setGates((rows) => [...rows, { id: `${event.run_id}-${rows.length}`, kind: event.status === "unverified" ? "unverified" : "blocked", title: event.status === "unverified" ? "完成被标记为未验证" : "完成被拦截", body: event.footer }]);
        setRunning(false);
        break;
      case "Failed":
        setGates((rows) => [...rows, { id: `${event.run_id}-failed-${rows.length}`, kind: "failed", title: "运行失败", body: event.error }]);
        setRunning(false);
        break;
      case "Finished":
        setRunning(false);
        setStatus("");
        if (activeSessionRef.current) void refreshSessions();
        break;
      case "Paused":
        setStatus("运行已在安全边界暂停。");
        break;
      case "Resumed":
        setStatus("运行已恢复。");
        break;
      case "Cancelled":
        setRunning(false);
        setStatus("运行已取消。");
        break;
      case "GuidanceQueued":
      case "GuidanceMerged":
        setStatus(`运行中追加指令：${event.count} 条`);
        break;
      default:
        break;
    }
  }

  function appendAssistantDelta(messageId: string, delta: string) {
    setMessages((rows) => {
      if (!rows.some((row) => row.id === messageId)) {
        return [...rows, { id: messageId, role: "assistant", content: delta, createdAt: Date.now(), live: true }];
      }
      return rows.map((row) => row.id === messageId ? { ...row, content: row.content + delta, live: true } : row);
    });
  }

  function setAssistantContent(messageId: string, content: string, live: boolean) {
    setMessages((rows) => {
      if (!rows.some((row) => row.id === messageId)) {
        return [...rows, { id: messageId, role: "assistant", content, createdAt: Date.now(), live }];
      }
      return rows.map((row) => row.id === messageId ? { ...row, content, live } : row);
    });
  }

  async function send() {
    const text = input.trim();
    if (!text || running) return;
    setError(null);
    setInput("");
    setOperations([]);
    setGates([]);
    let sessionId = activeSessionId;
    if (!sessionId) {
      const created = await createNewSession(text.slice(0, 42) || "New session");
      sessionId = created?.id ?? null;
    }
    if (!sessionId) {
      setError("无法创建真实会话，未发送。请检查后端。 ");
      return;
    }
    setMessages((rows) => [...rows, { id: `local-${Date.now()}`, role: "user", content: text, createdAt: Date.now() }]);
    setRunning(true);
    try {
      await agentChat({ sessionId, message: text, mode: "chat" });
      const fresh = await getMessages(sessionId);
      setMessages(fresh.map(toChatItem));
    } catch (err) {
      setError(String(err));
      setGates((rows) => [...rows, { id: `error-${Date.now()}`, kind: "failed", title: "发送失败", body: String(err) }]);
    } finally {
      setRunning(false);
    }
  }

  const groupedOps = useMemo(() => operations.slice().reverse(), [operations]);

  return (
    <section className="chat-view main">
      {error ? <div className="error-box">{error}</div> : null}
      <div className="stream message-stream">
        <div className="thread">
          {messages.length === 0 && gates.length === 0 && operations.length === 0 ? <EmptyState title="没有对话" body="创建或选择一个会话，然后发送第一条真实消息。" /> : null}
          {messages.map((message) => <MessageBubble key={message.id} item={message} />)}
          {groupedOps.map((op) => <OperationCard key={op.id} item={op} onToggle={() => setOperations((rows) => rows.map((row) => row.id === op.id ? { ...row, open: !row.open } : row))} />)}
          {gates.map((gate) => <GateCard key={gate.id} gate={gate} onOpenDrawer={onOpenDrawer} />)}
        </div>
      </div>
      <div className="composer-wrap">
        <div className="composer">
          {status ? <div className="status-line"><span className="pulse" />{status}</div> : null}
          <textarea className="composer-input" rows={1} value={input} onChange={(event) => setInput(event.target.value)} onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              void send();
            }
          }} placeholder="给 Atlas 指令…" />
          <div className="composer-bar">
            <span className="comp-btn comp-static" title="附件入口按设计稿显示；当前发送仍不提交附件。" aria-label="附加">
              <span aria-hidden="true">＋</span>
            </span>
            <span className="comp-spacer" />
            <span className="mode mode-static" title="后端 agent_chat_v2 mode=chat">
              <span className="dot" />
              默认模式
              <span className="ico-sm" aria-hidden="true">⌄</span>
            </span>
            <button className="send" title="发送" aria-label="发送" type="button" disabled={running || !input.trim()} onClick={() => void send()}>
              <span aria-hidden="true">{running ? "…" : "↑"}</span>
            </button>
          </div>
        </div>
      </div>
    </section>
  );
}

function toChatItem(row: MessageRecord): ChatItem {
  return { id: row.id, role: row.role === "user" ? "user" : row.role === "system" ? "system" : "assistant", content: row.content, createdAt: row.created_at };
}

function MessageBubble({ item }: { item: ChatItem }) {
  if (item.role === "user") {
    return (
      <article className="msg-user message user">
        <div className="bubble message-body">{item.content}</div>
        <div className="msg-stamp">{new Date(item.createdAt).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}</div>
      </article>
    );
  }
  return (
    <article className={`msg-ai message ${item.role}`}>
      <div className="ai-body message-body">{item.content || (item.live ? "正在生成…" : "")}</div>
    </article>
  );
}

function OperationCard({ item, onToggle }: { item: OperationItem; onToggle: () => void }) {
  return (
    <article className={`operation-card ${item.status}`}>
      <button type="button" className="operation-head" onClick={onToggle}>
        <span className="tag blue">{item.tool}</span>
        <span>{item.label}</span>
        <span className="subtle">{item.summary ?? item.detail ?? item.status}</span>
      </button>
      {item.open ? <pre className="code-panel">{[item.detail, ...item.output, item.summary].filter(Boolean).join("\n") || "暂无输出。"}</pre> : null}
    </article>
  );
}

function GateCard({ gate, onOpenDrawer }: { gate: GateItem; onOpenDrawer: () => void }) {
  return (
    <article className={`gate-card ${gate.kind}`}>
      <div className="gate-stripe" />
      <div className="gate-content">
        <div className="gate-kicker">FOUR-GATE INTERCEPT</div>
        <h3>{gate.title}</h3>
        <pre>{gate.body || "后端没有返回更多细节。"}</pre>
        <div className="gate-actions">
          <button className="primary-action" type="button" onClick={onOpenDrawer}>查看运行证据</button>
          <button className="ghost-button" type="button" disabled>需要权限裁决记录才可放行</button>
        </div>
      </div>
    </article>
  );
}
