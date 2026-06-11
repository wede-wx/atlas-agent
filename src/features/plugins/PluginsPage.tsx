import { useEffect, useState } from "react";
import { getPluginCapabilityEvents, listPluginPackages, setPluginPackageEnabled } from "../../bridge";
import type { PluginPackageRecord, UnknownRecord } from "../../types";

export function PluginsPage() {
  const [packages, setPackages] = useState<PluginPackageRecord[]>([]);
  const [events, setEvents] = useState<UnknownRecord[]>([]);
  const [error, setError] = useState<string | null>(null);

  async function load() {
    try {
      const [pkgRows, eventRows] = await Promise.all([listPluginPackages(), getPluginCapabilityEvents(null, 60)]);
      setPackages(pkgRows);
      setEvents(eventRows);
    } catch (err) {
      setError(String(err));
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function toggle(id: string, enabled: boolean) {
    await setPluginPackageEnabled(id, enabled);
    await load();
  }

  return (
    <section className="feature-page">
      <div className="page-head"><div><h1>插件</h1><p>插件包和 capability 事件均来自真实本地数据库。</p></div></div>
      {error ? <div className="error-box">{error}</div> : null}
      <div className="split-grid">
        <section className="panel-card">
          <h2>已安装插件</h2>
          {packages.length === 0 ? <p className="muted">暂无插件包。</p> : packages.map((pkg) => <article className="plugin-card" key={pkg.id}><div><strong>{pkg.name}</strong><p>{pkg.description}</p><span className="tag">{pkg.version}</span><span className="tag blue">{pkg.risk}</span><span className="tag">{pkg.trusted ? "trusted" : "untrusted"}</span></div><button className="ghost-button" type="button" onClick={() => void toggle(pkg.id, !pkg.enabled)}>{pkg.enabled ? "停用" : "启用"}</button></article>)}
        </section>
        <section className="panel-card">
          <h2>Capability 事件</h2>
          {events.length === 0 ? <p className="muted">暂无插件事件。</p> : events.map((event, index) => <pre className="code-panel" key={String(event.id ?? index)}>{JSON.stringify(event, null, 2)}</pre>)}
        </section>
      </div>
    </section>
  );
}
