import { useCallback, useEffect, useState } from "react";
import { AtlasMainMarkup } from "./AtlasMainMarkup";
import { BackendChatLayer, type PageKey } from "./features/chat/BackendChatLayer";
import { RunDrawer } from "./features/runs/RunDrawer";
import { ModelSettingsPage } from "./features/settings/ModelSettingsPage";
import { atlasLightCss, atlasPageHtml } from "./staticAtlas";
import { closeWindow, minimizeWindow, toggleMaximizeWindow } from "./bridge";
import "./styles.css";

type ThemeMode = "dark" | "light";
type WindowAction = "minimize" | "toggleMaximize" | "close";
const pageByLabel: Record<string, PageKey> = {
  知识库: "knowledge",
  插件: "plugins",
  防线测试: "evals",
  设置: "settings",
};

function htmlForPage(page: PageKey, theme: ThemeMode): string {
  const raw = (atlasPageHtml as Record<string, string>)[page] ?? "";
  const withTheme = raw.includes("<html") ? raw.replace("<html", `<html data-theme="${theme}"`) : raw;
  const extraStyle = `<style>${theme === "light" ? atlasLightCss : ""}</style>`;
  return withTheme.includes("</head>") ? withTheme.replace("</head>", `${extraStyle}</head>`) : `${extraStyle}${withTheme}`;
}

function usePortalTarget(selector: string): HTMLElement | null {
  const [target, setTarget] = useState<HTMLElement | null>(null);

  useEffect(() => {
    setTarget(document.querySelector(selector) as HTMLElement | null);
  }, [selector]);

  return target;
}

async function runWindowAction(action: WindowAction): Promise<void> {
  if (action === "minimize") {
    await minimizeWindow();
    return;
  }

  if (action === "toggleMaximize") {
    await toggleMaximizeWindow();
    return;
  }

  await closeWindow();
}

export function App() {
  const [theme, setTheme] = useState<ThemeMode>("dark");
  const [activePage, setActivePage] = useState<PageKey | null>(null);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [searchOpen, setSearchOpen] = useState(false);

  const navTarget = usePortalTarget(".nav .nav-scroll");
  const threadTarget = usePortalTarget(".thread");
  const composerTarget = usePortalTarget(".composer-wrap");
  const drawerTarget = usePortalTarget(".drawer");

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  useEffect(() => {
    const app = document.getElementById("app");
    if (!app) return;
    const settingsOpen = activePage === "settings";
    app.classList.toggle("page-open", activePage !== null && !settingsOpen);
    app.classList.toggle("settings-mode", settingsOpen);
    app.classList.toggle("settings-react-open", settingsOpen);
  }, [activePage]);

  useEffect(() => {
    const frame = document.getElementById("viewFrame") as HTMLIFrameElement | null;
    if (!frame) return;
    if (!activePage || activePage === "settings") {
      frame.removeAttribute("srcdoc");
      return;
    }

    frame.srcdoc = htmlForPage(activePage, theme);
  }, [activePage, theme]);

  const toggleDrawer = useCallback(() => {
    document.getElementById("app")?.classList.toggle("drawer-open");
  }, []);

  const toggleNav = useCallback(() => {
    document.getElementById("app")?.classList.toggle("nav-collapsed");
  }, []);

  const handleRootClick = useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      const target = event.target as HTMLElement;
      const button = target.closest("button") as HTMLButtonElement | null;
      const menu = target.closest("[data-menu]") as HTMLElement | null;

      if (button?.id === "themeToggle" || target.closest("#themeToggle")) {
        setTheme((current) => (current === "dark" ? "light" : "dark"));
        return;
      }

      const windowTitle = button?.getAttribute("aria-label") ?? button?.getAttribute("title") ?? "";
      const isWindowControl = Boolean(button?.closest(".win-ctrls"));
      if (isWindowControl) {
        event.preventDefault();
        event.stopPropagation();
      }
      if (windowTitle === "最小化") {
        void runWindowAction("minimize").catch((error) => console.error("minimize window failed", error));
        return;
      }
      if (windowTitle === "最大化") {
        void runWindowAction("toggleMaximize").catch((error) => console.error("toggle maximize window failed", error));
        return;
      }
      if (windowTitle === "关闭") {
        void runWindowAction("close").catch((error) => console.error("close window failed", error));
        return;
      }

      if (button?.id === "drawerToggle" || target.closest("#drawerToggle")) {
        toggleDrawer();
        return;
      }

      if (button?.id === "navToggle" || target.closest("#navToggle")) {
        toggleNav();
        return;
      }

      if (button?.classList.contains("menu-btn") && menu) {
        const wasOpen = menu.classList.contains("open");
        document.querySelectorAll("[data-menu]").forEach((item) => item.classList.remove("open"));
        if (!wasOpen) menu.classList.add("open");
        return;
      }

      const tab = target.closest(".drawer-tab") as HTMLElement | null;
      if (tab) {
        const tabName = tab.dataset.tab;
        document.querySelectorAll(".drawer-tab").forEach((item) => item.classList.toggle("active", item === tab));
        document.querySelectorAll(".drawer-inner").forEach((item) => {
          const inner = item as HTMLElement;
          inner.hidden = inner.dataset.view !== tabName;
        });
        return;
      }

      const toolBar = target.closest(".tool-bar") as HTMLElement | null;
      if (toolBar) {
        toolBar.closest(".tool")?.classList.toggle("open");
        return;
      }

      const label = button?.querySelector(".label")?.textContent?.trim() ?? button?.textContent?.trim() ?? "";
      if (label === "搜索") {
        setSearchOpen(true);
        return;
      }
      if (label === "← 返回" || label === "返回") {
        setActivePage(null);
        return;
      }

      const page = pageByLabel[label];
      if (page) setActivePage(page);
    },
    [toggleDrawer, toggleNav],
  );

  return (
    <div className="atlas-static-root backend-chat" onClick={handleRootClick}>
      <AtlasMainMarkup />
      <BackendChatLayer
        navTarget={navTarget}
        threadTarget={threadTarget}
        composerTarget={composerTarget}
        onActiveSessionChange={setActiveSessionId}
        onOpenSearch={() => setSearchOpen(true)}
        onOpenPage={setActivePage}
      />
      {drawerTarget ? <RunDrawer target={drawerTarget} sessionId={activeSessionId} /> : null}
      {activePage === "settings" ? <ModelSettingsPage theme={theme} onBack={() => setActivePage(null)} /> : null}
      <SearchOverlay open={searchOpen} onClose={() => setSearchOpen(false)} />
    </div>
  );
}

function SearchOverlay({ open, onClose }: { open: boolean; onClose: () => void }) {
  if (!open) return null;
  return (
    <div className="search-overlay show" onClick={onClose}>
      <div className="search-box" onClick={(event) => event.stopPropagation()}>
        <div className="search-input-wrap">
          <svg className="ico" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="9" cy="9" r="5.5" />
            <line x1="13.5" y1="13.5" x2="17" y2="17" />
          </svg>
          <input className="search-input" placeholder="搜索对话..." autoComplete="off" spellCheck="false" autoFocus />
          <button className="search-esc" type="button" onClick={onClose}>Esc</button>
        </div>
        <div className="search-results">
          <div className="search-group-title">搜索</div>
          <button className="search-item">
            <span className="search-item-main">
              <span className="search-item-title">搜索真实数据会在 B5 接入</span>
              <span className="search-item-sub">当前阶段只接主聊天路径；这里不展示静态假结果。</span>
            </span>
          </button>
        </div>
      </div>
    </div>
  );
}

export default App;

