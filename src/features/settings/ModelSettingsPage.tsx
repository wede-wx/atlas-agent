import { useCallback, useEffect, useMemo, useState } from "react";
import { checkModelSettings, deleteModelConnection, getConfig, listModels, revealModelConnectionKey, saveConfig } from "../../bridge";
import type { UnknownRecord } from "../../types";

type ThemeMode = "dark" | "light";

type ModelSettingsPageProps = {
  theme: ThemeMode;
  onBack: () => void;
};

type FormState = {
  connectionId: string;
  provider: string;
  providerId: string;
  routeId: string;
  connectionName: string;
  protocol: string;
  apiUrl: string;
  apiKey: string;
  clearApiKey: boolean;
  modelName: string;
  authHeader: string;
  theme: string;
  hasSavedApiKey: boolean;
};

type StatusState = {
  kind: "idle" | "loading" | "success" | "error";
  message: string;
};

const EMPTY_FORM: FormState = {
  connectionId: "",
  provider: "",
  providerId: "",
  routeId: "",
  connectionName: "",
  protocol: "",
  apiUrl: "",
  apiKey: "",
  clearApiKey: false,
  modelName: "",
  authHeader: "",
  theme: "",
  hasSavedApiKey: false,
};

const SETTING_SECTIONS = ["模型", "安全与权限", "Agent 规则", "通知", "外观", "常规", "数据与隐私"];

function asRecord(value: unknown): UnknownRecord {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as UnknownRecord) : {};
}

function asRecordArray(value: unknown): UnknownRecord[] {
  return Array.isArray(value)
    ? (value.filter((item) => item && typeof item === "object" && !Array.isArray(item)) as UnknownRecord[])
    : [];
}

function getString(source: UnknownRecord, keys: string[]): string {
  for (const key of keys) {
    const value = source[key];
    if (typeof value === "string") return value;
    if (typeof value === "number" || typeof value === "boolean") return String(value);
  }
  return "";
}

function getBoolean(source: UnknownRecord, keys: string[]): boolean | null {
  for (const key of keys) {
    const value = source[key];
    if (typeof value === "boolean") return value;
  }
  return null;
}

function firstNonEmpty(...values: string[]): string {
  return values.find((value) => value.trim().length > 0) ?? "";
}

function extractConnections(config: UnknownRecord): UnknownRecord[] {
  const llm = asRecord(config.llm);
  const model = asRecord(config.model);
  return [
    ...asRecordArray(llm.connections),
    ...asRecordArray(model.connections),
    ...asRecordArray(config.connections),
  ];
}

function connectionIdOf(connection: UnknownRecord): string {
  return getString(connection, ["id", "connection_id", "connectionId"]);
}

function connectionLabel(connection: UnknownRecord): string {
  return firstNonEmpty(
    getString(connection, ["connection_name", "connectionName", "name"]),
    getString(connection, ["model_name", "modelName", "model"]),
    getString(connection, ["provider", "provider_id", "providerId"]),
    "未命名连接",
  );
}

function hasSavedKey(source: UnknownRecord, fallback: UnknownRecord): boolean {
  const flag = getBoolean(source, ["has_api_key", "hasApiKey", "api_key_saved", "apiKeySaved"]);
  if (flag !== null) return flag;
  const fallbackFlag = getBoolean(fallback, ["has_api_key", "hasApiKey", "api_key_saved", "apiKeySaved"]);
  if (fallbackFlag !== null) return fallbackFlag;
  const key = firstNonEmpty(getString(source, ["api_key", "apiKey"]), getString(fallback, ["api_key", "apiKey"]));
  return key.length > 0;
}

