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
import { useTabs, type Tab } from "./hooks/useTabs";
import { useUsage } from "./hooks/useUsage";
import { useGitStatus } from "./hooks/useGitStatus";
import { useProfiles } from "./hooks/useProfiles";
import { useAppearance } from "./hooks/useAppearance";
import { usePersistedBool, usePersistedString } from "./hooks/usePersisted";
import { LangContext, createT, type Lang } from "./i18n";
import "./App.css";

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
  const defaultShell = shells.find((s) => s.id === "powershell") ?? shells[0];

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

  const { gitStatus, refresh: refreshGit } = useGitStatus(activeCwd);
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
        shellType: s?.shell_type ?? "powershell",
      });
    },
    [openTab, defaultShell, activeCwd],
  );

  const closeTab = (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    removeTab(id);
  };

  const tabLabel = (tab: Tab) => {
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

  // 往当前标签发一条命令，按它用的 shell 选语法
  const runInActiveShell = (
    build: (q: (p: string) => string, shellType: string) => string,
  ) => {
    const st = tabs.find((x) => x.id === activeId)?.shellType ?? "powershell";
    const quote =
      st === "cmd" ? cmdQuote : st === "bash" ? bashQuote : psQuote;
    invoke("pty_write", { id: activeId, data: build(quote, st) + "\r" }).catch(
      console.error,
    );
  };

  const openDirInTerminal = (path: string) =>
    runInActiveShell((q, st) =>
      st === "cmd"
        ? `cd /d ${q(path)}`
        : st === "bash"
          ? `cd ${q(path)}`
          : `Set-Location -LiteralPath ${q(path)}`,
    );

  // 右键「在终端显示」：用当前 shell 打印文件内容
  const showInTerminal = (path: string) =>
    runInActiveShell((q, st) =>
      st === "cmd"
        ? `type ${q(path)}`
        : st === "bash"
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
  const searchInputRef = useRef<HTMLInputElement>(null);
  const runSearch = (q: string, dir: number) => {
    const a = searchAddons.current[activeId];
    if (!a || !q) return;
    if (dir >= 0) a.findNext(q);
    else a.findPrevious(q);
  };

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
      if (!e.ctrlKey) return;
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
  }, [addTab, removeTab, switchTab, activeId, setFontSize]);

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
        <header className="topbar" data-tauri-drag-region>
          <div className="tabs">
            {tabs.map((tab) => (
              <div
                key={tab.id}
                className={"tab" + (tab.id === activeId ? " active" : "")}
                onClick={() => setActiveId(tab.id)}
              >
                <span className="tab-dot" />
                <span className="tab-title" title={cwdMap[tab.id] ?? ""}>
                  {tabLabel(tab)}
                </span>
                {tabs.length > 1 && (
                  <span
                    className="tab-close"
                    title={t("tab.close")}
                    onClick={(e) => closeTab(tab.id, e)}
                  >
                    ×
                  </span>
                )}
              </div>
            ))}

            <div className="tab-add-group">
              <button className="tab-add" title={t("tab.new")} onClick={() => addTab()}>
                ＋
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
                  ▾
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
                </div>
              )}
            </div>
          </div>

          <div className="topbar-right">
            <div className="search-box">
              <span className="search-icon">⌕</span>
              <input
                ref={searchInputRef}
                value={searchQuery}
                placeholder={t("search.placeholder")}
                onChange={(e) => {
                  setSearchQuery(e.target.value);
                  runSearch(e.target.value, 1);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") runSearch(searchQuery, e.shiftKey ? -1 : 1);
                  else if (e.key === "Escape") setSearchQuery("");
                }}
              />
            </div>
            <button
              className="icon-btn"
              title={t("toolbar.settings")}
              onClick={() => setSettingsOpen(true)}
            >
              ⚙
            </button>
            <div className="win-controls">
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
            </div>
          </div>
        </header>

        <div className="body">
          <aside className="sidebar">
            <FileTree
              rootPath={activeCwd}
              onOpenDir={openDirInTerminal}
              onOpenFile={openFile}
              onShowInTerminal={showInTerminal}
              showHidden={showHidden}
              gitStatus={gitDeco ? gitStatus : null}
              gitDeco={gitDeco}
            />
          </aside>

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
                shellPath={tab.shellPath}
                shellType={tab.shellType}
                onRegisterSearch={registerSearch}
                debugInput={debugInput}
                onUnregisterSearch={unregisterSearch}
              />
            ))}
            {tabs.length === 0 && <div className="empty-hint">{t("main.empty")}</div>}
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

      <SettingsPanel
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        currentTheme={theme.id}
        onSelectTheme={setTheme}
        hasBg={!!bgImage}
        overlay={overlay}
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
