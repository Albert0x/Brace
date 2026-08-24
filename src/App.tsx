import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import type { SearchAddon } from "@xterm/addon-search";
import TerminalView from "./components/TerminalView";
import FileTree from "./components/FileTree";
import PreviewPanel from "./components/PreviewPanel";
import GitPanel from "./components/GitPanel";
import SettingsPanel from "./components/SettingsPanel";
import {
  DEFAULT_COMMIT_TYPES,
  parseCommitTypes,
} from "./components/GitPanel";
import StatusBar from "./components/StatusBar";
import { useTabs, isRemoteTab, type Tab } from "./hooks/useTabs";
import { useSshSessions, sshLabel } from "./hooks/useSshSessions";
import { useUsage } from "./hooks/useUsage";
import { useGitStatus } from "./hooks/useGitStatus";
import { useProfiles } from "./hooks/useProfiles";
import { useAppearance } from "./hooks/useAppearance";
import {
  usePersistedBool,
  usePersistedNumber,
  usePersistedString,
} from "./hooks/usePersisted";
import { LangContext, createT, type Lang } from "./i18n";
import "./App.css";

// 搜索命中的配色。xterm 这里只收 #RRGGBB（不支持 alpha），所以用压暗的底色配描边
// 来保证深浅主题下都看得见；当前命中项用亮一档的颜色和其余结果区分开。
// 没有这组配置，findNext 只会把匹配项「选中」——而此时焦点在搜索框里，
// 终端处于失焦状态，选中色淡到几乎看不出来，用户的感受就是「搜索没生效」
const SEARCH_DECORATIONS = {
  matchBackground: "#5c4a1f",
  matchBorder: "#8a7434",
  matchOverviewRuler: "#c9a227",
  activeMatchBackground: "#b58900",
  activeMatchBorder: "#ffd75f",
  activeMatchColorOverviewRuler: "#ffd75f",
};

interface ShellInfo {
  id: string;
  name: string;
  path: string;
  shell_type: string;
}

