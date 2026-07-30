import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import type { SearchAddon } from "@xterm/addon-search";
import TerminalView from "./components/TerminalView";
import FileTree from "./components/FileTree";
import SettingsPanel from "./components/SettingsPanel";
import { THEMES, LIGHT_THEME, applyTheme, type Theme } from "./themes";
import { LangContext, createT, type Lang } from "./i18n";
import "./App.css";

interface ShellInfo {
  id: string;
  name: string;
  path: string;
  shell_type: string;
}

interface Tab {
  id: string;
  initialCwd: string;
  shellPath: string;
  shellType: string;
}

interface UsageStats {
  agent: string; // "claude" / "codex" / ""
  model: string;
  contextPct: number;
  fiveHourPct: number;
  fiveHourResetMs: number;
  sevenDayPct: number;
  sevenDayResetMs: number;
  codexTotalTokens: number;
  cacheAgeSec: number;
  hasRateLimits: boolean;
  hasData: boolean;
}

// 重置倒计时：4h 7m / 35m / 2d 3h / ✓
function fmtCountdown(resetMs: number): string {
  const diff = resetMs - Date.now();
  if (diff <= 0) return "✓";
  const h = Math.floor(diff / 3600000);
  const m = Math.floor((diff % 3600000) / 60000);
  if (h >= 24) return `${Math.floor(h / 24)}d ${h % 24}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

// 1234567 → "1.2M"；45678 → "46K"
function fmtTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return Math.round(n / 1_000) + "K";
  return String(n);
}

// 单条用量计：标签 + 进度条 + % + 可选重置倒计时
function UsageMeter({
  label,
  pct,
  reset,
}: {
  label: string;
  pct: number;
  reset?: number;
}) {
  const p = Math.max(0, Math.min(100, Math.round(pct)));
  const level = p >= 90 ? " hi" : p >= 70 ? " mid" : "";
  return (
    <span className="usage-meter">
      <span className="usage-label">{label}</span>
      <span className="usage-bar">
        <i className={"usage-fill" + level} style={{ width: `${p}%` }} />
      </span>
      <span className="usage-pct">{p}%</span>
      {reset != null && reset > 0 && (
        <span className="usage-reset">{fmtCountdown(reset)}</span>
      )}
    </span>
  );
}

function App() {
  const appWindow = getCurrentWindow();

  // 语言（默认英文）
  const [lang, setLang] = useState<Lang>(
    () => (localStorage.getItem("ht-lang") as Lang) || "en",
  );
  useEffect(() => {
    localStorage.setItem("ht-lang", lang);
  }, [lang]);
  const t = useMemo(() => createT(lang), [lang]);

  const [tabs, setTabs] = useState<Tab[]>(() => [
    { id: crypto.randomUUID(), initialCwd: "", shellPath: "", shellType: "powershell" },
  ]);
  const [activeId, setActiveId] = useState<string>(() => tabs[0].id);

  const [shells, setShells] = useState<ShellInfo[]>([]);
  const [shellMenu, setShellMenu] = useState(false);
  useEffect(() => {
    invoke<ShellInfo[]>("detect_shells").then(setShells).catch(() => {});
  }, []);
  const defaultShell = shells.find((s) => s.id === "powershell") ?? shells[0];

  // 主题
  const [theme, setTheme] = useState<Theme>(
    () => THEMES.find((x) => x.id === localStorage.getItem("ht-theme")) ?? THEMES[0],
  );
  const [settingsOpen, setSettingsOpen] = useState(false);
  useEffect(() => {
    localStorage.setItem("ht-theme", theme.id);
  }, [theme]);

  // 背景图 + 遮罩
  const [bgImage, setBgImage] = useState<string>(
    () => localStorage.getItem("ht-bg") ?? "",
  );
  const [overlay, setOverlay] = useState<number>(
    () => Number(localStorage.getItem("ht-overlay")) || 0.5,
  );
  useEffect(() => {
    try {
      if (bgImage) localStorage.setItem("ht-bg", bgImage);
      else localStorage.removeItem("ht-bg");
    } catch {
      /* 图太大超配额时仅本会话生效 */
    }
  }, [bgImage]);
  useEffect(() => {
    localStorage.setItem("ht-overlay", String(overlay));
  }, [overlay]);

  // General 设置
  const [appearance, setAppearance] = useState(
    () => localStorage.getItem("ht-appearance") ?? "dark",
  );
  const [uiZoom, setUiZoom] = useState(
    () => Number(localStorage.getItem("ht-zoom")) || 1,
  );
  const [showHidden, setShowHidden] = useState(
    () => localStorage.getItem("ht-hidden") === "1",
  );
  const [gitDeco, setGitDeco] = useState(
    () => localStorage.getItem("ht-gitdeco") === "1",
  );
  const [webgl, setWebgl] = useState(
    () => localStorage.getItem("ht-webgl") !== "0",
  );
  const [cursorBlink, setCursorBlink] = useState(
    () => localStorage.getItem("ht-cursor") !== "0",
  );
  useEffect(() => {
    localStorage.setItem("ht-appearance", appearance);
  }, [appearance]);
  useEffect(() => {
    localStorage.setItem("ht-zoom", String(uiZoom));
    (document.documentElement.style as any).zoom = String(uiZoom);
    requestAnimationFrame(() => window.dispatchEvent(new Event("resize")));
  }, [uiZoom]);
  useEffect(() => {
    localStorage.setItem("ht-hidden", showHidden ? "1" : "0");
  }, [showHidden]);
  useEffect(() => {
    localStorage.setItem("ht-gitdeco", gitDeco ? "1" : "0");
  }, [gitDeco]);
  useEffect(() => {
    localStorage.setItem("ht-webgl", webgl ? "1" : "0");
  }, [webgl]);
  useEffect(() => {
    localStorage.setItem("ht-cursor", cursorBlink ? "1" : "0");
  }, [cursorBlink]);

  // 跟随系统明暗
  const [systemDark, setSystemDark] = useState(
    () => window.matchMedia("(prefers-color-scheme: dark)").matches,
  );
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const h = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener("change", h);
    return () => mq.removeEventListener("change", h);
  }, []);
  const effectiveTheme =
    appearance === "light"
      ? LIGHT_THEME
      : appearance === "system"
        ? systemDark
          ? theme
          : LIGHT_THEME
        : theme;
  useEffect(() => {
    applyTheme(effectiveTheme);
  }, [effectiveTheme]);

  // 字体大小
  const [fontSize, setFontSize] = useState<number>(
    () => Number(localStorage.getItem("ht-fontsize")) || 14,
  );
  useEffect(() => {
    localStorage.setItem("ht-fontsize", String(fontSize));
  }, [fontSize]);

  // 文件树根目录
  const [homeCwd, setHomeCwd] = useState("");
  const [cwdMap, setCwdMap] = useState<Record<string, string>>({});
  useEffect(() => {
    invoke<string>("home_dir").then(setHomeCwd).catch(() => {});
  }, []);
  const handleCwd = useCallback((sid: string, path: string) => {
    setCwdMap((m) => (m[sid] === path ? m : { ...m, [sid]: path }));
  }, []);
  const activeCwd = cwdMap[activeId] ?? homeCwd;

  // Claude 用量：按当前标签轮询后端（含进程检测 + 读官方缓存），每 10s 一次
  const [usage, setUsage] = useState<UsageStats | null>(null);
  useEffect(() => {
    let alive = true;
    const poll = () =>
      invoke<UsageStats>("usage_stats", { sessionId: activeId })
        .then((u) => alive && setUsage(u))
        .catch(() => {});
    poll();
    const id = setInterval(poll, 10000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [activeId]);

  // 引导式默认开：检测到跑 claude 但用量没开时，状态栏提示一次；忽略后记住不再提示
  const [usagePromptOff, setUsagePromptOff] = useState(
    () => localStorage.getItem("brace-usage-prompt") === "off",
  );
  const enableUsage = () => {
    invoke("configure_statusline", { enable: true, force: false })
      .then(() =>
        invoke<UsageStats>("usage_stats", { sessionId: activeId }).then(setUsage),
      )
      .catch(() => {});
  };
  const dismissUsagePrompt = () => {
    setUsagePromptOff(true);
    localStorage.setItem("brace-usage-prompt", "off");
  };

  const tabLabel = (tab: Tab) => {
    const cwd = cwdMap[tab.id];
    const b = cwd ? cwd.split(/[\\/]/).filter(Boolean).pop() : "";
    if (b) return b;
    const s = shells.find((x) => x.shell_type === tab.shellType);
    return s?.name ?? "Terminal";
  };

  const openDirInTerminal = (path: string) => {
    invoke("pty_write", { id: activeId, data: `cd '${path}'\r` }).catch(console.error);
  };

  // 搜索
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

  const addTab = (shell?: ShellInfo) => {
    const s = shell ?? defaultShell;
    const id = crypto.randomUUID();
    setTabs((tb) => [
      ...tb,
      {
        id,
        initialCwd: activeCwd,
        shellPath: s?.path ?? "",
        shellType: s?.shell_type ?? "powershell",
      },
    ]);
    setActiveId(id);
  };

  const removeTab = (id: string) => {
    setTabs((prev) => {
      const idx = prev.findIndex((x) => x.id === id);
      const next = prev.filter((x) => x.id !== id);
      if (id === activeId && next.length) {
        setActiveId(next[Math.max(0, idx - 1)].id);
      }
      return next;
    });
  };

  const closeTab = (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    removeTab(id);
  };

  const switchTab = (dir: number) => {
    setTabs((prev) => {
      if (prev.length < 2) return prev;
      const idx = prev.findIndex((x) => x.id === activeId);
      setActiveId(prev[(idx + dir + prev.length) % prev.length].id);
      return prev;
    });
  };

  useEffect(() => {
    if (!shellMenu) return;
    const close = () => setShellMenu(false);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [shellMenu]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey) return;
      if (!e.shiftKey && e.code === "KeyT") {
        e.preventDefault();
        e.stopPropagation();
        addTab();
      } else if (!e.shiftKey && e.code === "KeyW") {
        e.preventDefault();
        e.stopPropagation();
        removeTab(activeId);
      } else if (e.code === "Tab") {
        e.preventDefault();
        e.stopPropagation();
        switchTab(e.shiftKey ? -1 : 1);
      } else if (!e.shiftKey && e.code === "KeyF") {
        e.preventDefault();
        e.stopPropagation();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      } else if (e.code === "Equal") {
        e.preventDefault();
        e.stopPropagation();
        setFontSize((f) => Math.min(28, f + 1));
      } else if (e.code === "Minus") {
        e.preventDefault();
        e.stopPropagation();
        setFontSize((f) => Math.max(8, f - 1));
      } else if (e.code === "Digit0") {
        e.preventDefault();
        e.stopPropagation();
        setFontSize(14);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [activeId, cwdMap, homeCwd, shells]);

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
              <button className="win-btn" title={t("win.minimize")} onClick={() => appWindow.minimize()}>
                <svg width="10" height="10" viewBox="0 0 10 10">
                  <rect y="4.5" width="10" height="1" fill="currentColor" />
                </svg>
              </button>
              <button className="win-btn" title={t("win.maximize")} onClick={() => appWindow.toggleMaximize()}>
                <svg width="10" height="10" viewBox="0 0 10 10">
                  <rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" />
                </svg>
              </button>
              <button className="win-btn win-close" title={t("win.close")} onClick={() => appWindow.close()}>
                <svg width="10" height="10" viewBox="0 0 10 10">
                  <path d="M1 1 L9 9 M9 1 L1 9" stroke="currentColor" strokeWidth="1.2" />
                </svg>
              </button>
            </div>
          </div>
        </header>

        <div className="body">
          <aside className="sidebar">
            <FileTree rootPath={activeCwd} onOpenDir={openDirInTerminal} showHidden={showHidden} />
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
                onUnregisterSearch={unregisterSearch}
              />
            ))}
            {tabs.length === 0 && <div className="empty-hint">{t("main.empty")}</div>}
          </main>
        </div>

        <footer className="statusbar">
          <div className="status-left">
            <span>◧ Files</span>
            <span>⑂ main</span>
            {usage && usage.agent && usage.hasData && (
              <div className="usage">
                {usage.model && <span className="usage-model">{usage.model}</span>}
                <UsageMeter label={t("usage.context")} pct={usage.contextPct} />
                {usage.agent === "claude" && usage.hasRateLimits && (
                  <>
                    <UsageMeter
                      label={t("usage.win5h")}
                      pct={usage.fiveHourPct}
                      reset={usage.fiveHourResetMs}
                    />
                    <UsageMeter
                      label={t("usage.win7d")}
                      pct={usage.sevenDayPct}
                      reset={usage.sevenDayResetMs}
                    />
                  </>
                )}
                {usage.agent === "codex" && usage.codexTotalTokens > 0 && (
                  <span className="usage-win">{fmtTokens(usage.codexTotalTokens)} tok</span>
                )}
              </div>
            )}
            {usage &&
              usage.agent === "claude" &&
              !usage.hasData &&
              !usagePromptOff && (
                <div className="usage-prompt">
                  <span>⚡ {t("usage.prompt")}</span>
                  <button className="usage-prompt-btn" onClick={enableUsage}>
                    {t("usage.promptEnable")}
                  </button>
                  <button
                    className="usage-prompt-x"
                    onClick={dismissUsagePrompt}
                    title={t("usage.promptDismiss")}
                  >
                    ✕
                  </button>
                </div>
              )}
          </div>
          <div className="status-right">
            <span>{fontSize}px</span>
            <span>{t("status.terminals", { n: tabs.length })}</span>
            <span>{theme.name}</span>
            <span>UTF-8</span>
            <span>Win 11</span>
          </div>
        </footer>
      </div>

      <SettingsPanel
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        currentTheme={theme.id}
        onSelectTheme={setTheme}
        hasBg={!!bgImage}
        overlay={overlay}
        onPickBg={setBgImage}
        onClearBg={() => setBgImage("")}
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
      />
    </LangContext.Provider>
  );
}

export default App;
