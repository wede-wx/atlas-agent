import { closeWindow, minimizeWindow, toggleMaximizeWindow } from "../bridge";

interface WindowChromeProps {
  onSearch: () => void;
}

export function WindowChrome({ onSearch }: WindowChromeProps) {
  void onSearch;
  return (
    <header className="topbar window-chrome" data-tauri-drag-region>
      <div className="tb-brand" data-tauri-drag-region>
        <span className="name">Atlas</span>
      </div>
      <div className="menubar" data-tauri-drag-region>
        <button className="menu-btn" type="button">文件</button>
        <button className="menu-btn" type="button">编辑</button>
        <button className="menu-btn" type="button">查看</button>
        <button className="menu-btn" type="button">帮助</button>
      </div>
      <div className="chrome-actions">
        <button className="tb-btn" type="button" aria-label="最小化" onClick={() => void minimizeWindow()}>—</button>
        <button className="tb-btn" type="button" aria-label="最大化" onClick={() => void toggleMaximizeWindow()}>□</button>
        <button className="tb-btn danger" type="button" aria-label="关闭" onClick={() => void closeWindow()}>×</button>
      </div>
    </header>
  );
}
