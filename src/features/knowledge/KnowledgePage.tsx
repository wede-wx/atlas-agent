import { useEffect, useState } from "react";
import { addKnowledgeItem, deleteKnowledgeItem, searchKnowledge } from "../../bridge";
import type { RetrievalHitRecord } from "../../types";

export function KnowledgePage() {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<RetrievalHitRecord[]>([]);
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function load(q = query) {
    try {
      setError(null);
      const result = await searchKnowledge({ query: q, scope: null, limit: 50 });
      setHits(result.hits);
    } catch (err) {
      setError(String(err));
    }
  }

  useEffect(() => {
    void load("");
  }, []);

  async function add() {
    if (!title.trim() || !text.trim()) return;
    const item = await addKnowledgeItem({ scope: "global", source: "user", trust: "trusted", title, text, confidence: 0.7, expires_at: null, embedding_ref: null });
    setNotice(`已保存知识：${item.title}`);
    setTitle("");
    setText("");
    await load("");
  }

  async function remove(itemId: string) {
    await deleteKnowledgeItem(itemId);
    await load(query);
  }

  return (
    <section className="feature-page">
      <div className="page-head"><div><h1>知识库</h1><p>列表使用 `search_knowledge` 空查询返回，可能不是完整列表；没有伪造 list 接口。</p></div></div>
      {error ? <div className="error-box">{error}</div> : null}
      {notice ? <div className="notice-box">{notice}</div> : null}
      <div className="toolbar"><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索知识…" /><button className="primary-action" type="button" onClick={() => void load()}>搜索</button></div>
      <div className="split-grid">
        <section className="panel-card">
          <h2>添加知识</h2>
          <input value={title} onChange={(event) => setTitle(event.target.value)} placeholder="标题" />
          <textarea value={text} onChange={(event) => setText(event.target.value)} placeholder="内容" />
          <button className="primary-action" type="button" onClick={() => void add()} disabled={!title.trim() || !text.trim()}>保存知识</button>
        </section>
        <section className="panel-card">
          <h2>结果</h2>
          {hits.length === 0 ? <p className="muted">暂无知识命中。</p> : hits.map((hit) => <article className="knowledge-card" key={hit.item_id}><div><strong>{hit.title}</strong><p>{hit.snippet}</p><span className="tag blue">{hit.scope}</span><span className="tag">score {hit.score.toFixed(2)}</span></div><button className="ghost-button danger-text" type="button" onClick={() => void remove(hit.item_id)}>删除</button></article>)}
        </section>
      </div>
    </section>
  );
}
