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

function SettingsWindow() {
  const [langRaw, setLang] = usePersistedString("ht-lang", "en");
  const lang = langRaw as Lang;
  const t = useMemo(() => createT(lang), [lang]);
  const appearanceState = useAppearance();
  const [showHidden, setShowHidden] = usePersistedBool("ht-hidden", false);
  const [gitDeco, setGitDeco] = usePersistedBool("ht-gitdeco", false);
  const [webgl, setWebgl] = usePersistedBool("ht-webgl", true);
  const [cursorBlink, setCursorBlink] = usePersistedBool("ht-cursor", true);
  const [debugInput, setDebugInput] = usePersistedBool("ht-debug-input", false);
  const [commitTypesRaw, setCommitTypesRaw] = usePersistedString(
    "ht-commit-types",
    DEFAULT_COMMIT_TYPES,
  );
  const profiles = useProfiles();
  const close = () => getCurrentWindow().close();

  return (
    <LangContext.Provider value={{ lang, setLang, t }}>
      <div
        className="bg-layer"
        style={{ backgroundImage: appearanceState.bgImage ? `url(${appearanceState.bgImage})` : "none" }}
      />
      <div
        className="bg-overlay"
        style={{
          background: appearanceState.effectiveTheme.ui.base,
          opacity: appearanceState.bgImage ? appearanceState.overlay : 0.78,
        }}
      />
      <SettingsPanel
        open
        onClose={close}
        currentTheme={appearanceState.theme.id}
        onSelectTheme={appearanceState.setTheme}
        hasBg={!!appearanceState.bgImage}
        overlay={appearanceState.overlay}
        onPickBg={appearanceState.pickBg}
        onClearBg={() => appearanceState.pickBg("")}
        onOverlay={appearanceState.setOverlay}
        appearance={appearanceState.appearance}
        onAppearance={appearanceState.setAppearance}
        uiZoom={appearanceState.uiZoom}
        onUiZoom={appearanceState.setUiZoom}
        showHidden={showHidden}
        onShowHidden={setShowHidden}
        gitDeco={gitDeco}
        onGitDeco={setGitDeco}
        webgl={webgl}
        onWebgl={setWebgl}
        cursorBlink={cursorBlink}
        onCursorBlink={setCursorBlink}
        commitTypes={commitTypesRaw}
        onCommitTypes={setCommitTypesRaw}
        debugInput={debugInput}
        onDebugInput={setDebugInput}
        onProfilesChanged={profiles.refresh}
      />
    </LangContext.Provider>
  );
}