function App() {
  // 纯浏览器（README 里说的 pnpm dev frontend-only）没有 __TAURI_INTERNALS__，
  // getCurrentWindow() 会直接抛错崩掉，得先判断环境
  const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  const appWindow = isTauri ? getCurrentWindow() : null;
  // macOS 使用系统原生交通灯按钮，不渲染自绘窗口按钮。
  const isMac = navigator.platform.toUpperCase().includes("MAC");
  const shortcutMod = isMac ? "⌘" : "Ctrl+";

  // 语言（默认英文）
  const [langRaw, setLang] = usePersistedString("ht-lang", "en");
  const lang = langRaw as Lang;
  const t = useMemo(() => createT(lang), [lang]);

  // ---- 环境探测 ----
  const [shells, setShells] = useState<ShellInfo[]>([]);
  const [shellMenu, setShellMenu] = useState(false);
  const [osVersion, setOsVersion] = useState("");
  const [homeCwd, setHomeCwd] = useState("");
  useEffect(() => {
    invoke<ShellInfo[]>("detect_shells").then(setShells).catch(() => {});
    // 状态栏显示的真实系统版本（别再假装每个人都是 Win11）
    invoke<string>("os_version").then(setOsVersion).catch(() => {});
    invoke<string>("home_dir").then(setHomeCwd).catch(() => {});
  }, []);
  const defaultShell =
    shells.find((s) => s.id === "powershell") ??
    shells.find((s) => s.id === "default") ??
    shells.find((s) => s.id === "zsh") ??
    shells.find((s) => s.id === "bash") ??
    shells[0];

  // ---- 各领域状态，逐个交给专门的 hook ----
  const {
    tabs,
    activeId,
    setActiveId,
    addTab: openTab,
    removeTab,
    switchTab,
    cwdMap,
    handleCwd,
    activeCwd,
    exitedMap,
    markExited,
    clearExited,
  } = useTabs(homeCwd);

  const {
    theme,
    setTheme,
    effectiveTheme,
    appearance,
    setAppearance,
    uiZoom,
    setUiZoom,
    fontSize,
    setFontSize,
    bgImage,
    pickBg,
    bgError,
    overlay,
    setOverlay,
  } = useAppearance();

  // 提交类型列表可自定义：默认是 Conventional Commits 通用集，
  // 团队有自己一套词表的直接改这里，不用改代码
  const [commitTypesRaw, setCommitTypesRaw] = usePersistedString(
    "ht-commit-types",
    DEFAULT_COMMIT_TYPES,
  );
  const commitTypes = useMemo(
    () => parseCommitTypes(commitTypesRaw),
    [commitTypesRaw],
  );

  // 输入诊断开关。持久化是有意的：用户重启 Brace 复现问题时不该又被关掉
  const [debugInput, setDebugInput] = usePersistedBool("ht-debug-input", false);

  const [showHidden, setShowHidden] = usePersistedBool("ht-hidden", false);
  const [gitDeco, setGitDeco] = usePersistedBool("ht-gitdeco", false);
  const [webgl, setWebgl] = usePersistedBool("ht-webgl", true);
  const [cursorBlink, setCursorBlink] = usePersistedBool("ht-cursor", true);
  // 回看行数。xterm 默认只有 1000 行，跑一次 pnpm build 或 cargo build
  // 就把之前的上文全冲掉了——而那通常正是你想往回翻的东西
  const [scrollback, setScrollback] = usePersistedNumber("ht-scrollback", 5000);
  // 侧边栏宽度。以前写死 220px，而文件树是主打功能之一——
  // 稍微深一点的路径就全是省略号，只能靠 title 悬浮才知道是什么
  const [sidebarWidth, setSidebarWidth] = usePersistedNumber("ht-sidebar", 220);

  // 拖拽改宽度。松手后补发一次 resize，让 xterm 按新的可用宽度重新排版——
  // 拖侧边栏不会触发 window 的 resize 事件，不补这一下终端会一直按旧列数渲染
  const startSidebarDrag = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = sidebarWidth;
    const prevCursor = document.body.style.cursor;
    const prevSelect = document.body.style.userSelect;
    document.body.style.cursor = "col-resize";
    // 拖动中别把界面文字一路选蓝
    document.body.style.userSelect = "none";

    const onMove = (ev: MouseEvent) => {
      // 夹在合理区间：太窄连图标都放不下，太宽终端就没地方了
      const next = Math.max(140, Math.min(520, startWidth + ev.clientX - startX));
      setSidebarWidth(next);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.style.cursor = prevCursor;
      document.body.style.userSelect = prevSelect;
      window.dispatchEvent(new Event("resize"));
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  const ssh = useSshSessions();
  // 起不来的时候必须说话。点了菜单没反应，用户只会以为界面坏了
  const [notice, setNotice] = useState("");

  // 开一个 SSH 标签。命令行由后端拼（那边有测试盯着「留空就不传」的约定），
  // 这里只负责把结果转交给 pty_create
  const addSshTab = useCallback(
    (sessionId: string) => {
      invoke<{ path: string; args: string[] }>("ssh_launch", { sessionId })
        .then((launch) =>
          openTab({
            // 远程会话的 cwd 由远端决定，本地传什么都没意义
            cwd: "",
            shellPath: launch.path,
            shellType: "ssh",
            args: launch.args,
          }),
        )
        .catch((e) => setNotice(String(e)));
    },
    [openTab],
  );

  const activeTab = tabs.find((x) => x.id === activeId);
  // 远程标签里，本地的文件树 / git 状态 / 配置组全都对不上号——
  // OSC 9;9 在远端 shell 里不生效，cwd 会停在连接前的本地目录。
  // 照着渲染不是「信息少一点」，是显示错的东西
  const activeIsRemote = isRemoteTab(activeTab);

  const { gitStatus, refresh: refreshGit } = useGitStatus(
    activeIsRemote ? "" : activeCwd,
    gitDeco,
  );
  // 这两个整块传给 StatusBar，不在这里解构——它们本来就是各自内聚的一块状态
  const usageState = useUsage(activeId);
  const profiles = useProfiles();

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [gitPanelOpen, setGitPanelOpen] = useState(false);
  // 文件预览：单击文件在侧边打开
  const [preview, setPreview] = useState<{ path: string; name: string } | null>(
    null,
  );
  const openFile = (path: string, name: string) => setPreview({ path, name });

  // 新标签继承当前目录，没指定 shell 就用默认那个
  const addTab = useCallback(
    (shell?: ShellInfo) => {
      const s = shell ?? defaultShell;
      openTab({
        cwd: activeCwd,
        shellPath: s?.path ?? "",
        shellType: s?.shell_type ?? "default",
      });
    },
    [openTab, defaultShell, activeCwd],
  );

  const closeTab = (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    removeTab(id);
  };

  const tabLabel = (tab: Tab) => {
    // 远程标签的 cwd 永远拿不到（OSC 9;9 不在远端生效），所以用连接目标当标题。
    // args 的最后一项就是 [user@]host
    if (isRemoteTab(tab)) return tab.args[tab.args.length - 1] || "SSH";
    const cwd = cwdMap[tab.id];
    const b = cwd ? cwd.split(/[\\/]/).filter(Boolean).pop() : "";
    if (b) return b;
    const s = shells.find((x) => x.shell_type === tab.shellType);
    return s?.name ?? "Terminal";
  };

  // 把路径安全地嵌进对应 shell 的命令字符串：PS/bash 用字面量单引号转义，
  // cmd 用双引号包裹（Windows 文件名本就不能含引号/尖括号/管道符，双引号足以挡住 & | < > 等元字符；
  // 唯一挡不住的是 cmd 对 %VAR% 的展开——这是 cmd.exe 自身的固有限制，无法在字符串层面完全消除）
  const psQuote = (path: string) => `'${path.replace(/'/g, "''")}'`;
  const bashQuote = (path: string) => `'${path.replace(/'/g, "'\\''")}'`;
  const cmdQuote = (path: string) => `"${path}"`;

  // 这个标签的 shell 是不是 POSIX 系。
  //
  // "default" 的含义是「交给后端按系统默认 shell 推断」，前端拿不到推断结果，
  // 只能按平台猜——而 Windows 上默认是 PowerShell，绝不能归进 POSIX：
  // bashQuote 的单引号转义是 '\''，PowerShell 要的是 ''，含单引号的路径会直接出错。
  // cd/cat 在 PowerShell 里恰好是 Set-Location/Get-Content 的别名，所以命令本身
  // 侥幸能跑，只有引号会露馅——正因为这样才更容易漏掉
  const isPosixShell = (st: string) =>
    st === "bash" || st === "zsh" || st === "sh" || (st === "default" && isMac);

  // 往当前标签发一条命令，按它用的 shell 选语法
  const runInActiveShell = (
    build: (q: (p: string) => string, shellType: string) => string,
  ) => {
    const st = tabs.find((x) => x.id === activeId)?.shellType ?? "default";
    const quote = st === "cmd" ? cmdQuote : isPosixShell(st) ? bashQuote : psQuote;
    invoke("pty_write", { id: activeId, data: build(quote, st) + "\r" }).catch(
      console.error,
    );
  };

  const openDirInTerminal = (path: string) =>
    runInActiveShell((q, st) =>
      st === "cmd"
        ? `cd /d ${q(path)}`
        : isPosixShell(st)
          ? `cd ${q(path)}`
          : `Set-Location -LiteralPath ${q(path)}`,
    );

  // 右键「在终端显示」：用当前 shell 打印文件内容
  const showInTerminal = (path: string) =>
    runInActiveShell((q, st) =>
      st === "cmd"
        ? `type ${q(path)}`
        : isPosixShell(st)
          ? `cat ${q(path)}`
          : `Get-Content -LiteralPath ${q(path)}`,
    );

  // ---- 搜索 ----
  const searchAddons = useRef<Record<string, SearchAddon>>({});
  const registerSearch = useCallback((id: string, a: SearchAddon) => {
    searchAddons.current[id] = a;
  }, []);
  const unregisterSearch = useCallback((id: string) => {
    delete searchAddons.current[id];
  }, []);
  const [searchQuery, setSearchQuery] = useState("");
  // 计数带上它属于哪个标签。这样换标签时不用在 effect 里把它清空，
  // 渲染时比对一下 id 就行——上一个终端的搜索结果自然就不会显示了
  const [searchResult, setSearchResult] = useState<{
    id: string;
    resultIndex: number;
    resultCount: number;
  } | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);

  // 命中数只有在开了 decorations 时才会上报，两件事是绑在一起的
  useEffect(() => {
    const a = searchAddons.current[activeId];
    if (!a) return;
    const sub = a.onDidChangeResults((r) =>
      setSearchResult({ id: activeId, ...r }),
    );
    return () => sub.dispose();
  }, [activeId]);

  const activeResult =
    searchResult?.id === activeId ? searchResult : null;

  const runSearch = (q: string, dir: number, incremental = false) => {
    const a = searchAddons.current[activeId];
    if (!a) return;
    if (!q) {
      // 清空搜索框就该把高亮一起收掉，否则满屏黄块留在那儿
      a.clearDecorations();
      setSearchResult(null);
      return;
    }

    // incremental：打字过程中不往下一个结果跳，只在当前位置往后找。
    // 之前每敲一个字符都 findNext，视口会跟着来回蹦
    const opts = { decorations: SEARCH_DECORATIONS, incremental };
    if (dir >= 0) a.findNext(q, opts);
    else a.findPrevious(q, opts);
  };

  const noMatch = !!searchQuery && activeResult?.resultCount === 0;

  useEffect(() => {
    if (!shellMenu) return;
    const close = () => setShellMenu(false);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [shellMenu]);

  // 全局快捷键。removeTab / switchTab 由 useTabs 保证引用稳定且内部读的是最新标签列表，
  // 所以这里不用再把 tabs 塞进依赖数组——那正是之前"关掉的标签被 Ctrl+W 复活"的成因
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const hasPrimaryModifier = isMac ? e.metaKey : e.ctrlKey;
      if (!hasPrimaryModifier) return;
      const stop = () => {
        e.preventDefault();
        e.stopPropagation();
      };
      if (!e.shiftKey && e.code === "KeyT") {
        stop();
        addTab();
      } else if (!e.shiftKey && e.code === "KeyW") {
        stop();
        removeTab(activeId);
      } else if (e.code === "Tab") {
        stop();
        switchTab(e.shiftKey ? -1 : 1);
      } else if (!e.shiftKey && e.code === "KeyF") {
        stop();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      } else if (e.code === "Equal") {
        stop();
        setFontSize((f) => Math.min(28, f + 1));
      } else if (e.code === "Minus") {
        stop();
        setFontSize((f) => Math.max(8, f - 1));
      } else if (e.code === "Digit0") {
        stop();
        setFontSize(14);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [addTab, removeTab, switchTab, activeId, setFontSize, isMac]);

  return (
    <LangContext.Provider value={{ lang, setLang, t }}>
      <div
        className="bg-layer"
        style={{ backgroundImage: bgImage ? `url(${bgImage})` : "none" }}
      />
      <div
        className="bg-overlay"
        style={{ background: effectiveTheme.ui.base, opacity: bgImage ? overlay : 0.9 }}
      />

      <div className="app">
        <header className={"topbar" + (isMac ? " mac" : "")} data-tauri-drag-region>
          <div className="tabs">
            {tabs.map((tab) => (
              <div
                key={tab.id}
                className={
                  "tab" +
                  (tab.id === activeId ? " active" : "") +
                  (tab.id in exitedMap ? " exited" : "")
                }
                onClick={() => setActiveId(tab.id)}
              >
                <span className="tab-dot" />
                <span
                  className="tab-title"
                  title={
                    tab.id in exitedMap
                      ? t("tab.exitedTitle")
                      : (cwdMap[tab.id] ?? "")
                  }
                >
                  {tabLabel(tab)}
                </span>
                {tabs.length > 1 && (
                  <span
                    className="tab-close"
                    title={t("tab.close", { mod: shortcutMod })}
                    onClick={(e) => closeTab(tab.id, e)}
                  >
                    ×
                  </span>
                )}
              </div>
            ))}

            <div className="tab-add-group">
              <button className="tab-add" title={t("tab.new", { mod: shortcutMod })} onClick={() => addTab()}>
                <span className="tab-add-icon" aria-hidden="true">+</span>
              </button>
              {shells.length > 1 && (
                <button
                  className="tab-add-caret"
                  title={t("tab.selectShell")}
                  onClick={(e) => {
                    e.stopPropagation();
                    setShellMenu((v) => !v);
                  }}
                >
                  <span className="tab-caret-icon" aria-hidden="true">⌄</span>
                </button>
              )}
              {shellMenu && (
                <div className="shell-menu" onClick={(e) => e.stopPropagation()}>
                  {shells.map((s) => (
                    <div
                      className="shell-item"
                      key={s.id}
                      onClick={() => {
                        addTab(s);
                        setShellMenu(false);
                      }}
                    >
                      <span>{s.name}</span>
                      <span className="shell-item-type">{s.shell_type}</span>
                    </div>
                  ))}
                  {/* 系统没有 ssh 客户端时整组都不出现——列一堆点了会报错的项没有意义 */}
                  {ssh.clientPath && ssh.sessions.length > 0 && (
                    <>
                      <div className="shell-sep" />
                      {ssh.sessions.map((sess) => (
                        <div
                          className="shell-item"
                          key={sess.id}
                          title={sess.note || undefined}
                          onClick={() => {
                            addSshTab(sess.id);
                            setShellMenu(false);
                          }}
                        >
                          <span>{sshLabel(sess)}</span>
                          <span className="shell-item-type">ssh</span>
                        </div>
                      ))}
                    </>
                  )}
                </div>
              )}
            </div>
          </div>

          <div className="topbar-right">
            <div className={"search-box" + (noMatch ? " no-match" : "")}>
              <span className="search-icon">⌕</span>
              <input
                ref={searchInputRef}
                value={searchQuery}
                placeholder={t("search.placeholder", { mod: shortcutMod })}
                onChange={(e) => {
                  setSearchQuery(e.target.value);
                  runSearch(e.target.value, 1, true);
                }}
                onKeyDown={(e) => {
                  // 打字只高亮，回车才跳到下一个（Shift+回车往回跳）
                  if (e.key === "Enter") runSearch(searchQuery, e.shiftKey ? -1 : 1);
                  else if (e.key === "Escape") {
                    setSearchQuery("");
                    runSearch("", 1);
                  }
                }}
              />
              {searchQuery && activeResult && (
                <span className="search-count">
                  {activeResult.resultCount === 0
                    ? t("search.none")
                    : `${activeResult.resultIndex + 1}/${activeResult.resultCount}`}
                </span>
              )}
            </div>
            <button
              className="icon-btn"
              title={t("toolbar.settings")}
              onClick={() => setSettingsOpen(true)}
            >
              <span className="settings-icon" aria-hidden="true">⚙︎</span>
            </button>
            {!isMac && <div className="win-controls">
              <button className="win-btn" title={t("win.minimize")} onClick={() => appWindow?.minimize()}>
                <svg width="10" height="10" viewBox="0 0 10 10">
                  <rect y="4.5" width="10" height="1" fill="currentColor" />
                </svg>
              </button>
              <button className="win-btn" title={t("win.maximize")} onClick={() => appWindow?.toggleMaximize()}>
                <svg width="10" height="10" viewBox="0 0 10 10">
                  <rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" />
                </svg>
              </button>
              <button className="win-btn win-close" title={t("win.close")} onClick={() => appWindow?.close()}>
                <svg width="10" height="10" viewBox="0 0 10 10">
                  <path d="M1 1 L9 9 M9 1 L1 9" stroke="currentColor" strokeWidth="1.2" />
                </svg>
              </button>
            </div>}
          </div>
        </header>

        <div className="body">
          <aside className="sidebar" style={{ flexBasis: sidebarWidth }}>
            {activeIsRemote ? (
              <div className="sidebar-remote">{t("sidebar.remote")}</div>
            ) : (
            <FileTree
              rootPath={activeCwd}
              onOpenDir={openDirInTerminal}
              onOpenFile={openFile}
              onShowInTerminal={showInTerminal}
              showHidden={showHidden}
              gitStatus={gitDeco ? gitStatus : null}
              gitDeco={gitDeco}
            />
            )}
          </aside>

          <div
            className="sidebar-resizer"
            onMouseDown={startSidebarDrag}
            title={t("sidebar.resize")}
          />

          <main className="main">
            {tabs.map((tab) => (
              <TerminalView
                key={tab.id}
                sessionId={tab.id}
                active={tab.id === activeId}
                onCwd={handleCwd}
                termTheme={effectiveTheme.terminal}
                initialCwd={tab.initialCwd}
                fontSize={fontSize}
                cursorBlink={cursorBlink}
                webgl={webgl}
                scrollback={scrollback}
                shellPath={tab.shellPath}
                shellType={tab.shellType}
                args={tab.args}
                onRegisterSearch={registerSearch}
                debugInput={debugInput}
                onUnregisterSearch={unregisterSearch}
                onExit={markExited}
                onRestarted={clearExited}
              />
            ))}
            {tabs.length === 0 && (
              <div className="empty-hint">{t("main.empty", { mod: shortcutMod })}</div>
            )}
            {preview && (
              <PreviewPanel
                path={preview.path}
                name={preview.name}
                onClose={() => setPreview(null)}
              />
            )}
          </main>
        </div>

        <StatusBar
          remote={activeIsRemote}
          gitStatus={gitStatus}
          onOpenGit={() => setGitPanelOpen(true)}
          profiles={profiles}
          usage={usageState}
          fontSize={fontSize}
          tabCount={tabs.length}
          themeName={theme.name}
          osVersion={osVersion}
        />
      </div>

      {/* 起不来的原因得说出来。SSH 那条路上最常见的是「系统里没有 ssh 客户端」，
          静默失败会让用户以为是菜单坏了 */}
      {notice && (
        <div className="app-notice" onClick={() => setNotice("")}>
          <span>{notice}</span>
          <span className="app-notice-close">×</span>
        </div>
      )}

      <SettingsPanel
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        currentTheme={theme.id}
        onSelectTheme={setTheme}
        hasBg={!!bgImage}
        overlay={overlay}
        bgError={bgError}
        onPickBg={pickBg}
        onClearBg={() => pickBg("")}
        onOverlay={setOverlay}
        appearance={appearance}
        onAppearance={setAppearance}
        uiZoom={uiZoom}
        onUiZoom={setUiZoom}
        showHidden={showHidden}
        onShowHidden={setShowHidden}
        gitDeco={gitDeco}
        onGitDeco={setGitDeco}
        webgl={webgl}
        onWebgl={setWebgl}
        cursorBlink={cursorBlink}
        onCursorBlink={setCursorBlink}
        scrollback={scrollback}
        onScrollback={setScrollback}
        sshSessions={ssh.sessions}
        sshClientPath={ssh.clientPath}
        onSshChanged={ssh.refresh}
        commitTypes={commitTypesRaw}
        debugInput={debugInput}
        onDebugInput={setDebugInput}
        onCommitTypes={setCommitTypesRaw}
        onProfilesChanged={profiles.refresh}
      />

      {gitPanelOpen && (
        <GitPanel
          cwd={activeCwd}
          gitStatus={gitStatus}
          commitTypes={commitTypes}
          onClose={() => setGitPanelOpen(false)}
          onDone={refreshGit}
        />
      )}
    </LangContext.Provider>
  );
}

export default App;
