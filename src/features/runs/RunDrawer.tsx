import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import {
  getAgentGraphNodeTraces,
  getAgentGraphSnapshot,
  getAgentPermissionDecisions,
  getAgentRunAudit,
  getAgentRunDiff,
  getAgentRunProgress,
  getAgentRuns,
  getAgentRunTerminal,
  getAgentRunTimeline,
  resolvePermissionConfirmation,
} from "../../bridge";
import type {
  AgentGraphSnapshot,
  AgentRunRecord,
  PermissionDecisionRecord,
  RunProgressSummary,
  RunTimelineEntry,
  UnknownRecord,
  WorkflowTraceReport,
} from "../../types";

type DrawerTab = "contract" | "timeline" | "graph";
type LoadState = "idle" | "loading" | "ready" | "error";

const RUN_LIMIT = 20;
const TIMELINE_LIMIT = 30;
const AUDIT_LIMIT = 50;
const TERMINAL_LIMIT = 80;
const DIFF_LIMIT = 40;

function Icon({ children, small = false }: { children: ReactNode; small?: boolean }) {
  return (
    <svg className={`ico${small ? " ico-sm" : ""}`} viewBox="0 0 20 20" aria-hidden="true">
      {children}
    </svg>
  );
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

function isRecord(value: unknown): value is UnknownRecord {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

function textValue(value: unknown, fallback = ""): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return fallback;
}

function numberValue(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function timestamp(value: unknown): number | null {
  const raw = numberValue(value);
  if (raw == null) return null;
  return raw < 10_000_000_000 ? raw * 1000 : raw;
}

function formatStamp(value: unknown): string {
  const time = timestamp(value);
  if (!time) return "";
  const date = new Date(time);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString([], {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function shortId(value: string | null | undefined): string {
  if (!value) return "";
  if (value.length <= 16) return value;
  return `${value.slice(0, 8)}...${value.slice(-6)}`;
}

function safeJson(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function arrayFrom(value: unknown): UnknownRecord[] {
  if (Array.isArray(value)) return value.map((item) => (isRecord(item) ? item : { value: item }));
  return [];
}

function recordsFrom(value: unknown): UnknownRecord[] {
  if (Array.isArray(value)) return arrayFrom(value);
  if (!isRecord(value)) return [];

  for (const key of ["entries", "records", "items", "lines", "diffs", "chunks", "events", "traces"]) {
    const nested = value[key];
    if (Array.isArray(nested)) return arrayFrom(nested);
  }

  return Object.keys(value).length ? [value] : [];
}

function runTime(run: AgentRunRecord): number {
  return timestamp(run.updated_at) ?? timestamp(run.created_at) ?? 0;
}

function latestRun(runs: AgentRunRecord[]): AgentRunRecord | null {
  return [...runs].sort((a, b) => runTime(b) - runTime(a))[0] ?? null;
}

function nestedRecord(source: UnknownRecord, key: string): UnknownRecord | null {
  const value = source[key];
  return isRecord(value) ? value : null;
}

function findStringField(source: UnknownRecord, keys: string[], depth = 0): string | null {
  for (const key of keys) {
    const value = source[key];
    if (typeof value === "string" && value.trim()) return value;
  }
  if (depth >= 2) return null;

  for (const key of ["metadata", "detail", "details", "graph", "graph_run", "workflow", "context"]) {
    const nested = nestedRecord(source, key);
    if (!nested) continue;
    const found = findStringField(nested, keys, depth + 1);
    if (found) return found;
  }
  return null;
}

function graphRunIdFromRun(run: AgentRunRecord | null): string | null {
  if (!run) return null;
  return findStringField(run, [
    "graph_run_id",
    "graphRunId",
    "graph_id",
    "graphId",
    "workflow_run_id",
    "workflowRunId",
    "agent_graph_run_id",
    "agentGraphRunId",
  ]);
}

function cssToken(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "") || "unknown";
}

function EmptyState({ title, detail }: { title: string; detail?: string }) {
  return (
    <div className="run-empty">
      <div className="run-empty-title">{title}</div>
      {detail ? <div className="run-empty-detail">{detail}</div> : null}
    </div>
  );
}

function ErrorCard({ title, error }: { title: string; error: string }) {
  return (
    <div className="gate run-error">
      <div className="gate-inner">
        <div className="gate-top">
          <span className="badge">
            <Icon><circle cx="10" cy="10" r="6.5" /><line x1="5.4" y1="5.4" x2="14.6" y2="14.6" /></Icon>
            错误
          </span>
          <span className="gate-title">{title}</span>
        </div>
        <div className="gate-rule">
          <span className="ctab">backend</span>
          <span className="rule-text backend-preserve-lines">{error}</span>
        </div>
      </div>
    </div>
  );
}

function RawFeed({ title, value, emptyText }: { title: string; value: unknown; emptyText: string }) {
  const rows = recordsFrom(value);
  if (rows.length === 0) {
    return (
      <section className="run-section">
        <div className="run-section-head">{title}</div>
        <div className="run-muted">{emptyText}</div>
      </section>
    );
  }

  return (
    <section className="run-section">
      <div className="run-section-head">{title}</div>
      <div className="run-raw-list">
        {rows.map((row, index) => (
          <pre className="run-raw" key={`${title}-${index}`}>{safeJson(row)}</pre>
        ))}
      </div>
    </section>
  );
}

export function RunDrawer({ target, sessionId }: { target: HTMLElement; sessionId: string | null }) {
  const [tab, setTab] = useState<DrawerTab>("contract");
  const [runs, setRuns] = useState<AgentRunRecord[]>([]);
  const [state, setState] = useState<LoadState>("idle");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function loadRuns() {
      if (!sessionId) {
        setRuns([]);
        setError(null);
        setState("idle");
        return;
      }

      setState("loading");
      setError(null);
      try {
        const records = await getAgentRuns(sessionId, RUN_LIMIT);
        if (cancelled) return;
        setRuns(records);
        setState("ready");
      } catch (err) {
        if (cancelled) return;
        setRuns([]);
        setError(errorText(err));
        setState("error");
      }
    }

    void loadRuns();
    return () => {
      cancelled = true;
    };
  }, [sessionId]);

  const currentRun = useMemo(() => latestRun(runs), [runs]);
  const currentRunId = currentRun?.id ?? null;

  const noRun = !sessionId || (!currentRunId && state !== "loading" && state !== "error");
  const commonEmpty = !sessionId ? "请先选择或创建一个会话。" : "当前会话还没有运行记录。真实发送一条消息后这里会显示该会话最近 run。";

  return createPortal(
    <div className="run-drawer-react" onClick={(event) => event.stopPropagation()}>
      <div className="drawer-tabs">
        <button className={`dtab ${tab === "contract" ? "active" : ""}`} type="button" data-tab="contract" onClick={() => setTab("contract")}>目标契约</button>
        <button className={`dtab ${tab === "timeline" ? "active" : ""}`} type="button" data-tab="timeline" onClick={() => setTab("timeline")}>运行时间线</button>
        <button className={`dtab ${tab === "graph" ? "active" : ""}`} type="button" data-tab="graph" onClick={() => setTab("graph")}>运行图</button>
      </div>

      <div className="drawer-scroll">
        <div className="drawer-inner">
          {state === "loading" ? <EmptyState title="正在读取运行记录..." detail="通过 get_agent_runs 拉取当前会话最近 run。" /> : null}
          {state === "error" && error ? <ErrorCard title="读取运行记录失败" error={error} /> : null}
          {state !== "loading" && state !== "error" && noRun ? <EmptyState title="当前没有可展示的 run" detail={commonEmpty} /> : null}
          {state === "ready" && currentRun ? <RunHeader run={currentRun} /> : null}

          {state === "ready" && currentRunId && tab === "contract" ? <ContractTab runId={currentRunId} /> : null}
          {state === "ready" && currentRunId && tab === "timeline" ? <TimelineTab runId={currentRunId} /> : null}
          {state === "ready" && currentRun && tab === "graph" ? <GraphTab run={currentRun} /> : null}
        </div>
      </div>
    </div>,
    target,
  );
}

function RunHeader({ run }: { run: AgentRunRecord }) {
  const status = textValue(run.status, "unknown");
  return (
    <section className="run-current">
      <div>
        <div className="run-kicker">当前 run</div>
        <div className="run-id">{shortId(run.id)}</div>
      </div>
      <span className={`run-pill tone-${cssToken(status)}`}>{status}</span>
      <div className="run-meta">
        {formatStamp(run.created_at) ? <span>创建 {formatStamp(run.created_at)}</span> : null}
        {formatStamp(run.updated_at) ? <span>更新 {formatStamp(run.updated_at)}</span> : null}
      </div>
    </section>
  );
}

function TimelineTab({ runId }: { runId: string }) {
  const [progress, setProgress] = useState<RunProgressSummary | null>(null);
  const [entries, setEntries] = useState<RunTimelineEntry[]>([]);
  const [total, setTotal] = useState(0);
  const [diff, setDiff] = useState<unknown>(null);
  const [terminal, setTerminal] = useState<unknown>(null);
  const [state, setState] = useState<LoadState>("loading");
  const [moreLoading, setMoreLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function loadInitial() {
      setState("loading");
      setError(null);
      setEntries([]);
      setTotal(0);
      setProgress(null);
      setDiff(null);
      setTerminal(null);

      const [progressResult, timelineResult, diffResult, terminalResult] = await Promise.allSettled([
        getAgentRunProgress(runId),
        getAgentRunTimeline(runId, TIMELINE_LIMIT, 0),
        getAgentRunDiff(runId, DIFF_LIMIT),
        getAgentRunTerminal(runId, TERMINAL_LIMIT, 0),
      ]);

      if (cancelled) return;

      if (progressResult.status === "fulfilled") setProgress(progressResult.value);
      if (diffResult.status === "fulfilled") setDiff(diffResult.value);
      if (terminalResult.status === "fulfilled") setTerminal(terminalResult.value);

      if (timelineResult.status === "fulfilled") {
        setEntries(timelineResult.value.entries ?? []);
        setTotal(timelineResult.value.total ?? timelineResult.value.entries?.length ?? 0);
        setState("ready");
        const softErrors = [progressResult, diffResult, terminalResult]
          .filter((item): item is PromiseRejectedResult => item.status === "rejected")
          .map((item) => errorText(item.reason));
        setError(softErrors.length ? `部分运行数据读取失败：${softErrors.join("；")}` : null);
      } else {
        setState("error");
        setError(errorText(timelineResult.reason));
      }
    }

    void loadInitial();
    return () => {
      cancelled = true;
    };
  }, [runId]);

  const loadMore = useCallback(async () => {
    if (moreLoading || entries.length >= total) return;
    setMoreLoading(true);
    setError(null);
    try {
      const next = await getAgentRunTimeline(runId, TIMELINE_LIMIT, entries.length);
      setEntries((current) => [...current, ...(next.entries ?? [])]);
      setTotal(next.total ?? total);
    } catch (err) {
      setError(`加载更多时间线失败：${errorText(err)}`);
    } finally {
      setMoreLoading(false);
    }
  }, [entries.length, moreLoading, runId, total]);

  if (state === "loading") return <EmptyState title="正在读取运行时间线..." detail="get_agent_run_timeline / get_agent_run_progress" />;
  if (state === "error" && error) return <ErrorCard title="读取运行时间线失败" error={error} />;

  return (
    <div className="run-tab">
      {error ? <ErrorCard title="运行时间线部分数据失败" error={error} /> : null}
      <ProgressOverview progress={progress} />
      <section className="run-section">
        <div className="run-section-head">
          <span>时间线</span>
          <span className="run-count">{entries.length}/{total}</span>
        </div>
        {entries.length === 0 ? <div className="run-muted">后端没有返回时间线条目。</div> : null}
        <div className="run-timeline">
          {entries.map((entry, index) => (
            <TimelineEntryView entry={entry} key={`${entry.kind}-${entry.id}-${entry.seq}-${index}`} />
          ))}
        </div>
        {entries.length < total ? (
          <button className="run-load-more" type="button" disabled={moreLoading} onClick={loadMore}>
            {moreLoading ? "正在加载..." : "加载更多"}
          </button>
        ) : null}
      </section>
      <RawFeed title="Diff" value={diff} emptyText="后端未返回 diff 记录。" />
      <RawFeed title="Terminal" value={terminal} emptyText="后端未返回终端输出；不会补齐被后端省略的无命令工具行。" />
    </div>
  );
}

function ProgressOverview({ progress }: { progress: RunProgressSummary | null }) {
  if (!progress) {
    return (
      <section className="run-progress-card">
        <div className="run-muted">后端未返回进度概览。</div>
      </section>
    );
  }

  const semantic = isRecord(progress.semantic) ? progress.semantic : {};
  const tone = textValue(semantic.tone ?? progress.tone ?? progress.status, "unknown");

  return (
    <section className="run-progress-card">
      <div className="run-section-head">
        <span>进度概览</span>
        <span className={`run-pill tone-${cssToken(tone)}`}>{tone}</span>
      </div>
      <div className="run-progress-grid">
        <div><span>状态</span><strong>{textValue(progress.status, "unknown")}</strong></div>
        <div><span>完成步骤</span><strong>{textValue(progress.completed_steps, "0")}</strong></div>
        <div><span>失败步骤</span><strong>{textValue(progress.failed_steps, "0")}</strong></div>
      </div>
      {progress.latest_message ? <div className="run-progress-message">{textValue(progress.latest_message)}</div> : null}
      {Object.keys(semantic).length ? <pre className="run-raw compact">{safeJson(semantic)}</pre> : null}
    </section>
  );
}

function TimelineEntryView({ entry }: { entry: RunTimelineEntry }) {
  return (
    <article className={`run-timeline-item status-${cssToken(textValue(entry.status, "unknown"))}`}>
      <div className="run-timeline-dot" />
      <div className="run-timeline-body">
        <div className="run-timeline-top">
          <span className="run-timeline-kind">{entry.kind}</span>
          {entry.status ? <span className="run-muted">{entry.status}</span> : null}
        </div>
        <div className="run-timeline-title">{entry.label || entry.id || "未命名事件"}</div>
        <div className="run-timeline-meta">
          {formatStamp(entry.at) ? <span>{formatStamp(entry.at)}</span> : null}
          {entry.finished_at ? <span>完成 {formatStamp(entry.finished_at)}</span> : null}
          <span>seq {entry.seq}</span>
        </div>
        {entry.detail != null ? <pre className="run-raw compact">{safeJson(entry.detail)}</pre> : null}
      </div>
    </article>
  );
}

function ContractTab({ runId }: { runId: string }) {
  const [decisions, setDecisions] = useState<PermissionDecisionRecord[]>([]);
  const [audit, setAudit] = useState<unknown>(null);
  const [state, setState] = useState<LoadState>("loading");
  const [error, setError] = useState<string | null>(null);
  const [resolving, setResolving] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function loadContract() {
      setState("loading");
      setError(null);
      setDecisions([]);
      setAudit(null);

      const [decisionResult, auditResult] = await Promise.allSettled([
        getAgentPermissionDecisions(runId, AUDIT_LIMIT),
        getAgentRunAudit(runId, AUDIT_LIMIT, 0),
      ]);

      if (cancelled) return;

      if (decisionResult.status === "fulfilled") setDecisions(decisionResult.value);
      if (auditResult.status === "fulfilled") setAudit(auditResult.value);

      if (decisionResult.status === "rejected" && auditResult.status === "rejected") {
        setState("error");
        setError(`审批账本：${errorText(decisionResult.reason)}；审计流：${errorText(auditResult.reason)}`);
      } else {
        setState("ready");
        const softErrors = [decisionResult, auditResult]
          .filter((item): item is PromiseRejectedResult => item.status === "rejected")
          .map((item) => errorText(item.reason));
        setError(softErrors.length ? `部分契约数据读取失败：${softErrors.join("；")}` : null);
      }
    }

    void loadContract();
    return () => {
      cancelled = true;
    };
  }, [runId]);

  const resolveDecision = useCallback(async (record: PermissionDecisionRecord, approved: boolean) => {
    setResolving(record.id);
    setError(null);
    try {
      const updated = await resolvePermissionConfirmation(record, approved);
      setDecisions((current) => current.map((item) => (item.id === record.id ? updated : item)));
    } catch (err) {
      setError(`写入权限裁决失败：${errorText(err)}`);
    } finally {
      setResolving(null);
    }
  }, []);

  if (state === "loading") return <EmptyState title="正在读取目标契约..." detail="get_agent_permission_decisions / get_agent_run_audit" />;
  if (state === "error" && error) return <ErrorCard title="读取目标契约失败" error={error} />;

  const auditRows = recordsFrom(audit);

  return (
    <div className="run-tab">
      {error ? <ErrorCard title="目标契约部分数据失败" error={error} /> : null}
      <section className="run-section">
        <div className="run-section-head">
          <span>权限审批账本</span>
          <span className="run-count">{decisions.length}</span>
        </div>
        {decisions.length === 0 ? <div className="run-muted">这个 run 没有权限裁决记录。</div> : null}
        <div className="decision-list">
          {decisions.map((record) => (
            <DecisionCard
              key={record.id}
              record={record}
              resolving={resolving === record.id}
              onResolve={resolveDecision}
            />
          ))}
        </div>
      </section>
      <section className="run-section">
        <div className="run-section-head">
          <span>审计流</span>
          <span className="run-count">{auditRows.length}</span>
        </div>
        {auditRows.length === 0 ? <div className="run-muted">后端没有返回契约项、验证证据或计划变更记录。</div> : null}
        <div className="audit-list">
          {auditRows.map((row, index) => (
            <AuditRow row={row} key={`audit-${index}`} />
          ))}
        </div>
      </section>
    </div>
  );
}

