import type { AppView } from "../types";
import { useAppState } from "../state/AppState";

interface SidebarProps {
  view: AppView;
  onView: (view: AppView) => void;
  onSearch: () => void;
}

export function Sidebar({ view, onView, onSearch }: SidebarProps) {
  const { sessions, projects, activeSessionId, setActiveSessionId, createNewSession } = useAppState();

  return (
    <aside className="nav sidebar">
      <div className="nav-scroll">
        <div className="nav-section">
          <button className="nav-item new-chat" type="button" onClick={() => void createNewSession("New session")}>
            <span className="ico">＋</span>
            <span className="label">新对话</span>
          </button>
          <button className="nav-item" type="button" onClick={onSearch}>
            <span className="ico">⌕</span>
            <span className="label">搜索</span>
          </button>
          <button className={view === "knowledge" ? "nav-item selected" : "nav-item"} type="button" onClick={() => onView("knowledge")}>
            <span className="ico">◇</span>
            <span className="label">知识库</span>
          </button>
          <button className={view === "plugins" ? "nav-item selected" : "nav-item"} type="button" onClick={() => onView("plugins")}>
            <span className="ico">⌘</span>
            <span className="label">插件</span>
          </button>
          <button className={view === "evals" ? "nav-item selected" : "nav-item"} type="button" onClick={() => onView("evals")}>
            <span className="ico">▥</span>
            <span className="label">防线测试</span>
          </button>
        </div>
        <section className="nav-section">
          <div className="nav-group">项目</div>
          {projects.length === 0 ? <p className="nav-empty">暂无项目。项目列表来自真实后端。</p> : projects.map((project) => (
            <div className="proj" key={project.id}>
              <button className="nav-item proj-toggle" type="button">
                <span className="ico chev">›</span>
                <span className="label">{project.title || project.root_path}</span>
              </button>
              <div className="proj-children">
                <span className="nav-item ghost"><span className="label">{project.root_path}</span></span>
              </div>
            </div>
          ))}
        </section>
        <section className="nav-section">
          <div className="nav-group">对话</div>
          {sessions.length === 0 ? <p className="nav-empty">暂无对话。</p> : sessions.map((session) => (
            <button
              className={session.id === activeSessionId ? "nav-item selected" : "nav-item"}
              type="button"
              key={session.id}
              onClick={() => {
                setActiveSessionId(session.id);
                onView("chat");
              }}
            >
              <span className="ico">•</span>
              <span className="label">{session.title || "Untitled"}</span>
              {session.created_at ? <span className="time">{new Date(session.created_at).toLocaleDateString("zh-CN", { month: "2-digit", day: "2-digit" })}</span> : null}
            </button>
          ))}
        </section>
      </div>
      <button className={view === "settings" ? "nav-item selected settings-link" : "nav-item settings-link"} type="button" onClick={() => onView("settings")}>
        <span className="ico">⚙</span>
        <span className="label">设置</span>
      </button>
    </aside>
  );
}
