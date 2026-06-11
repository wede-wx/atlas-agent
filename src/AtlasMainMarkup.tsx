export function AtlasMainMarkup() {
  return (
<>
    <div className={"app"} id={"app"}>
      <header className={"topbar"}>
        <button className={"tb-btn"} id={"navToggle"} title={"切换导航栏"} aria-label={"切换导航栏"}>
          <svg className={"ico"} viewBox={"0 0 20 20"}>
            <rect x={"3"} y={"4"} width={"14"} height={"12"} rx={"2"} />
            <line x1={"8"} y1={"4"} x2={"8"} y2={"16"} />
          </svg>
        </button>
        <div className={"tb-brand"}>
          <span className={"name"}>
            {"Atlas"}
          </span>
        </div>
        <nav className={"menubar"}>
          <div className={"menu"} data-menu>
            <button className={"menu-btn"}>
              {"文件"}
            </button>
            <div className={"menu-pop"}>
              <button className={"menu-row"}>
                {"新建对话"}
                <span className={"sc"}>
                  {"⌘N"}
                </span>
              </button>
              <button className={"menu-row"}>
                {"打开项目…"}
                <span className={"sc"}>
                  {"⌘O"}
                </span>
              </button>
              <div className={"menu-sep"} />
              <button className={"menu-row"}>
                {"导出对话…"}
              </button>
              <button className={"menu-row"}>
                {"关闭窗口"}
                <span className={"sc"}>
                  {"⌘W"}
                </span>
              </button>
            </div>
          </div>
          <div className={"menu"} data-menu>
            <button className={"menu-btn"}>
              {"编辑"}
            </button>
            <div className={"menu-pop"}>
              <button className={"menu-row"}>
                {"撤销"}
                <span className={"sc"}>
                  {"⌘Z"}
                </span>
              </button>
              <button className={"menu-row"}>
                {"重做"}
                <span className={"sc"}>
                  {"⇧⌘Z"}
                </span>
              </button>
              <div className={"menu-sep"} />
              <button className={"menu-row"}>
                {"复制"}
                <span className={"sc"}>
                  {"⌘C"}
                </span>
              </button>
              <button className={"menu-row"}>
                {"粘贴"}
                <span className={"sc"}>
                  {"⌘V"}
                </span>
              </button>
            </div>
          </div>
          <div className={"menu"} data-menu>
            <button className={"menu-btn"}>
              {"查看"}
            </button>
            <div className={"menu-pop"}>
              <button className={"menu-row"}>
                {"切换导航栏"}
                <span className={"sc"}>
                  {"⌘B"}
                </span>
              </button>
              <button className={"menu-row"}>
                {"切换右抽屉"}
                <span className={"sc"}>
                  {"⌘J"}
                </span>
              </button>
              <div className={"menu-sep"} />
              <button className={"menu-row"}>
                {"实际大小"}
                <span className={"sc"}>
                  {"⌘0"}
                </span>
              </button>
            </div>
          </div>
          <div className={"menu"} data-menu>
            <button className={"menu-btn"}>
              {"窗口"}
            </button>
            <div className={"menu-pop"}>
              <button className={"menu-row"}>
                {"最小化"}
                <span className={"sc"}>
                  {"⌘M"}
                </span>
              </button>
              <button className={"menu-row"}>
                {"缩放"}
              </button>
              <button className={"menu-row"}>
                {"进入全屏"}
                <span className={"sc"}>
                  {"⌃⌘F"}
                </span>
              </button>
            </div>
          </div>
          <div className={"menu"} data-menu>
            <button className={"menu-btn"}>
              {"帮助"}
            </button>
            <div className={"menu-pop"}>
              <button className={"menu-row"}>
                {"跳转 GitHub\n            "}
                <span className={"sc"}>
                  <svg viewBox={"0 0 20 20"} width={"13"} height={"13"} fill={"none"} stroke={"currentColor"} strokeWidth={"1.5"} strokeLinecap={"round"} strokeLinejoin={"round"}>
                    <path d={"M8 6h6v6"} />
                    <path d={"M14 6 7 13"} />
                  </svg>
                </span>
              </button>
              <div className={"menu-sub"} data-sub>
                <button className={"menu-row"}>
                  {"联系"}
                  <span className={"sc"}>
                    {"›"}
                  </span>
                </button>
                <div className={"menu-flyout"}>
                  <div className={"contact"}>
                    <div className={"contact-label"}>
                      {"邮箱"}
                    </div>
                    <div className={"contact-val"}>
                      <a href={"/cdn-cgi/l/email-protection"} className={"__cf_email__"} data-cfemail={"0f676a6363604f6e7b636e7c226e686a617b216b6a79"}>
                        {"[email"}
                        {"&#160;"}
                        {"protected]"}
                      </a>
                    </div>
                  </div>
                  <div className={"contact"}>
                    <div className={"contact-label"}>
                      {"微信"}
                    </div>
                    <div className={"qr"} role={"img"} aria-label={"微信二维码占位"}>
                      {"微信二维码"}
                    </div>
                  </div>
                </div>
              </div>
              <div className={"menu-sep"} />
              <button className={"menu-row"}>
                {"关于 Atlas"}
              </button>
              <button className={"menu-row"}>
                {"源许可证"}
              </button>
            </div>
          </div>
        </nav>
        <div className={"tb-session"} data-tauri-drag-region>
          {"抽取登录表单校验 hook"}
        </div>
        <div className={"tb-right"}>
          <button className={"tb-btn"} id={"themeToggle"} title={"切换主题"} aria-label={"切换主题"}>
            <svg className={"ico"} id={"themeIcon"} viewBox={"0 0 20 20"}>
              <path d={"M16 11.5A6 6 0 0 1 8.5 4a6 6 0 1 0 7.5 7.5Z"} />
            </svg>
          </button>
          <button className={"tb-btn"} id={"drawerToggle"} title={"右侧抽屉"} aria-label={"右侧抽屉"}>
            <svg className={"ico"} viewBox={"0 0 20 20"}>
              <rect x={"3"} y={"4"} width={"14"} height={"12"} rx={"2"} />
              <line x1={"13"} y1={"4"} x2={"13"} y2={"16"} />
            </svg>
          </button>
          <div className={"tb-divider"} />
          <div className={"win-ctrls"}>
            <button type={"button"} className={"tb-btn"} title={"最小化"} aria-label={"最小化"}>
              <svg className={"ico ico-sm"} viewBox={"0 0 20 20"}>
                <line x1={"5"} y1={"10"} x2={"15"} y2={"10"} />
              </svg>
            </button>
            <button type={"button"} className={"tb-btn"} title={"最大化"} aria-label={"最大化"}>
              <svg className={"ico ico-sm"} viewBox={"0 0 20 20"}>
                <rect x={"5"} y={"5"} width={"10"} height={"10"} rx={"1.5"} />
              </svg>
            </button>
            <button type={"button"} className={"tb-btn"} title={"关闭"} aria-label={"关闭"}>
              <svg className={"ico ico-sm"} viewBox={"0 0 20 20"}>
                <line x1={"5.5"} y1={"5.5"} x2={"14.5"} y2={"14.5"} />
                <line x1={"14.5"} y1={"5.5"} x2={"5.5"} y2={"14.5"} />
              </svg>
            </button>
          </div>
        </div>
      </header>
      <div className={"body"}>
        <aside className={"nav"}>
          <div className={"nav-scroll"}>
            <div className={"nav-section"}>
              <button className={"nav-item"}>
                <svg className={"ico"} viewBox={"0 0 20 20"}>
                  <path d={"M4 6.5A2.5 2.5 0 0 1 6.5 4h7A2.5 2.5 0 0 1 16 6.5v4A2.5 2.5 0 0 1 13.5 13H8l-3.2 2.6a.5.5 0 0 1-.8-.4Z"} />
                  <line x1={"10"} y1={"6.6"} x2={"10"} y2={"10.4"} />
                  <line x1={"8.1"} y1={"8.5"} x2={"11.9"} y2={"8.5"} />
                </svg>
                <span className={"label"}>
                  {"新对话"}
                </span>
              </button>
              <button className={"nav-item"} id={"searchNavBtn"}>
                <svg className={"ico"} viewBox={"0 0 20 20"}>
                  <circle cx={"9"} cy={"9"} r={"5"} />
                  <line x1={"12.8"} y1={"12.8"} x2={"16"} y2={"16"} />
                </svg>
                <span className={"label"}>
                  {"搜索"}
                </span>
              </button>
            </div>
            <div className={"nav-section"}>
              <button className={"nav-item"}>
                <svg className={"ico"} viewBox={"0 0 20 20"}>
                  <path d={"M5 4.5h7l3 3v8a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5.5a1 1 0 0 1 1-1Z"} />
                  <path d={"M11.5 4.5V8H15"} />
                </svg>
                <span className={"label"}>
                  {"知识库"}
                </span>
              </button>
              <button className={"nav-item"}>
                <svg className={"ico"} viewBox={"0 0 20 20"}>
                  <path d={"M8 4h4v2.2a1 1 0 0 0 1.5.9 2 2 0 1 1 0 3.4 1 1 0 0 0-1.5.9V16H8v-2.4a1 1 0 0 0-1.5-.9 2 2 0 1 1 0-3.4A1 1 0 0 0 8 8.4Z"} />
                </svg>
                <span className={"label"}>
                  {"插件"}
                </span>
              </button>
              <button className={"nav-item"}>
                <svg className={"ico"} viewBox={"0 0 20 20"}>
                  <path d={"M4 16V9"} />
                  <path d={"M8 16V5"} />
                  <path d={"M12 16v-5"} />
                  <path d={"M16 16V7"} />
                </svg>
                <span className={"label"}>
                  {"防线测试"}
                </span>
              </button>
            </div>
            <div className={"nav-group"}>
              {"项目 "}
              <span className={"add"}>
                <svg className={"ico ico-sm"} viewBox={"0 0 20 20"}>
                  <line x1={"10"} y1={"5"} x2={"10"} y2={"15"} />
                  <line x1={"5"} y1={"10"} x2={"15"} y2={"10"} />
                </svg>
              </span>
            </div>
            <div className={"nav-section"}>
              <div className={"proj open"} data-proj>
                <button className={"nav-item proj-toggle"}>
                  <svg className={"ico ico-sm chev"} viewBox={"0 0 20 20"}>
                    <path d={"M8 6l4 4-4 4"} />
                  </svg>
                  <svg className={"ico"} viewBox={"0 0 20 20"}>
                    <path d={"M3.5 6.5A1.5 1.5 0 0 1 5 5h2.6l1.3 1.4H15a1.5 1.5 0 0 1 1.5 1.5v6A1.5 1.5 0 0 1 15 15.5H5a1.5 1.5 0 0 1-1.5-1.5Z"} />
                  </svg>
                  <span className={"label"}>
                    {"payments-web"}
                  </span>
                </button>
                <div className={"proj-children"}>
                  <button className={"nav-item selected"}>
                    <span className={"dot"} />
                    <span className={"label"}>
                      {"抽取登录表单校验 hook"}
                    </span>
                  </button>
                  <button className={"subagent-node"} id={"subagentNode"}>
                    <span className={"sa-glyph"}>
                      <svg viewBox={"0 0 20 20"} fill={"none"} stroke={"currentColor"} strokeWidth={"1.8"} strokeLinecap={"round"} strokeLinejoin={"round"}>
                        <circle cx={"10"} cy={"7"} r={"3"} />
                        <path d={"M4.5 16c0-3 2.5-4.5 5.5-4.5s5.5 1.5 5.5 4.5"} />
                      </svg>
                    </span>
                    <span className={"label"}>
                      {"Volta"}
                    </span>
                    <span className={"sa-role"}>
                      {"explorer"}
                    </span>
                  </button>
                  <button className={"nav-item"}>
                    <span className={"dot"} />
                    <span className={"label"}>
                      {"迁移到 React Query"}
                    </span>
                  </button>
                  <button className={"nav-item"}>
                    <span className={"dot"} />
                    <span className={"label"}>
                      {"修复退款边界 case"}
                    </span>
                  </button>
                </div>
              </div>
              <div className={"proj"} data-proj>
                <button className={"nav-item proj-toggle"}>
                  <svg className={"ico ico-sm chev"} viewBox={"0 0 20 20"}>
                    <path d={"M8 6l4 4-4 4"} />
                  </svg>
                  <svg className={"ico"} viewBox={"0 0 20 20"}>
                    <path d={"M3.5 6.5A1.5 1.5 0 0 1 5 5h2.6l1.3 1.4H15a1.5 1.5 0 0 1 1.5 1.5v6A1.5 1.5 0 0 1 15 15.5H5a1.5 1.5 0 0 1-1.5-1.5Z"} />
                  </svg>
                  <span className={"label"}>
                    {"infra-tooling"}
                  </span>
                </button>
                <div className={"proj-children"}>
                  <button className={"nav-item"}>
                    <span className={"dot"} />
                    <span className={"label"}>
                      {"CI 缓存命中率"}
                    </span>
                  </button>
                  <button className={"nav-item"}>
                    <span className={"dot"} />
                    <span className={"label"}>
                      {"日志采样规则"}
                    </span>
                  </button>
                </div>
              </div>
              <div className={"proj"} data-proj>
                <button className={"nav-item proj-toggle"}>
                  <svg className={"ico ico-sm chev"} viewBox={"0 0 20 20"}>
                    <path d={"M8 6l4 4-4 4"} />
                  </svg>
                  <svg className={"ico"} viewBox={"0 0 20 20"}>
                    <path d={"M3.5 6.5A1.5 1.5 0 0 1 5 5h2.6l1.3 1.4H15a1.5 1.5 0 0 1 1.5 1.5v6A1.5 1.5 0 0 1 15 15.5H5a1.5 1.5 0 0 1-1.5-1.5Z"} />
                  </svg>
                  <span className={"label"}>
                    {"data-pipeline"}
                  </span>
                </button>
                <div className={"proj-children"}>
                  <button className={"nav-item"}>
                    <span className={"dot"} />
                    <span className={"label"}>
                      {"回填脚本审查"}
                    </span>
                  </button>
                </div>
              </div>
            </div>
            <div className={"nav-group"}>
              {"对话"}
            </div>
            <div className={"nav-section"}>
              <button className={"nav-item"}>
                <span className={"label"}>
                  {"重构通知中心组件"}
                </span>
                <span className={"time"}>
                  {"2小时前"}
                </span>
              </button>
              <button className={"nav-item"}>
                <span className={"label"}>
                  {"为定价表写单元测试"}
                </span>
                <span className={"time"}>
                  {"昨天"}
                </span>
              </button>
              <button className={"nav-item"}>
                <span className={"label"}>
                  {"排查内存泄漏"}
                </span>
                <span className={"time"}>
                  {"周二"}
                </span>
              </button>
              <button className={"nav-item"}>
                <span className={"label"}>
                  {"起草发布说明 v2.4"}
                </span>
                <span className={"time"}>
                  {"5天前"}
                </span>
              </button>
            </div>
          </div>
          <div className={"nav-foot"}>
            <button className={"nav-item"}>
              <svg className={"ico"} viewBox={"0 0 20 20"}>
                <circle cx={"10"} cy={"10"} r={"2.4"} />
                <path d={"M10 3.5v1.6M10 14.9v1.6M16.5 10h-1.6M5.1 10H3.5M14.6 5.4l-1.1 1.1M6.5 13.5l-1.1 1.1M14.6 14.6l-1.1-1.1M6.5 6.5 5.4 5.4"} />
              </svg>
              <span className={"label"}>
                {"设置"}
              </span>
            </button>
          </div>
        </aside>
        <main className={"main"}>
          <div className={"stream"}>
            <div className={"thread"}>
              <div className={"msg-user"}>
                <div className={"bubble"}>
                  {"帮我把登录页的表单校验逻辑抽成一个独立的 hook，跑一下测试确认没有回归。"}
                </div>
                <div className={"msg-stamp"}>
                  {"14:32"}
                </div>
              </div>
              <div className={"msg-ai"}>
                <div className={"ai-body"}>
                  <p>
                    {"好的。动手之前我想先摸清现有代码——让一个探索子代理去查 "}
                    <code className={"inl"}>
                      {"auth"}
                    </code>
                    {" 目录的结构和依赖，我再据此规划。"}
                  </p>
                </div>
              </div>
              <button className={"sa-create"} id={"saCreateNode"}>
                <span className={"sa-create-glyph"}>
                  <svg viewBox={"0 0 20 20"} fill={"none"} stroke={"currentColor"} strokeWidth={"1.6"} strokeLinecap={"round"} strokeLinejoin={"round"}>
                    <circle cx={"10"} cy={"7"} r={"3.1"} />
                    <path d={"M4.3 16c0-3.1 2.6-4.7 5.7-4.7s5.7 1.6 5.7 4.7"} />
                  </svg>
                </span>
                <span className={"sa-create-main"}>
                  <span className={"sa-create-line"}>
                    <span className={"lead"}>
                      {"已创建子代理"}
                    </span>
                    <span className={"nm"}>
                      {"Volta"}
                    </span>
                    <span className={"sa-create-role"}>
                      {"explorer · 探索"}
                    </span>
                  </span>
                  <span className={"sa-create-task"}>
                    {"任务：探查 auth 目录结构 "}
                    <span className={"ro"}>
                      {"· 只读探查、不得修改文件"}
                    </span>
                  </span>
                </span>
                <span className={"sa-create-status done"}>
                  <span className={"dot"} />
                  {"完成"}
                </span>
                <span className={"sa-create-arrow"}>
                  <svg className={"ico ico-sm"} viewBox={"0 0 20 20"}>
                    <path d={"M8 6l4 4-4 4"} />
                  </svg>
                </span>
              </button>
              <div className={"msg-ai"}>
                <div className={"ai-body"}>
                  <p>
                    {"Volta 回来了：校验逻辑内联在 "}
                    <code className={"inl"}>
                      {"LoginForm.tsx"}
                    </code>
                    {"，"}
                    <code className={"inl"}>
                      {"validators.ts"}
                    </code>
                    {" 是可复用的纯函数，无布局/API 依赖。先跑测试确认基线，再抽取到 "}
                    <code className={"inl"}>
                      {"useFormValidation.ts"}
                    </code>
                    {"。"}
                  </p>
                </div>
              </div>
              <div className={"tool open"} data-tool>
                <button className={"tool-bar"}>
                  <svg className={"ico ico-sm"} viewBox={"0 0 20 20"}>
                    <path d={"M5 7l3 3-3 3"} />
                    <line x1={"10"} y1={"14"} x2={"15"} y2={"14"} />
                  </svg>
                  <span>
                    {"已运行命令"}
                  </span>
                  <svg className={"ico ico-sm chev"} viewBox={"0 0 20 20"}>
                    <path d={"M6 8l4 4 4-4"} />
                  </svg>
                </button>
                <div className={"tool-panel"}>
                  <div className={"tool-head"}>
                    <span className={"tag"}>
                      {"Shell"}
                    </span>
                    <span>
                      {"~/payments-web"}
                    </span>
                  </div>
                  <div className={"tool-cmd"}>
                    <span className={"prompt"}>
                      {"$ "}
                    </span>
                    {"npm test -- src/features/auth"}
                  </div>
                  <div className={"tool-out"}>
                    {"PASS  src/features/auth/LoginForm.test.tsx\nPASS  src/features/auth/validators.test.ts\n\nTests:       24 passed, 24 total\nTime:        3.18 s"}
                  </div>
                  <div className={"tool-foot"}>
                    <span className={"ok"}>
                      <svg className={"ico ico-sm"} viewBox={"0 0 20 20"} style={{ "strokeWidth": 2 }}>
                        <path d={"M5 10.5l3 3 7-7"} />
                      </svg>
                      {"成功"}
                    </span>
                  </div>
                </div>
              </div>
              <div className={"msg-ai"}>
                <div className={"ai-body"}>
                  <p>
                    {"基线通过。已新建 "}
                    <code className={"inl"}>
                      {"useFormValidation.ts"}
                    </code>
                    {" 并接入 "}
                    <code className={"inl"}>
                      {"LoginForm"}
                    </code>
                    {"。为了让校验错误能在路由层统一兜底，我需要调整 "}
                    <code className={"inl"}>
                      {"src/ui/App.tsx"}
                    </code>
                    {" 的布局容器——"}
                  </p>
                </div>
              </div>
              <div className={"gate"} id={"gate"}>
                <div className={"gate-inner"}>
                  <div className={"gate-top"} id={"gateHead"}>
                    <span className={"badge"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <circle cx={"10"} cy={"10"} r={"6.5"} />
                        <line x1={"5.4"} y1={"5.4"} x2={"14.6"} y2={"14.6"} />
                      </svg>
                      {"\n                  已拦截\n                "}
                    </span>
                    <span className={"gate-title"}>
                      {"试图修改受保护文件"}
                    </span>
                    <span className={"gate-spacer"} />
                    <span className={"gate-result"} id={"gateResult"} />
                    <svg className={"gate-chev"} viewBox={"0 0 20 20"} fill={"none"} stroke={"currentColor"} strokeWidth={"1.6"} strokeLinecap={"round"} strokeLinejoin={"round"}>
                      <path d={"M6 8l4 4 4-4"} />
                    </svg>
                  </div>
                  <div className={"gate-body"}>
                    <div>
                      <div className={"gate-rule"}>
                        <span className={"ctab"}>
                          {"N1"}
                        </span>
                        <span className={"rule-text"}>
                          {"不得改动 src/ui/ 下的布局文件"}
                        </span>
                      </div>
                      <div className={"gate-why"}>
                        {"此动作将修改 "}
                        <span className={"path"}>
                          {"src/ui/App.tsx"}
                        </span>
                      </div>
                    </div>
                    <div className={"diff"}>
                      <div className={"diff-head"}>
                        <span className={"path"} style={{ "fontFamily": "var(--mono)", "color": "var(--text-2)" }}>
                          {"src/ui/App.tsx"}
                        </span>
                        <span>
                          {"·"}
                        </span>
                        <span>
                          {"2 处改动"}
                        </span>
                      </div>
                      <div className={"diff-body"}>
                        <div className={"diff-line ctx"}>
                          {"function App() {"}
                        </div>
                        <div className={"diff-line ctx"}>
                          {"  return ("}
                        </div>
                        <div className={"diff-line del"}>
                          {"&lt;"}
                          {"Layout"}
                          {"&gt;"}
                        </div>
                        <div className={"diff-line add"}>
                          {"&lt;"}
                          {"Layout className=\"with-validation-banner\""}
                          {"&gt;"}
                        </div>
                        <div className={"diff-line ctx"}>
                          {"&lt;"}
                          {"Router /"}
                          {"&gt;"}
                        </div>
                        <div className={"diff-line add"}>
                          {"&lt;"}
                          {"ValidationBoundary /"}
                          {"&gt;"}
                        </div>
                        <div className={"diff-line ctx"}>
                          {"&lt;"}
                          {"/Layout"}
                          {"&gt;"}
                        </div>
                      </div>
                    </div>
                    <div className={"gate-actions"}>
                      <button className={"btn btn-block"} id={"btnReject"}>
                        {"拒绝"}
                      </button>
                      <button className={"btn btn-ghost"} id={"btnAllow"}>
                        {"仅这一次允许"}
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
          <div className={"composer-wrap"}>
            <div className={"composer"}>
              <textarea className={"composer-input"} rows={1} placeholder={"给 Atlas 指令…"} />
              <div className={"composer-bar"}>
                <button className={"comp-btn"} title={"附加"} aria-label={"附加"}>
                  <svg className={"ico"} viewBox={"0 0 20 20"}>
                    <line x1={"10"} y1={"5"} x2={"10"} y2={"15"} />
                    <line x1={"5"} y1={"10"} x2={"15"} y2={"10"} />
                  </svg>
                </button>
                <span className={"comp-spacer"} />
                <button className={"mode"}>
                  <span className={"dot"} />
                  {"默认模式\n              "}
                  <svg className={"ico ico-sm"} viewBox={"0 0 20 20"} style={{ "color": "var(--text-3)" }}>
                    <path d={"M6 8l4 4 4-4"} />
                  </svg>
                </button>
                <button className={"send"} title={"发送"} aria-label={"发送"}>
                  <svg viewBox={"0 0 20 20"} width={"16"} height={"16"} fill={"none"} stroke={"currentColor"} strokeWidth={"1.8"} strokeLinecap={"round"} strokeLinejoin={"round"}>
                    <line x1={"10"} y1={"15.5"} x2={"10"} y2={"5"} />
                    <path d={"M5.5 9.5 10 5l4.5 4.5"} />
                  </svg>
                </button>
              </div>
            </div>
          </div>
        </main>
        <section className={"sa-panel"} data-screen-label={"子代理 Volta"}>
          <div className={"sa-inner"}>
            <button className={"sa-back"} id={"saBack"}>
              <svg className={"ico ico-sm"} viewBox={"0 0 20 20"}>
                <path d={"M12 5l-5 5 5 5"} />
              </svg>
              {"\n          返回对话\n        "}
            </button>
            <div className={"sa-id"}>
              <div className={"sa-id-top"}>
                <div className={"sa-avatar"}>
                  <svg viewBox={"0 0 20 20"} fill={"none"} stroke={"currentColor"} strokeWidth={"1.6"} strokeLinecap={"round"} strokeLinejoin={"round"}>
                    <circle cx={"10"} cy={"7"} r={"3.2"} />
                    <path d={"M4 16.5c0-3.2 2.7-4.8 6-4.8s6 1.6 6 4.8"} />
                  </svg>
                </div>
                <div className={"sa-id-main"}>
                  <div className={"sa-name-line"}>
                    <span className={"sa-name"}>
                      {"Volta"}
                    </span>
                    <span className={"role-tag role-explorer"}>
                      {"explorer · 探索"}
                    </span>
                  </div>
                  <div className={"sa-model"}>
                    {"claude-sonnet-4.5"}
                  </div>
                </div>
                <span className={"sa-status done"}>
                  <span className={"dot"} />
                  {"完成"}
                </span>
              </div>
              <div className={"sa-scope"}>
                <div className={"sa-scope-head"}>
                  <span className={"sa-scope-label"}>
                    {"工具范围"}
                  </span>
                  <span className={"sa-readonly"}>
                    <svg viewBox={"0 0 20 20"} fill={"none"} stroke={"currentColor"} strokeWidth={"1.7"} strokeLinecap={"round"} strokeLinejoin={"round"}>
                      <rect x={"5"} y={"9"} width={"10"} height={"7"} rx={"1.5"} />
                      <path d={"M7 9V7a3 3 0 0 1 6 0v2"} />
                    </svg>
                    {"只读"}
                  </span>
                </div>
                <div className={"sa-tools"}>
                  <span className={"sa-tool"}>
                    <svg viewBox={"0 0 20 20"} fill={"none"} stroke={"currentColor"} strokeWidth={"1.6"} strokeLinecap={"round"} strokeLinejoin={"round"}>
                      <path d={"M6 4.5h5l3 3v8a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V5.5a1 1 0 0 1 1-1Z"} />
                      <path d={"M10.5 4.5V8H14"} />
                    </svg>
                    {"读文件"}
                  </span>
                  <span className={"sa-tool"}>
                    <svg viewBox={"0 0 20 20"} fill={"none"} stroke={"currentColor"} strokeWidth={"1.6"} strokeLinecap={"round"} strokeLinejoin={"round"}>
                      <circle cx={"9"} cy={"9"} r={"5"} />
                      <line x1={"12.8"} y1={"12.8"} x2={"16"} y2={"16"} />
                    </svg>
                    {"搜索"}
                  </span>
                  <span className={"sa-tool"}>
                    <svg viewBox={"0 0 20 20"} fill={"none"} stroke={"currentColor"} strokeWidth={"1.6"} strokeLinecap={"round"} strokeLinejoin={"round"}>
                      <path d={"M5 7l3 3-3 3"} />
                      <line x1={"10"} y1={"14"} x2={"15"} y2={"14"} />
                    </svg>
                    {"grep"}
                  </span>
                </div>
                <div className={"sa-guard"}>
                  <svg viewBox={"0 0 20 20"} fill={"none"} stroke={"currentColor"} strokeWidth={"1.6"} strokeLinecap={"round"} strokeLinejoin={"round"}>
                    <path d={"M10 3l5 2v4.5c0 3.2-2.2 5.6-5 6.5-2.8-.9-5-3.3-5-6.5V5z"} />
                  </svg>
                  <span>
                    {"子代理在主 agent 的护栏之内运行，不能获得额外权限或关闭安全检查。"}
                  </span>
                </div>
              </div>
            </div>
            <div className={"sa-h2"}>
              {"执行过程"}
            </div>
            <div className={"sa-h2-sub"}>
              {"仅限 Volta 自己做的事 · 真实记录回放。"}
            </div>
            <div className={"rt-axis"}>
              <div className={"rt-node step"} data-rtnode>
                <div className={"rt-rail"}>
                  <span className={"rt-dot t-step"} />
                </div>
                <div className={"rt-main"}>
                  <button className={"rt-row"}>
                    <span className={"rt-ico"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <circle cx={"10"} cy={"10"} r={"6.5"} />
                        <path d={"M10 7v3.2l2 1.3"} />
                      </svg>
                    </span>
                    <span className={"rt-label"}>
                      {"接到交接，开始探查 auth 目录"}
                    </span>
                    <span className={"rt-dur"}>
                      {"0.3s"}
                    </span>
                    <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                      <path d={"M6 8l4 4 4-4"} />
                    </svg>
                  </button>
                  <div className={"rt-detail"}>
                    <div className={"rt-replay"}>
                      <span className={"dotmark"} />
                      {"真实记录回放"}
                    </div>
                    <div className={"rt-code"}>
                      {"收到契约：探查 src/features/auth 的结构与依赖\n范围：只读，不修改任何文件\n计划：列目录 → 读关键文件 → grep 校验函数"}
                    </div>
                  </div>
                </div>
              </div>
              <div className={"rt-node tool"} data-rtnode>
                <div className={"rt-rail"}>
                  <span className={"rt-dot t-tool"} />
                </div>
                <div className={"rt-main"}>
                  <button className={"rt-row"}>
                    <span className={"rt-ico"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <path d={"M5 4.5h7l3 3v8a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5.5a1 1 0 0 1 1-1Z"} />
                        <path d={"M11.5 4.5V8H15"} />
                      </svg>
                    </span>
                    <span className={"rt-label"}>
                      <span className={"rt-toolname"}>
                        {"read"}
                      </span>
                      {" LoginForm.tsx"}
                    </span>
                    <span className={"rt-chip ok"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <path d={"M5 10.5l3 3 7-7"} />
                      </svg>
                    </span>
                    <span className={"rt-dur"}>
                      {"0.2s"}
                    </span>
                    <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                      <path d={"M6 8l4 4 4-4"} />
                    </svg>
                  </button>
                  <div className={"rt-detail"}>
                    <div className={"rt-replay"}>
                      <span className={"dotmark"} />
                      {"真实记录回放"}
                    </div>
                    <div className={"rt-code"}>
                      {"读取 src/features/auth/LoginForm.tsx · 148 行\n发现内联校验逻辑 validateEmail / validatePassword\n依赖：react, ./validators, ../../ui/Field"}
                    </div>
                  </div>
                </div>
              </div>
              <div className={"rt-node tool"} data-rtnode>
                <div className={"rt-rail"}>
                  <span className={"rt-dot t-tool"} />
                </div>
                <div className={"rt-main"}>
                  <button className={"rt-row"}>
                    <span className={"rt-ico"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <path d={"M5 4.5h7l3 3v8a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5.5a1 1 0 0 1 1-1Z"} />
                        <path d={"M11.5 4.5V8H15"} />
                      </svg>
                    </span>
                    <span className={"rt-label"}>
                      <span className={"rt-toolname"}>
                        {"read"}
                      </span>
                      {" validators.ts"}
                    </span>
                    <span className={"rt-chip ok"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <path d={"M5 10.5l3 3 7-7"} />
                      </svg>
                    </span>
                    <span className={"rt-dur"}>
                      {"0.2s"}
                    </span>
                    <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                      <path d={"M6 8l4 4 4-4"} />
                    </svg>
                  </button>
                  <div className={"rt-detail"}>
                    <div className={"rt-replay"}>
                      <span className={"dotmark"} />
                      {"真实记录回放"}
                    </div>
                    <div className={"rt-code"}>
                      {"读取 src/features/auth/validators.ts · 36 行\n导出 4 个纯函数，无副作用\n可直接被新 hook 复用"}
                    </div>
                  </div>
                </div>
              </div>
              <div className={"rt-node tool"} data-rtnode>
                <div className={"rt-rail"}>
                  <span className={"rt-dot t-tool"} />
                </div>
                <div className={"rt-main"}>
                  <button className={"rt-row"}>
                    <span className={"rt-ico"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <path d={"M5 7l3 3-3 3"} />
                        <line x1={"10"} y1={"14"} x2={"15"} y2={"14"} />
                      </svg>
                    </span>
                    <span className={"rt-label"}>
                      <span className={"rt-toolname"}>
                        {"grep"}
                      </span>
                      {" \"validate\" src/features/auth"}
                    </span>
                    <span className={"rt-chip ok"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <path d={"M5 10.5l3 3 7-7"} />
                      </svg>
                    </span>
                    <span className={"rt-dur"}>
                      {"0.4s"}
                    </span>
                    <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                      <path d={"M6 8l4 4 4-4"} />
                    </svg>
                  </button>
                  <div className={"rt-detail"}>
                    <div className={"rt-replay"}>
                      <span className={"dotmark"} />
                      {"真实记录回放"}
                    </div>
                    <div className={"rt-code"}>
                      {"grep -rn \"validate\" src/features/auth\nLoginForm.tsx:42  validateEmail(value)\nLoginForm.tsx:58  validatePassword(value)\nvalidators.ts:3   export function validateEmail\nvalidators.ts:12  export function validatePassword\n命中 4 处，集中在两个文件"}
                    </div>
                  </div>
                </div>
              </div>
              <div className={"rt-node step"} data-rtnode>
                <div className={"rt-rail"}>
                  <span className={"rt-dot t-step"} />
                </div>
                <div className={"rt-main"}>
                  <button className={"rt-row"}>
                    <span className={"rt-ico"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <circle cx={"10"} cy={"10"} r={"6.5"} />
                        <path d={"M10 7v3.2l2 1.3"} />
                      </svg>
                    </span>
                    <span className={"rt-label"}>
                      {"汇报目录结构与依赖关系"}
                    </span>
                    <span className={"rt-dur"}>
                      {"0.5s"}
                    </span>
                    <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                      <path d={"M6 8l4 4 4-4"} />
                    </svg>
                  </button>
                  <div className={"rt-detail"}>
                    <div className={"rt-replay"}>
                      <span className={"dotmark"} />
                      {"真实记录回放"}
                    </div>
                    <div className={"rt-code"}>
                      {"整理结论：\n- auth/ 下 2 个相关文件：LoginForm.tsx、validators.ts\n- 校验逻辑内联在 LoginForm，可抽到 hook\n- validators.ts 为纯函数，hook 可直接复用\n- 无外部布局/API 依赖，改动可控\n交回主 agent。"}
                    </div>
                  </div>
                </div>
              </div>
            </div>
            <div className={"sa-h2"}>
              {"交接"}
            </div>
            <div className={"sa-h2-sub"}>
              {"Volta 与主 agent 之间的职责移交。"}
            </div>
            <div className={"handoff"}>
              <div className={"ho-card"}>
                <div className={"ho-dir"}>
                  <span className={"ho-party is-main"}>
                    <span className={"pdot"} />
                    {"主 agent"}
                  </span>
                  <span className={"ho-arrow"}>
                    <svg viewBox={"0 0 20 20"} fill={"none"} stroke={"currentColor"} strokeWidth={"1.6"} strokeLinecap={"round"} strokeLinejoin={"round"}>
                      <line x1={"4"} y1={"10"} x2={"15"} y2={"10"} />
                      <path d={"M11.5 6.5L15 10l-3.5 3.5"} />
                    </svg>
                  </span>
                  <span className={"ho-party explorer"}>
                    <span className={"pdot"} />
                    {"Volta · explorer"}
                  </span>
                </div>
                <div className={"ho-fields"}>
                  <div className={"ho-field"}>
                    <span className={"k"}>
                      {"任务"}
                    </span>
                    <span className={"v"}>
                      {"探查 auth 目录结构"}
                    </span>
                  </div>
                  <div className={"ho-field"}>
                    <span className={"k"}>
                      {"原因"}
                    </span>
                    <span className={"v"}>
                      {"需要先了解现有代码，再规划如何抽取 hook。"}
                    </span>
                  </div>
                  <div className={"ho-field"}>
                    <span className={"k"}>
                      {"约定"}
                    </span>
                    <span className={"v contract"}>
                      {"只读探查 · 不得修改文件 · 仅返回结构与依赖摘要"}
                    </span>
                  </div>
                </div>
              </div>
              <div className={"ho-card"}>
                <div className={"ho-dir"}>
                  <span className={"ho-party explorer"}>
                    <span className={"pdot"} />
                    {"Volta · explorer"}
                  </span>
                  <span className={"ho-arrow"}>
                    <svg viewBox={"0 0 20 20"} fill={"none"} stroke={"currentColor"} strokeWidth={"1.6"} strokeLinecap={"round"} strokeLinejoin={"round"}>
                      <line x1={"4"} y1={"10"} x2={"15"} y2={"10"} />
                      <path d={"M11.5 6.5L15 10l-3.5 3.5"} />
                    </svg>
                  </span>
                  <span className={"ho-party is-main"}>
                    <span className={"pdot"} />
                    {"主 agent"}
                  </span>
                </div>
                <div className={"ho-fields"}>
                  <div className={"ho-field"}>
                    <span className={"k"}>
                      {"交回"}
                    </span>
                    <span className={"v"}>
                      <span className={"ho-result-line"}>
                        <span className={"ok"}>
                          <svg viewBox={"0 0 20 20"} fill={"none"} stroke={"currentColor"}>
                            <path d={"M5 10.5l3 3 7-7"} />
                          </svg>
                        </span>
                        {"auth 目录的文件清单与依赖关系"}
                      </span>
                    </span>
                  </div>
                  <div className={"ho-field"}>
                    <span className={"k"}>
                      {"结果"}
                    </span>
                    <span className={"v contract"}>
                      {"2 个文件：LoginForm.tsx、validators.ts\n校验逻辑内联可抽取 · validators 为纯函数可复用 · 无布局/API 依赖"}
                    </span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </section>
        <aside className={"drawer"}>
          <div className={"drawer-tabs"}>
            <button className={"dtab active"} data-tab={"contract"}>
              {"目标契约"}
            </button>
            <button className={"dtab"} data-tab={"timeline"}>
              {"运行时间线"}
            </button>
            <button className={"dtab"} data-tab={"graph"}>
              {"运行图"}
            </button>
          </div>
          <div className={"drawer-scroll"}>
            <div className={"drawer-inner view-contract"} data-view={"contract"}>
              <div className={"gc-head"}>
                <span className={"gc-title"}>
                  {"目标契约"}
                </span>
                <span className={"gc-spacer"} />
                <span className={"gc-status pending"} id={"gcStatus"}>
                  {"待确认"}
                </span>
              </div>
              <div className={"gc-goal"}>
                <span className={"ql"}>
                  {"这一轮的目标"}
                </span>
                {"\n            将登录表单校验逻辑抽取为独立 hook，并确保测试无回归。\n          "}
              </div>
              <div className={"gc-confirm"}>
                <span className={"ct"}>
                  {"确认后契约将被冻结，作为本轮对照的基准。"}
                </span>
                <button className={"btn btn-block"} id={"gcFreeze"} style={{ "padding": "7px 13px", "fontSize": "12px" }}>
                  {"确认冻结"}
                </button>
              </div>
              <div className={"gc-group"}>
                <div className={"gc-glabel"}>
                  {"必须做 "}
                  <span className={"cnt"}>
                    {"Must · 3"}
                  </span>
                </div>
                <div className={"gc-item must"}>
                  <span className={"gc-id hard"}>
                    {"M1"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"将校验逻辑抽取到 useFormValidation.ts 并被 LoginForm 引用。"}
                    </div>
                  </div>
                </div>
                <div className={"gc-item must"}>
                  <span className={"gc-id hard"}>
                    {"M2"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"保留现有的错误文案与字段校验顺序。"}
                    </div>
                  </div>
                </div>
                <div className={"gc-item must soft-item"}>
                  <span className={"gc-id soft"}>
                    {"M3"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"为新 hook 补充单元测试。"}
                    </div>
                    <div className={"gc-sub"}>
                      <span className={"gc-soft-tag"}>
                        {"软性 · 可商量"}
                      </span>
                    </div>
                  </div>
                </div>
              </div>
              <div className={"gc-group"}>
                <div className={"gc-glabel"}>
                  {"不可做 "}
                  <span className={"cnt"}>
                    {"Must Not · 3"}
                  </span>
                </div>
                <div className={"gc-item not"}>
                  <span className={"gc-id hard"}>
                    {"N1"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"不得改动 src/ui/ 下的布局文件。"}
                    </div>
                  </div>
                </div>
                <div className={"gc-item not"}>
                  <span className={"gc-id hard"}>
                    {"N2"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"不得修改登录接口的请求/响应结构。"}
                    </div>
                  </div>
                </div>
                <div className={"gc-item not soft-item"}>
                  <span className={"gc-id soft"}>
                    {"N3"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"避免引入新的第三方校验库。"}
                    </div>
                    <div className={"gc-sub"}>
                      <span className={"gc-soft-tag"}>
                        {"软性 · 可商量"}
                      </span>
                    </div>
                  </div>
                </div>
              </div>
              <div className={"gc-group"}>
                <div className={"gc-glabel"}>
                  {"需保护 "}
                  <span className={"cnt"}>
                    {"Preserve · 4"}
                  </span>
                </div>
                <div className={"gc-item"}>
                  <span className={"gc-id hard"}>
                    {"P1"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"LoginForm 对外暴露的 props 接口。"}
                    </div>
                    <div className={"gc-sub"}>
                      <span className={"gc-type"}>
                        {"API"}
                      </span>
                    </div>
                  </div>
                </div>
                <div className={"gc-item"}>
                  <span className={"gc-id hard"}>
                    {"P2"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"提交按钮的禁用/加载交互行为。"}
                    </div>
                    <div className={"gc-sub"}>
                      <span className={"gc-type"}>
                        {"行为"}
                      </span>
                    </div>
                  </div>
                </div>
                <div className={"gc-item soft-item"}>
                  <span className={"gc-id soft"}>
                    {"P3"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"表单容器的栅格布局。"}
                    </div>
                    <div className={"gc-sub"}>
                      <span className={"gc-type"}>
                        {"布局"}
                      </span>
                    </div>
                  </div>
                </div>
                <div className={"gc-item soft-item"}>
                  <span className={"gc-id soft"}>
                    {"P4"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"validators.ts 的导出文件路径。"}
                    </div>
                    <div className={"gc-sub"}>
                      <span className={"gc-type"}>
                        {"文件"}
                      </span>
                    </div>
                  </div>
                </div>
              </div>
              <div className={"gc-group"}>
                <div className={"gc-glabel"}>
                  {"约束 "}
                  <span className={"cnt"}>
                    {"Constraints · 2"}
                  </span>
                </div>
                <div className={"gc-item"}>
                  <span className={"gc-id hard"}>
                    {"C1"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"改动控制在 src/features/auth/ 范围内。"}
                    </div>
                  </div>
                </div>
                <div className={"gc-item soft-item"}>
                  <span className={"gc-id soft"}>
                    {"C2"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"单次 diff 不超过 200 行。"}
                    </div>
                    <div className={"gc-sub"}>
                      <span className={"gc-soft-tag"}>
                        {"软性 · 可商量"}
                      </span>
                    </div>
                  </div>
                </div>
              </div>
              <div className={"gc-group"}>
                <div className={"gc-glabel"}>
                  {"验收标准 "}
                  <span className={"cnt"}>
                    {"Acceptance · 4"}
                  </span>
                </div>
                <div className={"gc-item"}>
                  <span className={"gc-id hard"}>
                    {"A1"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"auth 目录下全部测试通过。"}
                    </div>
                  </div>
                  <span className={"gc-dot pass"} title={"通过"} />
                </div>
                <div className={"gc-item"}>
                  <span className={"gc-id hard"}>
                    {"A2"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"useFormValidation 覆盖率 ≥ 90%。"}
                    </div>
                  </div>
                  <span className={"gc-dot pend"} title={"待定"} />
                </div>
                <div className={"gc-item"}>
                  <span className={"gc-id hard"}>
                    {"A3"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"登录页面手动冒烟通过。"}
                    </div>
                  </div>
                  <span className={"gc-dot fail"} title={"失败"} />
                </div>
                <div className={"gc-item soft-item"}>
                  <span className={"gc-id soft"}>
                    {"A4"}
                  </span>
                  <div className={"gc-body"}>
                    <div className={"gc-text"}>
                      {"无新增 ESLint 警告。"}
                    </div>
                  </div>
                  <span className={"gc-dot waived"} title={"已豁免"} />
                </div>
              </div>
              <div className={"gc-scope"}>
                <div className={"gc-glabel"} style={{ "marginBottom": "2px" }}>
                  {"范围边界"}
                </div>
                <div className={"gc-scope-grid"}>
                  <div className={"gc-scope-col in"}>
                    <div className={"sc-h"}>
                      <span className={"dot"} />
                      {"范围内"}
                    </div>
                    <ul>
                      <li>
                        {"src/features/auth/ 下的源码"}
                      </li>
                      <li>
                        {"该目录的测试文件"}
                      </li>
                      <li>
                        {"新建的 hook 文件"}
                      </li>
                    </ul>
                  </div>
                  <div className={"gc-scope-col out"}>
                    <div className={"sc-h"}>
                      <span className={"dot"} />
                      {"范围外"}
                    </div>
                    <ul>
                      <li>
                        {"UI 布局与全局样式"}
                      </li>
                      <li>
                        {"后端登录接口"}
                      </li>
                      <li>
                        {"构建与 CI 配置"}
                      </li>
                    </ul>
                  </div>
                </div>
              </div>
            </div>
            <div className={"drawer-inner view-timeline"} data-view={"timeline"} hidden>
              <div className={"rt-head"}>
                <div className={"rt-statusline"}>
                  <span className={"rt-badge running"}>
                    <span className={"pdot"} />
                    {"运行中"}
                  </span>
                  <span className={"rt-mode"}>
                    {"默认模式"}
                  </span>
                </div>
                <div className={"rt-times"}>
                  <span>
                    {"开始 14:32:07"}
                  </span>
                  <span>
                    {"耗时 48.6s"}
                  </span>
                </div>
              </div>
              <div className={"rt-axis"}>
                <div className={"rt-node step"} data-rtnode>
                  <div className={"rt-rail"}>
                    <span className={"rt-dot t-step"} />
                  </div>
                  <div className={"rt-main"}>
                    <button className={"rt-row"}>
                      <span className={"rt-ico"}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <circle cx={"10"} cy={"10"} r={"6.5"} />
                          <path d={"M10 7v3.2l2 1.3"} />
                        </svg>
                      </span>
                      <span className={"rt-label"}>
                        {"规划：确认测试基线并定位校验逻辑"}
                      </span>
                      <span className={"rt-dur"}>
                        {"0.4s"}
                      </span>
                      <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                        <path d={"M6 8l4 4 4-4"} />
                      </svg>
                    </button>
                    <div className={"rt-detail"}>
                      <div className={"rt-replay"}>
                        <span className={"dotmark"} />
                        {"真实记录回放"}
                      </div>
                      <div className={"rt-code"}>
                        {"读取 src/features/auth/LoginForm.tsx\n读取 src/features/auth/validators.ts\n计划：抽取 validate* 函数 → useFormValidation.ts\n先跑测试建立基线，再做改动。"}
                      </div>
                    </div>
                  </div>
                </div>
                <div className={"rt-node tool"} data-rtnode>
                  <div className={"rt-rail"}>
                    <span className={"rt-dot t-tool"} />
                  </div>
                  <div className={"rt-main"}>
                    <button className={"rt-row"}>
                      <span className={"rt-ico"}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <path d={"M5 7l3 3-3 3"} />
                          <line x1={"10"} y1={"14"} x2={"15"} y2={"14"} />
                        </svg>
                      </span>
                      <span className={"rt-label"}>
                        <span className={"rt-toolname"}>
                          {"shell"}
                        </span>
                        {" npm test"}
                      </span>
                      <span className={"rt-chip ok"}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <path d={"M5 10.5l3 3 7-7"} />
                        </svg>
                      </span>
                      <span className={"rt-dur"}>
                        {"3.2s"}
                      </span>
                      <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                        <path d={"M6 8l4 4 4-4"} />
                      </svg>
                    </button>
                    <div className={"rt-detail"}>
                      <div className={"rt-replay"}>
                        <span className={"dotmark"} />
                        {"真实记录回放"}
                      </div>
                      <div className={"rt-code"}>
                        <span className={"prompt"}>
                          {"$ "}
                        </span>
                        {"npm test -- src/features/auth\n"}
                        <span className={"k"}>
                          {"PASS"}
                        </span>
                        {"  src/features/auth/LoginForm.test.tsx\n"}
                        <span className={"k"}>
                          {"PASS"}
                        </span>
                        {"  src/features/auth/validators.test.ts\n\nTests:       "}
                        <span className={"g"}>
                          {"24 passed"}
                        </span>
                        {", 24 total\nTime:        3.18 s"}
                      </div>
                    </div>
                  </div>
                </div>
                <div className={"rt-node usage"} data-rtnode>
                  <div className={"rt-rail"}>
                    <span className={"rt-dot t-usage"} />
                  </div>
                  <div className={"rt-main"}>
                    <button className={"rt-row"}>
                      <span className={"rt-ico"}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <circle cx={"10"} cy={"10"} r={"6.5"} />
                          <path d={"M10 6v4l2.5 1.5"} />
                        </svg>
                      </span>
                      <span className={"rt-label"}>
                        {"用量 · 4.2K tokens（in 3.1K / out 1.1K）"}
                      </span>
                      <span className={"rt-dur"}>
                        {"—"}
                      </span>
                    </button>
                  </div>
                </div>
                <div className={"rt-node step"} data-rtnode>
                  <div className={"rt-rail"}>
                    <span className={"rt-dot t-step"} />
                  </div>
                  <div className={"rt-main"}>
                    <button className={"rt-row"}>
                      <span className={"rt-ico"}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <circle cx={"10"} cy={"10"} r={"6.5"} />
                          <path d={"M10 7v3.2l2 1.3"} />
                        </svg>
                      </span>
                      <span className={"rt-label"}>
                        {"抽取 useFormValidation.ts 并接入 LoginForm"}
                      </span>
                      <span className={"rt-dur"}>
                        {"5.8s"}
                      </span>
                      <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                        <path d={"M6 8l4 4 4-4"} />
                      </svg>
                    </button>
                    <div className={"rt-detail"}>
                      <div className={"rt-replay"}>
                        <span className={"dotmark"} />
                        {"真实记录回放"}
                      </div>
                      <div className={"rt-code"}>
                        {"新建 src/features/auth/useFormValidation.ts (+62)\n编辑 src/features/auth/LoginForm.tsx (-18 / +6)\n导出 useFormValidation 并替换内联校验。"}
                      </div>
                    </div>
                  </div>
                </div>
                <div className={"rt-node tool"} data-rtnode>
                  <div className={"rt-rail"}>
                    <span className={"rt-dot t-tool"} />
                  </div>
                  <div className={"rt-main"}>
                    <button className={"rt-row"}>
                      <span className={"rt-ico"}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <path d={"M5 4.5h7l3 3v8a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5.5a1 1 0 0 1 1-1Z"} />
                          <path d={"M11.5 4.5V8H15"} />
                        </svg>
                      </span>
                      <span className={"rt-label"}>
                        <span className={"rt-toolname"}>
                          {"write"}
                        </span>
                        {" useFormValidation.ts"}
                      </span>
                      <span className={"rt-chip ok"}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <path d={"M5 10.5l3 3 7-7"} />
                        </svg>
                      </span>
                      <span className={"rt-dur"}>
                        {"0.2s"}
                      </span>
                      <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                        <path d={"M6 8l4 4 4-4"} />
                      </svg>
                    </button>
                    <div className={"rt-detail"}>
                      <div className={"rt-replay"}>
                        <span className={"dotmark"} />
                        {"真实记录回放"}
                      </div>
                      <div className={"rt-code"}>
                        {"写入 useFormValidation.ts · 62 行\n"}
                        <span className={"g"}>
                          {"+ export function useFormValidation(...)"}
                        </span>
                        <span className={"g"}>
                          {"+   const [errors, setErrors] = useState({})"}
                        </span>
                        <span className={"g"}>
                          {"+   ..."}
                        </span>
                      </div>
                    </div>
                  </div>
                </div>
                <div className={"rt-node perm"} data-rtnode>
                  <div className={"rt-rail"}>
                    <span className={"rt-dot t-perm block"} />
                  </div>
                  <div className={"rt-main"}>
                    <button className={"rt-row"}>
                      <span className={"rt-ico"}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <circle cx={"10"} cy={"10"} r={"6.5"} />
                          <line x1={"5.4"} y1={"5.4"} x2={"14.6"} y2={"14.6"} />
                        </svg>
                      </span>
                      <span className={"rt-label"}>
                        {"权限裁决 · 试图修改受保护文件"}
                      </span>
                      <span className={"rt-chip fail"}>
                        {"已拦截"}
                      </span>
                      <span className={"rt-dur"}>
                        {"0.1s"}
                      </span>
                      <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                        <path d={"M6 8l4 4 4-4"} />
                      </svg>
                    </button>
                    <div className={"rt-detail"}>
                      <div className={"rt-replay"}>
                        <span className={"dotmark"} />
                        {"真实记录回放"}
                      </div>
                      <div className={"rt-code"}>
                        {"规则 "}
                        <span className={"r"}>
                          {"N1"}
                        </span>
                        {" · 不得改动 src/ui/ 下的布局文件\n动作：编辑 "}
                        <span className={"r"}>
                          {"src/ui/App.tsx"}
                        </span>
                        {"\n裁决："}
                        <span className={"r"}>
                          {"BLOCK"}
                        </span>
                        {" — 已拦截，动作被撤销\n（对应对话流中的 Block 卡片）"}
                      </div>
                    </div>
                  </div>
                </div>
                <div className={"rt-node step"} data-rtnode>
                  <div className={"rt-rail"}>
                    <span className={"rt-dot t-step"} />
                  </div>
                  <div className={"rt-main"}>
                    <button className={"rt-row"}>
                      <span className={"rt-ico"}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <circle cx={"10"} cy={"10"} r={"6.5"} />
                          <path d={"M10 7v3.2l2 1.3"} />
                        </svg>
                      </span>
                      <span className={"rt-label"}>
                        {"改走兜底方案，不触碰 ui/ 布局"}
                      </span>
                      <span className={"rt-dur"}>
                        {"2.1s"}
                      </span>
                      <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                        <path d={"M6 8l4 4 4-4"} />
                      </svg>
                    </button>
                    <div className={"rt-detail"}>
                      <div className={"rt-replay"}>
                        <span className={"dotmark"} />
                        {"真实记录回放"}
                      </div>
                      <div className={"rt-code"}>
                        {"放弃修改 App.tsx。\n改为在 LoginForm 内部处理校验边界。\n保持 src/ui/ 不变。"}
                      </div>
                    </div>
                  </div>
                </div>
                <div className={"rt-node tool"} data-rtnode>
                  <div className={"rt-rail"}>
                    <span className={"rt-dot t-tool"} />
                  </div>
                  <div className={"rt-main"}>
                    <button className={"rt-row"}>
                      <span className={"rt-ico"}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <path d={"M5 7l3 3-3 3"} />
                          <line x1={"10"} y1={"14"} x2={"15"} y2={"14"} />
                        </svg>
                      </span>
                      <span className={"rt-label"}>
                        <span className={"rt-toolname"}>
                          {"shell"}
                        </span>
                        {" npm test（复跑）"}
                      </span>
                      <span className={"rt-chip ok"}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <path d={"M5 10.5l3 3 7-7"} />
                        </svg>
                      </span>
                      <span className={"rt-dur"}>
                        {"3.0s"}
                      </span>
                      <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                        <path d={"M6 8l4 4 4-4"} />
                      </svg>
                    </button>
                    <div className={"rt-detail"}>
                      <div className={"rt-replay"}>
                        <span className={"dotmark"} />
                        {"真实记录回放"}
                      </div>
                      <div className={"rt-code"}>
                        <span className={"prompt"}>
                          {"$ "}
                        </span>
                        {"npm test -- src/features/auth\n"}
                        <span className={"k"}>
                          {"PASS"}
                        </span>
                        {"  src/features/auth/useFormValidation.test.ts\n"}
                        <span className={"k"}>
                          {"PASS"}
                        </span>
                        {"  src/features/auth/LoginForm.test.tsx\n\nTests:       "}
                        <span className={"g"}>
                          {"27 passed"}
                        </span>
                        {", 27 total"}
                      </div>
                    </div>
                  </div>
                </div>
                <div className={"rt-node verify"} data-rtnode>
                  <div className={"rt-rail"}>
                    <span className={"rt-dot t-verify ok"} />
                  </div>
                  <div className={"rt-main"}>
                    <button className={"rt-row"}>
                      <span className={"rt-ico"} style={{ "color": "var(--allow)" }}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <path d={"M10 3l5 2v4.5c0 3.2-2.2 5.6-5 6.5-2.8-.9-5-3.3-5-6.5V5z"} />
                          <path d={"M8 10l1.5 1.5L13 8"} />
                        </svg>
                      </span>
                      <span className={"rt-label"}>
                        {"验证通过 · 验收标准 A1"}
                      </span>
                      <span className={"rt-vid"}>
                        {"VF-3A1"}
                      </span>
                      <span className={"rt-dur"}>
                        {"3.6s"}
                      </span>
                      <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                        <path d={"M6 8l4 4 4-4"} />
                      </svg>
                    </button>
                    <div className={"rt-detail"}>
                      <div className={"rt-replay"}>
                        <span className={"dotmark"} />
                        {"真实记录回放"}
                      </div>
                      <div className={"rt-code"}>
                        {"验证项：auth 目录全部测试通过\n命令：npm test -- src/features/auth\n结果："}
                        <span className={"g"}>
                          {"PASS · 27/27"}
                        </span>
                        {"\n裁决："}
                        <span className={"g"}>
                          {"VERIFIED"}
                        </span>
                        {" — A1 标记为通过"}
                      </div>
                    </div>
                  </div>
                </div>
                <div className={"rt-node usage"} data-rtnode>
                  <div className={"rt-rail"}>
                    <span className={"rt-dot t-usage"} />
                  </div>
                  <div className={"rt-main"}>
                    <button className={"rt-row"}>
                      <span className={"rt-ico"}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <circle cx={"10"} cy={"10"} r={"6.5"} />
                          <path d={"M10 6v4l2.5 1.5"} />
                        </svg>
                      </span>
                      <span className={"rt-label"}>
                        {"用量 · 6.8K tokens（in 5.2K / out 1.6K）"}
                      </span>
                      <span className={"rt-dur"}>
                        {"—"}
                      </span>
                    </button>
                  </div>
                </div>
                <div className={"rt-node verify"} data-rtnode>
                  <div className={"rt-rail"}>
                    <span className={"rt-dot t-verify"} />
                  </div>
                  <div className={"rt-main"}>
                    <button className={"rt-row"}>
                      <span className={"rt-ico"} style={{ "color": "var(--brand)" }}>
                        <svg className={"ico"} viewBox={"0 0 20 20"}>
                          <path d={"M10 3l5 2v4.5c0 3.2-2.2 5.6-5 6.5-2.8-.9-5-3.3-5-6.5V5z"} />
                          <path d={"M10 8v3"} />
                          <circle cx={"10"} cy={"13.2"} r={".4"} fill={"currentColor"} stroke={"none"} />
                        </svg>
                      </span>
                      <span className={"rt-label"}>
                        {"正在验证 · 覆盖率 A2"}
                      </span>
                      <span className={"rt-vid"}>
                        {"VF-3A2"}
                      </span>
                      <span className={"rt-dur"}>
                        {"…"}
                      </span>
                      <svg className={"ico rt-caret"} viewBox={"0 0 20 20"}>
                        <path d={"M6 8l4 4 4-4"} />
                      </svg>
                    </button>
                    <div className={"rt-detail"}>
                      <div className={"rt-replay"}>
                        <span className={"dotmark"} />
                        {"真实记录回放"}
                      </div>
                      <div className={"rt-code"}>
                        {"验证项：useFormValidation 覆盖率 ≥ 90%\n命令：npm run coverage -- src/features/auth\n状态："}
                        <span className={"k"}>
                          {"运行中…"}
                        </span>
                      </div>
                    </div>
                  </div>
                </div>
              </div>
              <div className={"rt-foot"}>
                <span className={"rt-count"}>
                  {"共 18 个事件"}
                </span>
                <button className={"rt-more"}>
                  {"加载更多"}
                </button>
              </div>
            </div>
            <div className={"drawer-inner view-graph"} data-view={"graph"} hidden>
              <div className={"ag-head"}>
                <div className={"ag-goal"}>
                  <span className={"ql"}>
                    {"编排目标"}
                  </span>
                  {"\n              将登录表单校验逻辑抽取为独立 hook\n            "}
                </div>
                <div className={"ag-statusline"}>
                  <span className={"ag-badge running"}>
                    <span className={"pdot"} />
                    {"运行中"}
                  </span>
                  <span className={"rt-mode"}>
                    {"执行至验证节点"}
                  </span>
                </div>
                <div className={"ag-legend"}>
                  <span className={"ag-leg"}>
                    <span className={"d"} style={{ "background": "#4a4a4a" }} />
                    {"待执行"}
                  </span>
                  <span className={"ag-leg"}>
                    <span className={"d"} style={{ "background": "var(--brand)" }} />
                    {"运行中"}
                  </span>
                  <span className={"ag-leg"}>
                    <span className={"d"} style={{ "background": "var(--allow)" }} />
                    {"成功"}
                  </span>
                  <span className={"ag-leg"}>
                    <span className={"d"} style={{ "background": "var(--block)" }} />
                    {"失败"}
                  </span>
                  <span className={"ag-leg"}>
                    <span className={"d"} style={{ "background": "var(--disclose)" }} />
                    {"阻塞"}
                  </span>
                </div>
              </div>
              <div className={"ag-canvas"}>
                <svg className={"ag-edges"} viewBox={"0 0 324 472"}>
                  <defs>
                    <marker id={"agArrow"} markerWidth={"8"} markerHeight={"8"} refX={"6"} refY={"3"} orient={"auto"} markerUnits={"userSpaceOnUse"}>
                      <path d={"M0,0 L6,3 L0,6"} fill={"none"} stroke={"#3d3d3d"} strokeWidth={"1.3"} />
                    </marker>
                  </defs>
                  <path d={"M83,66 L83,104"} markerEnd={"url(#agArrow)"} />
                  <path d={"M83,158 L83,196"} markerEnd={"url(#agArrow)"} />
                  <path d={"M83,250 L83,292"} markerEnd={"url(#agArrow)"} />
                  <path d={"M83,350 C80,378 78,382 78,402"} markerEnd={"url(#agArrow)"} />
                  <path d={"M83,350 C110,374 243,376 243,402"} markerEnd={"url(#agArrow)"} />
                  <path className={"loop"} d={"M316,420 L316,223 L160,223"} markerEnd={"url(#agArrow)"} />
                </svg>
                <div className={"ag-elabel pass"} style={{ "left": "60px", "top": "378px" }}>
                  {"通过"}
                </div>
                <div className={"ag-elabel fail"} style={{ "left": "172px", "top": "374px" }}>
                  {"失败"}
                </div>
                <div className={"ag-elabel"} style={{ "left": "262px", "top": "300px" }}>
                  {"修复后重试"}
                </div>
                <div className={"ag-node t-agent s-success"} data-node={"n1"} style={{ "left": "8px", "top": "12px", "width": "150px" }}>
                  <div className={"ag-node-top"}>
                    <span className={"ag-tico"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <circle cx={"10"} cy={"7.5"} r={"3"} />
                        <path d={"M4.5 16c0-3 2.5-4.5 5.5-4.5s5.5 1.5 5.5 4.5"} />
                      </svg>
                    </span>
                    <span className={"ag-title"}>
                      {"规划任务"}
                    </span>
                    <span className={"ag-stat"} />
                  </div>
                  <div className={"ag-meta"}>
                    <span className={"ag-kind"}>
                      {"agent"}
                    </span>
                    {" · 子 agent"}
                  </div>
                </div>
                <div className={"ag-node t-tool s-success"} data-node={"n2"} style={{ "left": "8px", "top": "104px", "width": "150px" }}>
                  <div className={"ag-node-top"}>
                    <span className={"ag-tico"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <path d={"M7.5 6.5L4 10l3.5 3.5"} />
                        <path d={"M12.5 6.5L16 10l-3.5 3.5"} />
                      </svg>
                    </span>
                    <span className={"ag-title"}>
                      {"抽取校验代码"}
                    </span>
                    <span className={"ag-stat"} />
                  </div>
                  <div className={"ag-meta"}>
                    <span className={"ag-kind"}>
                      {"tool"}
                    </span>
                    {" · write"}
                  </div>
                </div>
                <div className={"ag-node t-tool s-success"} data-node={"n3"} style={{ "left": "8px", "top": "196px", "width": "150px" }}>
                  <span className={"ag-retry"}>
                    {"尝试 2/2"}
                  </span>
                  <div className={"ag-node-top"}>
                    <span className={"ag-tico"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <path d={"M5 7l3 3-3 3"} />
                        <line x1={"10"} y1={"14"} x2={"15"} y2={"14"} />
                      </svg>
                    </span>
                    <span className={"ag-title"}>
                      {"跑测试"}
                    </span>
                    <span className={"ag-stat"} />
                  </div>
                  <div className={"ag-meta"}>
                    <span className={"ag-kind"}>
                      {"tool"}
                    </span>
                    {" · shell"}
                  </div>
                </div>
                <div className={"ag-node t-verify s-running sel"} data-node={"n4"} style={{ "left": "8px", "top": "292px", "width": "150px" }}>
                  <div className={"ag-node-top"}>
                    <span className={"ag-tico"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <path d={"M10 3l5 2v4.5c0 3.2-2.2 5.6-5 6.5-2.8-.9-5-3.3-5-6.5V5z"} />
                        <path d={"M8 10l1.5 1.5L13 8"} />
                      </svg>
                    </span>
                    <span className={"ag-title"}>
                      {"验证审查"}
                    </span>
                    <span className={"ag-stat"} />
                  </div>
                  <div className={"ag-meta"}>
                    <span className={"ag-kind"}>
                      {"verifier"}
                    </span>
                    {" · 对抗审查"}
                  </div>
                </div>
                <div className={"ag-node t-agent s-pending"} data-node={"n5"} style={{ "left": "8px", "top": "404px", "width": "132px" }}>
                  <div className={"ag-node-top"}>
                    <span className={"ag-tico"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <path d={"M5 10.5l3 3 7-7"} />
                      </svg>
                    </span>
                    <span className={"ag-title"}>
                      {"完成"}
                    </span>
                    <span className={"ag-stat"} />
                  </div>
                  <div className={"ag-meta"}>
                    <span className={"ag-kind"}>
                      {"agent"}
                    </span>
                    {" · 收尾"}
                  </div>
                </div>
                <div className={"ag-node t-agent s-pending"} data-node={"n6"} style={{ "left": "170px", "top": "404px", "width": "146px" }}>
                  <div className={"ag-node-top"}>
                    <span className={"ag-tico"}>
                      <svg className={"ico"} viewBox={"0 0 20 20"}>
                        <path d={"M14 6.5a4.5 4.5 0 1 0 1 4"} />
                        <path d={"M15 4v3.2h-3.2"} />
                      </svg>
                    </span>
                    <span className={"ag-title"}>
                      {"修复"}
                    </span>
                    <span className={"ag-stat"} />
                  </div>
                  <div className={"ag-meta"}>
                    <span className={"ag-kind"}>
                      {"agent"}
                    </span>
                    {" · 子 agent"}
                  </div>
                </div>
              </div>
              <div className={"ag-inspector"} id={"agInspector"}>
                <div className={"ag-insp-head"}>
                  <span className={"ag-insp-title"} id={"agInspTitle"}>
                    {"验证审查"}
                  </span>
                  <span className={"ag-insp-kind"} id={"agInspKind"}>
                    {"verifier"}
                  </span>
                </div>
                <div className={"ag-insp-replay"}>
                  <span className={"dotmark"} />
                  <span id={"agInspStatus"}>
                    {"运行中 · 真实记录回放"}
                  </span>
                </div>
                <div className={"ag-io-label"}>
                  {"input"}
                </div>
                <div className={"ag-code"} id={"agInspInput"} />
                <div className={"ag-io-label"}>
                  {"output"}
                </div>
                <div className={"ag-code"} id={"agInspOutput"} />
              </div>
            </div>
          </div>
        </aside>
        <iframe id={"viewFrame"} className={"view-frame"} title={"page"} />
      </div>
    </div>
</>
  );
}