function DecisionCard({
  record,
  resolving,
  onResolve,
}: {
  record: PermissionDecisionRecord;
  resolving: boolean;
  onResolve: (record: PermissionDecisionRecord, approved: boolean) => void;
}) {
  const decision = record.decision || "unknown";
  const pending = ["pending", "needs_confirm", "needs_confirmation", "requested", "unknown", ""].includes(decision.toLowerCase());

  return (
    <article className={`decision-card decision-${cssToken(decision)}`}>
      <div className="decision-top">
        <span className="decision-risk">{record.risk || "risk:unknown"}</span>
        <span className={`run-pill tone-${cssToken(decision)}`}>{decision}</span>
      </div>
      <div className="decision-subject">{record.subject || "未命名权限对象"}</div>
      <div className="decision-action">{record.action || "未记录动作"}</div>
      <div className="decision-meta">
        <span>{record.mode || "mode:unknown"}</span>
        <span>{record.decided_by || "decider:unknown"}</span>
        {formatStamp(record.created_at) ? <span>{formatStamp(record.created_at)}</span> : null}
      </div>
      {record.reason ? <div className="decision-reason">{record.reason}</div> : null}
      {pending ? (
        <div className="gate-actions">
          <button className="btn btn-ghost" type="button" disabled={resolving} onClick={() => onResolve(record, false)}>拒绝</button>
          <button className="btn btn-block" type="button" disabled={resolving} onClick={() => onResolve(record, true)}>批准</button>
        </div>
      ) : null}
    </article>
  );
}

