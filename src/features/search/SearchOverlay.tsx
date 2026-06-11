import { useCallback, useEffect, useMemo, useState } from "react";
import { searchKnowledge, searchSessions } from "../../bridge";
import type { RetrievalHitRecord, SessionRecord } from "../../types";

interface SearchOverlayProps {
  open: boolean;
  onClose: () => void;
  onSession: (id: string) => void;
}

export function SearchOverlay({ open, onClose, onSession }: SearchOverlayProps) {
  const [query, setQuery] = useState("");
  const [sessions, setSessions] = useState<SessionRecord[]>([]);
  const [knowledge, setKnowledge] = useState<RetrievalHitRecord[]>([]);
  const [error, setError] = useState<string | null>(null);

  const runSearch = useCallback(async (text: string) => {
    if (!text.trim()) {
      setSessions([]);
      setKnowledge([]);
      setError(null);
      return;
    }
    try {
      setError(null);
      const [sessionRows, knowledgeRows] = await Promise.all([
        searchSessions(text),
        searchKnowledge({ query: text, scope: null, limit: 8 }),
      ]);
      setSessions(sessionRows);
      setKnowledge(knowledgeRows.hits);
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    if (!open) return;
    const id = window.setTimeout(() => void runSearch(query), 180);
    return () => window.clearTimeout(id);
  }, [open, query, runSearch]);

  const hasResults = useMemo(() => sessions.length > 0 || knowledge.length > 0, [knowledge.length, sessions.length]);

  if (!open) return null;

  return (
    <div className="overlay" role="dialog" aria-modal="true">
      <button className="overlay-backdrop" type="button" aria-label="关闭搜索" onClick={onClose} />
      <section className="search-panel">
        <div className="search-input-wrap">
          <span>⌕</span>
          <input autoFocus value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索对话和知识…" />
          <button className="ghost-button" type="button" onClick={onClose}>关闭</button>
        </div>
        {error ? <div className="error-box">{error}</div> : null}
        {!hasResults && query ? <p className="muted">没有真实后端结果。</p> : null}
        <div className="search-results">
          {sessions.map((session) => (
            <button className="result-row" type="button" key={session.id} onClick={() => { onSession(session.id); onClose(); }}>
              <span className="tag">对话</span>
              <span>{session.title || "Untitled"}</span>
            </button>
          ))}
          {knowledge.map((hit) => (
            <div className="result-row" key={hit.item_id}>
              <span className="tag blue">知识</span>
              <span>{hit.title}</span>
              <span className="subtle">{hit.snippet}</span>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}
