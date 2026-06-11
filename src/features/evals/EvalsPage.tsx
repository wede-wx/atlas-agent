import { useEffect, useState } from "react";
import { getAgentEvalSuites, runAgentEvalSuiteVerifiers, scoreAgentEvalSuite } from "../../bridge";
import type { EvalSuite, UnknownRecord } from "../../types";

export function EvalsPage() {
  const [suites, setSuites] = useState<EvalSuite[]>([]);
  const [selected, setSelected] = useState<string>("");
  const [report, setReport] = useState<UnknownRecord | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    async function load() {
      try {
        const rows = await getAgentEvalSuites();
        if (!mounted) return;
        setSuites(rows);
        setSelected(rows[0]?.id ?? "");
      } catch (err) {
        if (mounted) setError(String(err));
      }
    }
    void load();
    return () => {
      mounted = false;
    };
  }, []);

  async function runSuite() {
    if (!selected) return;
    const result = await runAgentEvalSuiteVerifiers(selected, null, null, false);
    setReport(result);
  }

  async function scoreSuite() {
    if (!selected) return;
    const result = await scoreAgentEvalSuite(selected, []);
    setReport(result);
  }

  return (
    <section className="feature-page">
      <div className="page-head"><div><h1>防线测试</h1><p>内置 eval suite 来自 Rust 后端，运行 verifier 不伪造结果。</p></div></div>
      {error ? <div className="error-box">{error}</div> : null}
      <div className="split-grid">
        <section className="panel-card">
          <h2>套件</h2>
          {suites.length === 0 ? <p className="muted">暂无 eval suite。</p> : suites.map((suite) => <button className={suite.id === selected ? "suite-row active" : "suite-row"} type="button" key={suite.id} onClick={() => setSelected(suite.id)}><strong>{suite.name}</strong><span>{suite.description}</span><span className="tag">{suite.cases.length} cases</span></button>)}
        </section>
        <section className="panel-card">
          <h2>执行</h2>
          <div className="row-actions"><button className="primary-action" type="button" disabled={!selected} onClick={() => void runSuite()}>运行 verifiers</button><button className="ghost-button" type="button" disabled={!selected} onClick={() => void scoreSuite()}>空结果评分</button></div>
          <pre className="code-panel">{report ? JSON.stringify(report, null, 2) : "还没有运行。"}</pre>
        </section>
      </div>
    </section>
  );
}