function AuditRow({ row }: { row: UnknownRecord }) {
  const title = textValue(row.label ?? row.title ?? row.kind ?? row.event_type ?? row.type, "审计记录");
  const tone = textValue(row.tone ?? row.status ?? row.verdict ?? row.decision, "audit");
  return (
    <article className={`audit-row tone-${cssToken(tone)}`}>
      <div className="audit-row-top">
        <span>{title}</span>
        <span className="run-muted">{tone}</span>
      </div>
      <pre className="run-raw compact">{safeJson(row.detail ?? row)}</pre>
    </article>
  );
}

function GraphTab({ run }: { run: AgentRunRecord }) {
  const graphRunId = useMemo(() => graphRunIdFromRun(run), [run]);
  const [snapshot, setSnapshot] = useState<AgentGraphSnapshot | null>(null);
  const [traces, setTraces] = useState<WorkflowTraceReport | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [state, setState] = useState<LoadState>("idle");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function loadGraph() {
      if (!graphRunId) {
        setSnapshot(null);
        setTraces(null);
        setSelectedNodeId(null);
        setState("idle");
        setError(null);
        return;
      }

      setState("loading");
      setError(null);
      const [snapshotResult, traceResult] = await Promise.allSettled([
        getAgentGraphSnapshot(graphRunId),
        getAgentGraphNodeTraces(graphRunId),
      ]);
      if (cancelled) return;

      if (snapshotResult.status === "fulfilled") setSnapshot(snapshotResult.value);
      if (traceResult.status === "fulfilled") setTraces(traceResult.value);

      if (snapshotResult.status === "rejected") {
        setState("error");
        setError(errorText(snapshotResult.reason));
      } else {
        setState("ready");
        setError(traceResult.status === "rejected" ? `节点 trace 读取失败：${errorText(traceResult.reason)}` : null);
      }
    }

    void loadGraph();
    return () => {
      cancelled = true;
    };
  }, [graphRunId]);

  if (!graphRunId) {
    return (
      <EmptyState
        title="当前 run 没有运行图记录"
        detail="已找到 get_agent_graph_snapshot/get_agent_graph_node_traces，但未找到本轮可用的按 session/source_run 获取 graphRunId 的命令；不会用普通 run 伪造图。"
      />
    );
  }

  if (state === "loading") return <EmptyState title="正在读取运行图..." detail="get_agent_graph_snapshot / get_agent_graph_node_traces" />;
  if (state === "error" && error) return <ErrorCard title="读取运行图失败" error={error} />;

  const nodes = arrayFrom(snapshot?.nodes);
  const edges = arrayFrom(snapshot?.edges);
  const checkpoints = arrayFrom(snapshot?.checkpoints);
  const traceRows = recordsFrom(traces);
  const selectedTraceRows = selectedNodeId
    ? traceRows.filter((row) => Object.values(row).some((value) => String(value) === selectedNodeId))
    : traceRows;

  return (
    <div className="run-tab">
      {error ? <ErrorCard title="运行图部分数据失败" error={error} /> : null}
      <section className="run-section">
        <div className="run-section-head">
          <span>Graph Run</span>
          <span className="run-id small">{shortId(graphRunId)}</span>
        </div>
        {nodes.length === 0 ? <div className="run-muted">后端 snapshot 没有返回节点。</div> : null}
        <div className="run-graph-grid">
          {nodes.map((node, index) => {
            const nodeId = textValue(node.id ?? node.node_id ?? node.name, String(index));
            const selected = selectedNodeId === nodeId;
            return (
              <button className={`ag-node ${selected ? "sel" : ""}`} type="button" key={nodeId} onClick={() => setSelectedNodeId(nodeId)}>
                <span className="ag-node-title">{textValue(node.label ?? node.name ?? node.id ?? node.node_id, `节点 ${index + 1}`)}</span>
                <span className="ag-node-meta">{textValue(node.kind ?? node.status, nodeId)}</span>
              </button>
            );
          })}
        </div>
      </section>
      <RawFeed title="Edges" value={edges} emptyText="后端 snapshot 没有返回边。" />
      <RawFeed title="Checkpoints" value={checkpoints} emptyText="后端 snapshot 没有返回检查点。" />
      <section className="run-section">
        <div className="run-section-head">
          <span>节点详情</span>
          <span className="run-count">{selectedTraceRows.length}</span>
        </div>
        {selectedTraceRows.length === 0 ? <div className="run-muted">后端没有返回当前节点 trace。</div> : null}
        <div className="run-raw-list">
          {selectedTraceRows.map((row, index) => (
            <pre className="run-raw" key={`trace-${index}`}>{safeJson(row)}</pre>
          ))}
        </div>
      </section>
      {snapshot && recordsFrom(snapshot).length ? <RawFeed title="Snapshot Raw" value={snapshot} emptyText="后端未返回 snapshot 原始记录。" /> : null}
    </div>
  );
}
