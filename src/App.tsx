import { useState, useEffect, useCallback, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import type { SearchAddon } from "@xterm/addon-search";
import TerminalView from "./components/TerminalView";
import FileTree from "./components/FileTree";
import SettingsPanel from "./components/SettingsPanel";
import { THEMES, applyTheme, type Theme } from "./themes";
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

function App() {
  const appWindow = getCurrentWindow();

  const [tabs, setTabs] = useState<Tab[]>(() => [
    { id: crypto.randomUUID(), initialCwd: "", shellPath: "", shellType: "powershell" },
  ]);
  const [activeId, setActiveId] = useState<string>(() => tabs[0].id);

  // 可用 shell 检测
  const [shells, setShells] = useState<ShellInfo[]>([]);
  const [shellMenu, setShellMenu] = useState(false);
  useEffect(() => {
    invoke<ShellInfo[]>("detect_shells").then(setShells).catch(() => {});
  }, []);
  const defaultShell = shells.find((s) => s.id === "powershell") ?? shells[0];

  // 主题
  const [theme, setTheme] = useState<Theme>(
    () => THEMES.find((t) => t.id === localStorage.getItem("ht-theme")) ?? THEMES[0],
  );
  const [settingsOpen, setSettingsOpen] = useState(false);
  useEffect(() => {
    applyTheme(theme);
    localStorage.setItem("ht-theme", theme.id);
  }, [theme]);

  // 背景图 + 遮罩浓度
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
      // 图太大超配额时仅本会话生效
    }
  }, [bgImage]);
  useEffect(() => {
    localStorage.setItem("ht-overlay", String(overlay));
  }, [overlay]);

  // 终端字体大小
  const [fontSize, setFontSize] = useState<number>(
    () => Number(localStorage.getItem("ht-fontsize")) || 14,
  );
  useEffect(() => {
    localStorage.setItem("ht-fontsize", String(fontSize));
  }, [fontSize]);

  // 文件树根目录：跟随各终端上报的 cwd
  const [homeCwd, setHomeCwd] = useState("");
  const [cwdMap, setCwdMap] = useState<Record<string, string>>({});
  useEffect(() => {
    invoke<string>("home_dir").then(setHomeCwd).catch(() => {});
  }, []);
  const handleCwd = useCallback((sid: string, path: string) => {
    setCwdMap((m) => (m[sid] === path ? m : { ...m, [sid]: path }));
  }, []);
  const activeCwd = cwdMap[activeId] ?? homeCwd;

  // 标签标题用当前目录名，未上报时回退 shell 名
  const tabLabel = (tab: Tab) => {
    const cwd = cwdMap[tab.id];
    const b = cwd ? cwd.split(/[\\/]/).filter(Boolean).pop() : "";
    if (b) return b;
    const s = shells.find((x) => x.shell_type === tab.shellType);
    return s?.name ?? "Terminal";
  };

  // 文件树双击文件夹 → 让活动终端 cd 过去
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
    setTabs((t) => [
      ...t,
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
      const idx = prev.findIndex((t) => t.id === id);
      const next = prev.filter((t) => t.id !== id);
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
      const idx = prev.findIndex((t) => t.id === activeId);
      setActiveId(prev[(idx + dir + prev.length) % prev.length].id);
      return prev;
    });
  };

  // shell 菜单点外关闭
  useEffect(() => {
    if (!shellMenu) return;
    const close = () => setShellMenu(false);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [shellMenu]);

  // 快捷键
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
    <>
      {/* 背景层 */}
      <div
        className="bg-layer"
        style={{ backgroundImage: bgImage ? `url(${bgImage})` : "none" }}
      />
      <div
        className="bg-overlay"
        style={{ background: theme.ui.base, opacity: bgImage ? overlay : 0.9 }}
      />

      <div className="app">
        {/* ---------- 顶栏 ---------- */}
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
                    title="关闭标签 (Ctrl+W)"
                    onClick={(e) => closeTab(tab.id, e)}
                  >
                    ×
                  </span>
                )}
              </div>
            ))}

            <div className="tab-add-group">
              <button
                className="tab-add"
                title="新建标签 (Ctrl+T)"
                onClick={() => addTab()}
              >
                ＋
              </button>
              {shells.length > 1 && (
                <button
                  className="tab-add-caret"
                  title="选择 Shell"
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
                placeholder="搜索 (Ctrl+F)"
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
              title="设置"
              onClick={() => setSettingsOpen(true)}
            >
              ⚙
            </button>
            <div className="win-controls">
              <button className="win-btn" title="最小化" onClick={() => appWindow.minimize()}>
                <svg width="10" height="10" viewBox="0 0 10 10">
                  <rect y="4.5" width="10" height="1" fill="currentColor" />
                </svg>
              </button>
              <button className="win-btn" title="最大化" onClick={() => appWindow.toggleMaximize()}>
                <svg width="10" height="10" viewBox="0 0 10 10">
                  <rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" />
                </svg>
              </button>
              <button className="win-btn win-close" title="关闭" onClick={() => appWindow.close()}>
                <svg width="10" height="10" viewBox="0 0 10 10">
                  <path d="M1 1 L9 9 M9 1 L1 9" stroke="currentColor" strokeWidth="1.2" />
                </svg>
              </button>
            </div>
          </div>
        </header>

        {/* ---------- 中部：侧边栏 + 终端 ---------- */}
        <div className="body">
          <aside className="sidebar">
            <FileTree rootPath={activeCwd} onOpenDir={openDirInTerminal} />
          </aside>

          <main className="main">
            {tabs.map((tab) => (
              <TerminalView
                key={tab.id}
                sessionId={tab.id}
                active={tab.id === activeId}
                onCwd={handleCwd}
                termTheme={theme.terminal}
                initialCwd={tab.initialCwd}
                fontSize={fontSize}
                shellPath={tab.shellPath}
                shellType={tab.shellType}
                onRegisterSearch={registerSearch}
                onUnregisterSearch={unregisterSearch}
              />
            ))}
            {tabs.length === 0 && (
              <div className="empty-hint">没有终端，点右上角 ＋ 或按 Ctrl+T 新建</div>
            )}
          </main>
        </div>

        {/* ---------- 底部：状态栏 ---------- */}
        <footer className="statusbar">
          <div className="status-left">
            <span>◧ Files</span>
            <span>⑂ main</span>
          </div>
          <div className="status-right">
            <span>{fontSize}px</span>
            <span>{tabs.length} 个终端</span>
            <span>{theme.name}</span>
            <span>UTF-8</span>
            <span>Win 11</span>
          </div>
        </footer>
      </div>

      {/* ---------- 设置面板 ---------- */}
      <SettingsPanel
        open={settingsOpen}
        current={theme.id}
        onSelect={setTheme}
        onClose={() => setSettingsOpen(false)}
        hasBg={!!bgImage}
        overlay={overlay}
        onPickBg={setBgImage}
        onClearBg={() => setBgImage("")}
        onOverlay={setOverlay}
      />
    </>
  );
}

export default App;