function formFromConfig(config: UnknownRecord, selectedId?: string): FormState {
  const connections = extractConnections(config);
  const llm = asRecord(config.llm);
  const model = asRecord(config.model);
  const ui = asRecord(config.ui);
  const defaultId = firstNonEmpty(
    selectedId ?? "",
    getString(llm, ["default_connection_id", "defaultConnectionId"]),
    getString(model, ["default_connection_id", "defaultConnectionId"]),
    getString(config, ["default_connection_id", "defaultConnectionId", "connection_id", "connectionId"]),
  );
  const selected = connections.find((connection) => connectionIdOf(connection) === defaultId) ?? connections[0];
  const source = selected ?? config;
  const theme = firstNonEmpty(getString(config, ["theme"]), getString(ui, ["theme"]));

  return {
    connectionId: firstNonEmpty(connectionIdOf(source), getString(config, ["connection_id", "connectionId"])),
    provider: firstNonEmpty(getString(source, ["provider"]), getString(config, ["provider"]), getString(llm, ["provider"]), getString(model, ["provider"])),
    providerId: firstNonEmpty(getString(source, ["provider_id", "providerId"]), getString(config, ["provider_id", "providerId"])),
    routeId: firstNonEmpty(getString(source, ["route_id", "routeId"]), getString(config, ["route_id", "routeId"])),
    connectionName: getString(source, ["connection_name", "connectionName", "name"]),
    protocol: firstNonEmpty(getString(source, ["protocol"]), getString(config, ["protocol"])),
    apiUrl: firstNonEmpty(getString(source, ["api_url", "apiUrl", "base_url", "baseUrl"]), getString(config, ["api_url", "apiUrl", "base_url", "baseUrl"])),
    apiKey: "",
    clearApiKey: false,
    modelName: firstNonEmpty(getString(source, ["model_name", "modelName", "model"]), getString(config, ["model_name", "modelName", "model"])),
    authHeader: firstNonEmpty(getString(source, ["auth_header", "authHeader"]), getString(config, ["auth_header", "authHeader"])),
    theme,
    hasSavedApiKey: hasSavedKey(source, config),
  };
}

function parseModels(result: unknown): string[] {
  const record = asRecord(result);
  const nested = asRecord(record.data);
  const raw = Array.isArray(result)
    ? result
    : Array.isArray(record.models)
      ? record.models
      : Array.isArray(nested.models)
        ? nested.models
        : [];

  return raw
    .map((item) => {
      if (typeof item === "string") return item;
      const itemRecord = asRecord(item);
      return firstNonEmpty(getString(itemRecord, ["id"]), getString(itemRecord, ["name"]), getString(itemRecord, ["model"]));
    })
    .filter((item, index, all) => item.length > 0 && all.indexOf(item) === index);
}

function resultMessage(result: unknown, fallback: string): string {
  const record = asRecord(result);
  return firstNonEmpty(getString(record, ["message", "status"]), fallback);
}

function formatError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  try {
    return JSON.stringify(error);
  } catch {
    return "未知错误";
  }
}

function buildModelPayload(form: FormState): UnknownRecord {
  return {
    connectionId: form.connectionId || null,
    provider: form.provider,
    providerId: form.providerId || null,
    routeId: form.routeId || null,
    protocol: form.protocol || null,
    apiUrl: form.apiUrl.trim(),
    apiKey: form.apiKey.trim() || null,
    clearApiKey: form.clearApiKey,
    modelName: form.modelName.trim(),
    authHeader: form.authHeader || null,
  };
}

function buildSavePayload(form: FormState): UnknownRecord {
  if (!form.provider.trim()) throw new Error("请先选择服务商。不会保存空服务商配置。");
  if (!form.apiUrl.trim()) throw new Error("请先填写 API 地址。不会保存空地址配置。");
  if (!form.modelName.trim()) throw new Error("请先填写或选择模型。不会保存空模型配置。");
  if (!form.theme.trim()) {
    throw new Error("后端配置缺少 theme，无法构造 save_config 必需参数；未保存。请先报告此配置结构不匹配问题。");
  }

  return {
    connection_id: form.connectionId || null,
    provider: form.provider,
    provider_id: form.providerId || null,
    route_id: form.routeId || null,
    connection_name: form.connectionName.trim() || null,
    protocol: form.protocol || null,
    api_url: form.apiUrl.trim(),
    api_key: form.apiKey.trim() || null,
    clear_api_key: form.clearApiKey,
    model_name: form.modelName.trim(),
    auth_header: form.authHeader || null,
    theme: form.theme,
  };
}

