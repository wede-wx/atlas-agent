import { useEffect, useMemo, useState } from "react";
import {
  checkModelSettings,
  exportLocalData,
  getBackendStatus,
  getConfig,
  getLocalDbHealth,
  getMcpServers,
  getMemories,
  initAgentRules,
  listModels,
  readGlobalAgentRules,
  resetLocalData,
  saveConfig,
  saveGlobalAgentRules,
  setMcpServerTrust,
  testMcpServer,
} from "../../bridge";
import { useAppState } from "../../state/AppState";
import { asRecord, textOf, type McpServerConfig, type UiPreferences, type UnknownRecord } from "../../types";

interface SettingsPageProps {
  onBack: () => void;
}

export function SettingsPage({ onBack }: SettingsPageProps) {
  const { prefs, updatePreference } = useAppState();
  const [config, setConfig] = useState<UnknownRecord>({});
  const [backend, setBackend] = useState<UnknownRecord | null>(null);
  const [mcp, setMcp] = useState<McpServerConfig[]>([]);
  const [rules, setRules] = useState("");
  const [memories, setMemories] = useState<UnknownRecord[]>([]);
  const [dbHealth, setDbHealth] = useState<UnknownRecord | null>(null);
  const [modelResult, setModelResult] = useState<UnknownRecord | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    async function load() {
      try {
        const [cfg, status, servers, globalRules, memoryRows, health] = await Promise.all([
          getConfig(),
          getBackendStatus(),
          getMcpServers(),
          readGlobalAgentRules(),
          getMemories(),
          getLocalDbHealth(),
        ]);
        if (!mounted) return;
        setConfig(cfg);
        setBackend(status);
        setMcp(servers);
        setRules(textOf(asRecord(globalRules).content));
        setMemories(memoryRows);
        setDbHealth(health);
      } catch (err) {
        if (mounted) setError(String(err));
      }
    }
    void load();
    return () => {
      mounted = false;
    };
  }, []);

  const modelPayload = useMemo(() => ({
    provider: textOf(config.provider, "openai-compatible"),
    providerId: typeof config.provider_id === "string" ? config.provider_id : null,
    routeId: typeof config.route_id === "string" ? config.route_id : null,
    protocol: typeof config.protocol === "string" ? config.protocol : null,
    apiUrl: textOf(config.api_url),
    apiKey: null,
    clearApiKey: false,
    modelName: textOf(config.model_name),
    authHeader: typeof config.auth_header === "string" ? config.auth_header : null,
  }), [config]);

  async function saveModelConfig() {
    await saveConfig({
      connectionId: typeof config.connection_id === "string" ? config.connection_id : null,
      provider: modelPayload.provider,
      providerId: modelPayload.providerId,
      routeId: modelPayload.routeId,
      connectionName: typeof config.connection_name === "string" ? config.connection_name : null,
      protocol: modelPayload.protocol,
      apiUrl: modelPayload.apiUrl,
      apiKey: null,
      clearApiKey: false,
      modelName: modelPayload.modelName,
      authHeader: modelPayload.authHeader,
      theme: prefs.theme.mode,
      soundEnabled: prefs.notifications.sound,
    });
    setNotice("模型配置已通过 save_config 写入后端。");
  }

  async function testModel(kind: "check" | "list") {
    const result = kind === "check" ? await checkModelSettings(modelPayload) : await listModels(modelPayload);
    setModelResult(result);
  }

  async function saveRules() {
    await saveGlobalAgentRules(rules);
    setNotice("Agent 规则已保存。");
  }

  async function toggleMcpTrust(server: McpServerConfig) {
    const updated = await setMcpServerTrust(server.id, !server.trusted);
    setMcp((rows) => rows.map((row) => row.id === updated.id ? updated : row));
  }

  async function updateTheme(mode: UiPreferences["theme"]["mode"]) {
    await updatePreference("theme", { mode });
    setNotice(`外观偏好已保存到 ui.theme：${mode}`);
  }

  async function exportData() {
    const path = await exportLocalData();
    setNotice(`已导出本地数据：${path}`);
  }

  async function resetAppStateOnly() {
    if (!window.confirm("这会调用真实 reset_local_data 并清理 app_state，确定继续？")) return;
    const result = await resetLocalData({ app_state: true });
    setNotice(`reset_local_data 返回：${JSON.stringify(result)}`);
  }

  return (
    <section className="settings-page">
      <div className="page-head">
        <button className="ghost-button" type="button" onClick={onBack}>返回</button>
        <div>
          <h1>设置</h1>
          <p>七个板块均调用真实后端；没有后端数据时显示空态。</p>
        </div>
      </div>
      {error ? <div className="error-box">{error}</div> : null}
      {notice ? <div className="notice-box">{notice}</div> : null}
      <div className="settings-grid">
        <section className="settings-card">
          <h2>模型</h2>
          <Field label="Provider" value={modelPayload.provider} onChange={(value) => setConfig({ ...config, provider: value })} />
          <Field label="API URL" value={modelPayload.apiUrl} onChange={(value) => setConfig({ ...config, api_url: value })} />
          <Field label="Model" value={modelPayload.modelName} onChange={(value) => setConfig({ ...config, model_name: value })} />
          <div className="row-actions"><button className="primary-action" type="button" onClick={() => void saveModelConfig()}>保存模型配置</button><button className="ghost-button" type="button" onClick={() => void testModel("check")}>检查</button><button className="ghost-button" type="button" onClick={() => void testModel("list")}>列模型</button></div>
          <pre className="code-panel">{modelResult ? JSON.stringify(modelResult, null, 2) : JSON.stringify(backend, null, 2)}</pre>
        </section>
        <section className="settings-card">
          <h2>安全与权限</h2>
          <p className="muted">MCP 信任状态来自真实 `get_mcp_servers`。</p>
          {mcp.length === 0 ? <p className="muted">暂无 MCP server。</p> : mcp.map((server) => <div className="srow" key={server.id}><div><strong>{server.name}</strong><p>{server.transport} · {server.risk} · {server.last_status ?? "unknown"}</p></div><button className="ghost-button" type="button" onClick={() => void toggleMcpTrust(server)}>{server.trusted ? "取消信任" : "信任"}</button><button className="ghost-button" type="button" onClick={() => void testMcpServer(server.id).then((res) => setNotice(JSON.stringify(res)))}>测试</button></div>)}
        </section>
        <section className="settings-card wide">
          <h2>Agent 规则</h2>
          <textarea className="large-textarea" value={rules} onChange={(event) => setRules(event.target.value)} placeholder="全局 Agent 规则为空。" />
          <div className="row-actions"><button className="primary-action" type="button" onClick={() => void saveRules()}>保存规则</button><button className="ghost-button" type="button" onClick={() => void initAgentRules(null).then((res) => setNotice(JSON.stringify(res)))}>初始化项目规则</button></div>
        </section>
        <section className="settings-card">
          <h2>通知</h2>
          <Toggle label="运行完成" checked={prefs.notifications.runCompleted} onChange={(runCompleted) => void updatePreference("notifications", { ...prefs.notifications, runCompleted })} />
          <Toggle label="四道门拦截" checked={prefs.notifications.blockedGate} onChange={(blockedGate) => void updatePreference("notifications", { ...prefs.notifications, blockedGate })} />
          <Toggle label="需要权限" checked={prefs.notifications.permissionNeeded} onChange={(permissionNeeded) => void updatePreference("notifications", { ...prefs.notifications, permissionNeeded })} />
          <Toggle label="声音" checked={prefs.notifications.sound} onChange={(sound) => void updatePreference("notifications", { ...prefs.notifications, sound })} />
        </section>
        <section className="settings-card">
          <h2>外观</h2>
          <div className="segmented"><button className={prefs.theme.mode === "dark" ? "active" : ""} type="button" onClick={() => void updateTheme("dark")}>深色</button><button className={prefs.theme.mode === "light" ? "active" : ""} type="button" onClick={() => void updateTheme("light")}>浅色</button><button className={prefs.theme.mode === "system" ? "active" : ""} type="button" onClick={() => void updateTheme("system")}>系统</button></div>
          <p className="muted">字段：ui.theme</p>
        </section>
        <section className="settings-card">
          <h2>常规</h2>
          <label className="field-label">默认模式</label>
          <select value={prefs.general.defaultAgentMode} onChange={(event) => void updatePreference("general", { ...prefs.general, defaultAgentMode: event.target.value as "chat" | "agent" })}><option value="chat">chat</option><option value="agent">agent</option></select>
          <Toggle label="运行时打开右抽屉" checked={prefs.general.openDrawerOnRun} onChange={(openDrawerOnRun) => void updatePreference("general", { ...prefs.general, openDrawerOnRun })} />
          <Toggle label="无会话时自动创建" checked={prefs.general.autoCreateSession} onChange={(autoCreateSession) => void updatePreference("general", { ...prefs.general, autoCreateSession })} />
        </section>
        <section className="settings-card">
          <h2>数据与隐私</h2>
          <pre className="code-panel">{JSON.stringify({ dbHealth, memoryCount: memories.length }, null, 2)}</pre>
          <div className="row-actions"><button className="ghost-button" type="button" onClick={() => void exportData()}>导出本地数据</button><button className="ghost-button danger-text" type="button" onClick={() => void resetAppStateOnly()}>重置 UI 偏好</button></div>
        </section>
      </div>
    </section>
  );
}

function Field({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) {
  return <label className="field"><span>{label}</span><input value={value} onChange={(event) => onChange(event.target.value)} /></label>;
}

function Toggle({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) {
  return <label className="toggle-row"><span>{label}</span><button className={checked ? "switch on" : "switch"} type="button" onClick={() => onChange(!checked)}><span /></button></label>;
}