function MainApp() {
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
  const [homeCwd, setHomeCwd] = useState("");
  useEffect(() => {
    invoke<ShellInfo[]>("detect_shells").then(setShells).catch(() => {});
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
  } = useTabs(homeCwd);

  const {
    effectiveTheme,
    fontSize,
    setFontSize,
    bgImage,
    overlay,
  } = useAppearance();

  // 提交类型列表可自定义：默认是 Conventional Commits 通用集，
  // 团队有自己一套词表的直接改这里，不用改代码
  const [commitTypesRaw] = usePersistedString(
    "ht-commit-types",
    DEFAULT_COMMIT_TYPES,
  );
  const commitTypes = useMemo(
    () => parseCommitTypes(commitTypesRaw),
    [commitTypesRaw],
  );

  // 输入诊断开关。持久化是有意的：用户重启 Brace 复现问题时不该又被关掉
  const [debugInput] = usePersistedBool("ht-debug-input", false);

  const [showHidden] = usePersistedBool("ht-hidden", false);
  const [gitDeco] = usePersistedBool("ht-gitdeco", false);
  const [webgl] = usePersistedBool("ht-webgl", true);
  const [cursorBlink] = usePersistedBool("ht-cursor", true);

  const { gitStatus, refresh: refreshGit } = useGitStatus(activeCwd);
  // 这两个整块传给 StatusBar，不在这里解构——它们本来就是各自内聚的一块状态
  const usageState = useUsage(activeId);
  const profiles = useProfiles();

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

  const addConversation = useCallback(async () => {
    let agent = usageState.usage?.agent;
    try {
      const current = await invoke<{ agent: string }>("usage_stats", { sessionId: activeId });
      agent = current.agent;
    } catch {
      // 浏览器预览或检测失败时，仍然创建一个普通终端，不让菜单失效。
    }
    const currentTab = tabs.find((tab) => tab.id === activeId);
    const startupCommand = agent === "claude" || agent === "codex" ? agent : undefined;
    openTab({
      cwd: activeCwd,
      shellPath: currentTab?.shellPath ?? defaultShell?.path ?? "",
      shellType: currentTab?.shellType ?? defaultShell?.shell_type ?? "default",
      startupCommand,
    });
  }, [activeCwd, activeId, defaultShell, openTab, tabs, usageState.usage?.agent]);

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
    const st = tabs.find((x) => x.id === activeId)?.shellType ?? "default";
    const isPosix = st === "bash" || st === "zsh" || st === "sh" || st === "default";
    const quote =
      st === "cmd" ? cmdQuote : isPosix ? bashQuote : psQuote;
    invoke("pty_write", { id: activeId, data: build(quote, st) + "\r" }).catch(
      console.error,
    );
  };

  const openDirInTerminal = (path: string) =>
    runInActiveShell((q, st) =>
      st === "cmd"
        ? `cd /d ${q(path)}`
        : st === "bash" || st === "zsh" || st === "sh" || st === "default"
          ? `cd ${q(path)}`
          : `Set-Location -LiteralPath ${q(path)}`,
    );

  // 右键「在终端显示」：用当前 shell 打印文件内容
  const showInTerminal = (path: string) =>
    runInActiveShell((q, st) =>
      st === "cmd"
        ? `type ${q(path)}`
        : st === "bash" || st === "zsh" || st === "sh" || st === "default"
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
                className={"tab" + (tab.id === activeId ? " active" : "")}
                onClick={() => setActiveId(tab.id)}
              >
                <span className="tab-shell-icon" aria-hidden="true">›_</span>
                <span className="tab-title" title={cwdMap[tab.id] ?? ""}>
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
                <span className="tab-add-icon" aria-hidden="true">＋</span>
              </button>
            </div>
          </div>

          <div className="topbar-right">
            <div className="search-box">
              <span className="search-icon">⌕</span>
              <input
                ref={searchInputRef}
                value={searchQuery}
                placeholder={t("search.placeholder", { mod: shortcutMod })}
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
              className="icon-btn toolbar-settings-button"
              title={t("toolbar.settings")}
              aria-label={t("toolbar.settings")}
              onClick={() => invoke("open_settings_window").catch(console.error)}
            >
              <svg className="settings-icon" aria-hidden="true" viewBox="0 0 20 20">
                <path d="M10 7.25a2.75 2.75 0 1 0 0 5.5 2.75 2.75 0 0 0 0-5.5Z" />
                <path d="M16.42 11.35a6.75 6.75 0 0 0 0-2.7l1.3-1.02-1.5-2.6-1.54.62a6.8 6.8 0 0 0-2.34-1.35L12.1 2.65h-3L8.85 4.3A6.8 6.8 0 0 0 6.5 5.65l-1.53-.62-1.5 2.6 1.3 1.02a6.75 6.75 0 0 0 0 2.7l-1.3 1.02 1.5 2.6 1.53-.62a6.8 6.8 0 0 0 2.35 1.35l.25 1.65h3l.24-1.65a6.8 6.8 0 0 0 2.34-1.35l1.54.62 1.5-2.6-1.3-1.02Z" />
              </svg>
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
                startupCommand={tab.startupCommand}
                activeAgent={tab.id === activeId ? usageState.usage?.agent ?? "" : ""}
                onNewTerminal={() => addTab()}
                onNewConversation={addConversation}
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
          cwd={activeCwd}
          gitStatus={gitStatus}
          onOpenGit={() => setGitPanelOpen(true)}
          profiles={profiles}
          usage={usageState}
        />
      </div>

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

export default function App() {
  return new URLSearchParams(window.location.search).get("window") === "settings"
    ? <SettingsWindow />
    : <MainApp />;
}