export function ModelSettingsPage({ theme, onBack }: ModelSettingsPageProps) {
  const [config, setConfig] = useState<UnknownRecord | null>(null);
  const [connections, setConnections] = useState<UnknownRecord[]>([]);
  const [selectedConnectionId, setSelectedConnectionId] = useState<string>("");
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [models, setModels] = useState<string[]>([]);
  const [loadStatus, setLoadStatus] = useState<StatusState>({ kind: "loading", message: "正在读取本地配置…" });
  const [actionStatus, setActionStatus] = useState<StatusState>({ kind: "idle", message: "" });
  const [showApiKey, setShowApiKey] = useState(false);
  const [revealedApiKey, setRevealedApiKey] = useState("");
  const [advancedOpen, setAdvancedOpen] = useState(false);

  const loadConfig = useCallback(async (preferredId = selectedConnectionId) => {
    setLoadStatus({ kind: "loading", message: "正在读取本地配置…" });
    try {
      const nextConfig = await getConfig();
      const nextConnections = extractConnections(nextConfig);
      const nextForm = formFromConfig(nextConfig, preferredId);
      setConfig(nextConfig);
      setConnections(nextConnections);
      setSelectedConnectionId(nextForm.connectionId);
      setForm(nextForm);
      setShowApiKey(false);
      setRevealedApiKey("");
      setLoadStatus({ kind: "success", message: nextConnections.length > 0 ? "已载入已保存模型配置。" : "尚未配置模型连接。" });
    } catch (error) {
      setLoadStatus({ kind: "error", message: `读取配置失败：${formatError(error)}` });
    }
  }, [selectedConnectionId]);

  useEffect(() => {
    void loadConfig("");
  }, [loadConfig]);

  const canListModels = useMemo(() => Boolean(form.provider.trim() && form.apiUrl.trim()), [form.provider, form.apiUrl]);
  const canCheck = useMemo(() => Boolean(form.provider.trim() && form.apiUrl.trim() && form.modelName.trim()), [form.provider, form.apiUrl, form.modelName]);

  const updateField = <K extends keyof FormState>(field: K, value: FormState[K]) => {
    setForm((current) => ({ ...current, [field]: value }));
  };

  const selectConnection = (connection: UnknownRecord) => {
    if (!config) return;
    const id = connectionIdOf(connection);
    setSelectedConnectionId(id);
    setForm(formFromConfig(config, id));
    setModels([]);
    setShowApiKey(false);
    setRevealedApiKey("");
    setActionStatus({ kind: "idle", message: "" });
  };

  const handleListModels = async () => {
    if (!canListModels) {
      setActionStatus({ kind: "error", message: "请先填写服务商和 API 地址，再获取模型列表。" });
      return;
    }
    setActionStatus({ kind: "loading", message: "正在向后端请求模型列表…" });
    try {
      const result = await listModels(buildModelPayload(form));
      const nextModels = parseModels(result);
      setModels(nextModels);
      setActionStatus({
        kind: "success",
        message: nextModels.length > 0 ? `已获取 ${nextModels.length} 个模型。` : resultMessage(result, "后端没有返回可用模型。"),
      });
    } catch (error) {
      setActionStatus({ kind: "error", message: `获取模型失败：${formatError(error)}` });
    }
  };

  const handleCheck = async () => {
    if (!canCheck) {
      setActionStatus({ kind: "error", message: "请先填写服务商、API 地址和模型，再测试连接。" });
      return;
    }
    setActionStatus({ kind: "loading", message: "正在校验模型连接…" });
    try {
      const result = await checkModelSettings(buildModelPayload(form));
      const returnedModels = parseModels(result);
      if (returnedModels.length > 0) setModels(returnedModels);
      setActionStatus({ kind: "success", message: resultMessage(result, "模型连接校验完成。") });
    } catch (error) {
      setActionStatus({ kind: "error", message: `模型连接校验失败：${formatError(error)}` });
    }
  };

  const handleSave = async () => {
    setActionStatus({ kind: "loading", message: "正在保存模型配置…" });
    try {
      const payload = buildSavePayload(form);
      await saveConfig(payload);
      setShowApiKey(false);
      setRevealedApiKey("");
      setForm((current) => ({ ...current, apiKey: "", clearApiKey: false, hasSavedApiKey: true }));
      setActionStatus({ kind: "success", message: "模型配置已保存。" });
      await loadConfig(form.connectionId);
    } catch (error) {
      setActionStatus({ kind: "error", message: `保存失败：${formatError(error)}` });
    }
  };

  const handleDeleteConnection = async (connection: UnknownRecord) => {
    const id = connectionIdOf(connection);
    if (!id) {
      setActionStatus({ kind: "error", message: "无法删除：后端没有返回该连接的 connection_id。" });
      return;
    }
    const label = connectionLabel(connection);
    if (!window.confirm(`确认删除模型连接“${label}”？此操作会真实写入本地配置。`)) return;
    setActionStatus({ kind: "loading", message: `正在删除模型连接：${label}…` });
    try {
      await deleteModelConnection(id);
      setModels([]);
      setShowApiKey(false);
      setRevealedApiKey("");
      setSelectedConnectionId("");
      setActionStatus({ kind: "success", message: `已删除模型连接：${label}。` });
      await loadConfig("");
    } catch (error) {
      setActionStatus({ kind: "error", message: `删除失败：${formatError(error)}` });
    }
  };

  const handleToggleApiKeyVisibility = async () => {
    if (showApiKey) {
      setShowApiKey(false);
      setRevealedApiKey("");
      return;
    }

    if (form.apiKey.length > 0) {
      setShowApiKey(true);
      return;
    }

    if (!form.hasSavedApiKey) {
      setShowApiKey(true);
      return;
    }

    if (!form.connectionId.trim()) {
      setActionStatus({ kind: "error", message: "无法查看已保存密钥：当前连接缺少 connection_id。" });
      return;
    }

    setActionStatus({ kind: "loading", message: "正在按需读取已保存 API Key…" });
    try {
      const key = await revealModelConnectionKey(form.connectionId);
      setRevealedApiKey(key);
      setShowApiKey(true);
      setActionStatus({ kind: "success", message: key ? "已临时显示已保存 API Key；切回隐藏会立即清除明文。" : "该连接已保存的 API Key 为空。" });
    } catch (error) {
      setRevealedApiKey("");
      setShowApiKey(false);
      setActionStatus({ kind: "error", message: `读取已保存 API Key 失败：${formatError(error)}` });
    }
  };

  const visibleApiKeyValue = showApiKey && form.apiKey.length === 0 ? revealedApiKey : form.apiKey;

  return (
    <section className="settings-react-page" data-theme={theme} aria-label="设置">
      <aside className="settings-react-nav">
        <button type="button" className="settings-react-back" onClick={onBack}>← 返回</button>
        <div className="settings-react-nav-title">设置</div>
        <div className="settings-react-nav-list">
          {SETTING_SECTIONS.map((section) => (
            <button key={section} type="button" className={`snav-item ${section === "模型" ? "active" : "disabled"}`} disabled={section !== "模型"}>
              <span>{section}</span>
              {section !== "模型" ? <small>后续接入</small> : null}
            </button>
          ))}
        </div>
      </aside>

      <main className="settings-react-content">
        <section className="pane active model-pane">
          <div className="pane-head">
            <div>
              <div className="pane-kicker">Model Connection</div>
              <h1>模型</h1>
              <p>配置 Atlas 调用真实模型所需的连接。这里直接读取和保存本地 Tauri 后端配置。</p>
            </div>
            <button type="button" className="ghost-btn" onClick={() => void loadConfig()}>重新读取</button>
          </div>

          <div className={`settings-status ${loadStatus.kind}`}>{loadStatus.message}</div>

          <div className="group">
            <div className="group-head">
              <div>
                <h2>已保存连接</h2>
                <p>来自 get_config。如果后端没有配置，这里显示真实空态。</p>
              </div>
            </div>
            {connections.length > 0 ? (
              <div className="conn-list">
                {connections.map((connection, index) => {
                  const id = connectionIdOf(connection) || `connection-${index}`;
                  const active = id === selectedConnectionId || (!selectedConnectionId && index === 0);
                  return (
                    <div key={id} className={`conn ${active ? "active" : ""}`}>
                      <button type="button" className="conn-select" onClick={() => selectConnection(connection)}>
                        <span className="conn-main">
                        <span className="conn-name-line">
                          <strong>{connectionLabel(connection)}</strong>
                          <span>{getString(connection, ["provider", "provider_id", "providerId"]) || "未知服务商"}</span>
                        </span>
                        <span className="conn-model">{getString(connection, ["model_name", "modelName", "model"]) || "未记录模型"}</span>
                        </span>
                      </button>
                      <button type="button" className="conn-delete" onClick={() => void handleDeleteConnection(connection)}>删除</button>
                    </div>
                  );
                })}
              </div>
            ) : (
              <div className="settings-empty-card">
                <strong>尚未配置模型连接</strong>
                <span>填写下方表单后保存。页面不会生成默认连接，也不会展示假模型列表。</span>
              </div>
            )}
          </div>

          <div className="group">
            <div className="group-head">
              <div>
                <h2>连接参数</h2>
                <p>保存前只提交你在表单中明确填写或选择的字段；API Key 留空时保持现有密钥。</p>
              </div>
            </div>

            <div className="conn-form">
              <label className="field">
                <span className="field-label">连接名称</span>
                <input className="input" value={form.connectionName} placeholder="例如 OpenAI 主连接" onChange={(event) => updateField("connectionName", event.target.value)} />
              </label>

              <label className="field">
                <span className="field-label">服务商</span>
                <select className="select" value={form.provider} onChange={(event) => updateField("provider", event.target.value)}>
                  <option value="">选择服务商</option>
                  <option value="openai">OpenAI</option>
                  <option value="anthropic">Anthropic</option>
                  <option value="deepseek">DeepSeek</option>
                  <option value="ollama">Ollama</option>
                  <option value="lmstudio">LM Studio</option>
                  <option value="custom">OpenAI-compatible / Custom</option>
                </select>
              </label>

              <label className="field wide">
                <span className="field-label">API 地址</span>
                <input className="input" value={form.apiUrl} placeholder="https://api.openai.com/v1" onChange={(event) => updateField("apiUrl", event.target.value)} />
              </label>

              <label className="field wide">
                <span className="field-label">API Key</span>
                <span className="api-key-field">
                  <input className="input" type={showApiKey ? "text" : "password"} value={visibleApiKeyValue} placeholder={form.hasSavedApiKey ? "已保存密钥，留空保持不变" : "未保存密钥；本地模型可留空"} onChange={(event) => { setRevealedApiKey(""); updateField("apiKey", event.target.value); }} />
                  <button type="button" className="api-key-toggle" title={showApiKey ? "隐藏 API Key" : "显示 API Key"} aria-label={showApiKey ? "隐藏 API Key" : "显示 API Key"} onMouseDown={(event) => event.preventDefault()} onClick={() => void handleToggleApiKeyVisibility()}>
                    <svg viewBox="0 0 20 20" aria-hidden="true">
                      <path d="M2.6 10s2.5-4.4 7.4-4.4 7.4 4.4 7.4 4.4-2.5 4.4-7.4 4.4S2.6 10 2.6 10Z" />
                      <circle cx="10" cy="10" r="2.2" />
                      {showApiKey ? null : <line x1="4.2" y1="16" x2="15.8" y2="4" />}
                    </svg>
                  </button>
                </span>
              </label>

              <label className="field">
                <span className="field-label">模型</span>
                <input className="input" value={form.modelName} placeholder="例如 gpt-4.1-mini" onChange={(event) => updateField("modelName", event.target.value)} list="atlas-real-models" />
                <datalist id="atlas-real-models">
                  {models.map((model) => <option key={model} value={model} />)}
                </datalist>
              </label>

              <label className="field inline-field">
                <input type="checkbox" checked={form.clearApiKey} onChange={(event) => { setShowApiKey(false); setRevealedApiKey(""); updateField("clearApiKey", event.target.checked); }} />
                <span>清除已保存 API Key</span>
              </label>
            </div>

            <div className={`advanced-options ${advancedOpen ? "open" : ""}`}>
              <button type="button" className="advanced-toggle" onClick={() => setAdvancedOpen((current) => !current)}>
                <span>高级选项</span>
                <span>{advancedOpen ? "收起" : "展开"}</span>
              </button>
              {advancedOpen ? (
                <div className="advanced-grid">
                  <label className="field">
                    <span className="field-label">Provider ID</span>
                    <input className="input" value={form.providerId} placeholder="留空由后端推导" onChange={(event) => updateField("providerId", event.target.value)} />
                  </label>

                  <label className="field">
                    <span className="field-label">Route ID</span>
                    <input className="input" value={form.routeId} placeholder="留空由后端推导" onChange={(event) => updateField("routeId", event.target.value)} />
                  </label>

                  <label className="field">
                    <span className="field-label">协议</span>
                    <select className="select" value={form.protocol} onChange={(event) => updateField("protocol", event.target.value)}>
                      <option value="">使用后端默认协议</option>
                      <option value="openai">OpenAI compatible</option>
                      <option value="anthropic">Anthropic</option>
                      <option value="ollama">Ollama</option>
                    </select>
                  </label>

                  <label className="field">
                    <span className="field-label">认证 Header</span>
                    <select className="select" value={form.authHeader} onChange={(event) => updateField("authHeader", event.target.value)}>
                      <option value="">使用后端默认</option>
                      <option value="authorization">Authorization</option>
                      <option value="x-api-key">x-api-key</option>
                      <option value="api-key">api-key</option>
                    </select>
                  </label>
                </div>
              ) : null}
            </div>

            <div className="model-list-box">
              <div className="model-list-head">
                <span>后端返回模型</span>
                <span>{models.length > 0 ? `${models.length} 个` : "暂无"}</span>
              </div>
              {models.length > 0 ? (
                <div className="model-chip-list">
                  {models.map((model) => (
                    <button key={model} type="button" className={model === form.modelName ? "model-chip active" : "model-chip"} onClick={() => updateField("modelName", model)}>{model}</button>
                  ))}
                </div>
              ) : (
                <p>尚未从后端获取模型列表。点击“获取模型”后只展示真实返回结果。</p>
              )}
            </div>

            <div className="form-foot">
              <div className={`test-status ${actionStatus.kind}`}>{actionStatus.message || "空态、错误态和后端返回会显示在这里。"}</div>
              <div className="form-actions">
                <button type="button" className="ghost-btn" onClick={handleListModels}>获取模型</button>
                <button type="button" className="ghost-btn" onClick={handleCheck}>测试连接</button>
                <button type="button" className="primary-btn" onClick={handleSave}>保存配置</button>
              </div>
            </div>
          </div>
        </section>
      </main>
    </section>
  );
}
