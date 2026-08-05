import { useCallback, useEffect, useRef, useState } from "react";

export interface Tab {
  id: string;
  initialCwd: string;
  shellPath: string;
  shellType: string;
}

interface StoredTab {
  cwd: string;
  shellPath: string;
  shellType: string;
}

// 关掉再打开时把上次的标签组（shell + 目录）原样开回来。存的是 shell 和目录，
// 不是 tab id——PTY 会话是新的，id 每次重新生成
const SESSION_KEY = "brace-session";
// 恢复上限。存档理论上不会很大，但一个坏掉/手改过的存档不该让 Brace 启动时
// 一口气拉起几百个 shell 进程
const MAX_RESTORED_TABS = 20;

function newTab(t?: Partial<StoredTab>): Tab {
  return {
    id: crypto.randomUUID(),
    initialCwd: t?.cwd ?? "",
    shellPath: t?.shellPath ?? "",
    shellType: t?.shellType ?? "powershell",
  };
}

function restoreSession(): { tabs: Tab[]; activeId: string } {
  let stored: StoredTab[] = [];
  let activeIndex = 0;
  try {
    const raw = localStorage.getItem(SESSION_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed?.tabs)) {
        stored = parsed.tabs
          .filter((x: unknown) => x && typeof x === "object")
          .slice(0, MAX_RESTORED_TABS);
      }
      if (Number.isInteger(parsed?.activeIndex)) activeIndex = parsed.activeIndex;
    }
  } catch {
    // 存档坏了（手改过、写到一半断电）就当没有，照常开一个默认标签，
    // 总比启动时白屏强
  }
  const tabs = stored.length ? stored.map((s) => newTab(s)) : [newTab()];
  const active = tabs[activeIndex] ?? tabs[0];
  return { tabs, activeId: active.id };
}

/**
 * 标签页状态：增删切、每个标签的当前目录、会话存档。
 *
 * 所有操作函数都从 ref 读最新的 tabs/activeId，因此引用是稳定的，
 * 调用方（比如全局快捷键监听）不用把 tabs 塞进依赖数组。
 * 这类依赖遗漏正是之前那个 bug 的来源：鼠标关掉一个非活动标签后 activeId 没变、
 * effect 不重建，快捷键里的闭包还攥着旧列表，再按 Ctrl+W 就把关掉的标签"复活"了。
 */
export function useTabs(homeCwd: string) {
  const [session] = useState(restoreSession);
  const [tabs, setTabs] = useState<Tab[]>(session.tabs);
  const [activeId, setActiveId] = useState<string>(session.activeId);
  const [cwdMap, setCwdMap] = useState<Record<string, string>>({});

  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const activeIdRef = useRef(activeId);
  activeIdRef.current = activeId;

  const addTab = useCallback(
    (opts: { cwd: string; shellPath: string; shellType: string }) => {
      const tab = newTab(opts);
      setTabs((prev) => [...prev, tab]);
      setActiveId(tab.id);
      return tab;
    },
    [],
  );

  const removeTab = useCallback((id: string) => {
    const current = tabsRef.current;
    const idx = current.findIndex((x) => x.id === id);
    if (idx < 0) return;
    const next = current.filter((x) => x.id !== id);
    setTabs(next);
    // 会话没了，它的 cwd 记录也得跟着走，否则 cwdMap 会一直攒已关闭标签的条目
    setCwdMap((m) => {
      if (!(id in m)) return m;
      const { [id]: _gone, ...rest } = m;
      return rest;
    });
    if (id === activeIdRef.current) {
      // 关掉最后一个就把 activeId 清空，别让它悬空指向已销毁的会话
      setActiveId(next.length ? next[Math.max(0, idx - 1)].id : "");
    }
  }, []);

  const switchTab = useCallback((dir: number) => {
    const current = tabsRef.current;
    if (current.length < 2) return;
    const idx = current.findIndex((x) => x.id === activeIdRef.current);
    setActiveId(current[(idx + dir + current.length) % current.length].id);
  }, []);

  const handleCwd = useCallback((sid: string, path: string) => {
    setCwdMap((m) => (m[sid] === path ? m : { ...m, [sid]: path }));
  }, []);

  const activeCwd = cwdMap[activeId] ?? homeCwd;

  // 存档：每个标签的 shell 和实时 cwd。「上次在哪个目录」对每个标签都成立，
  // 不再是全局只记一个 last-cwd
  useEffect(() => {
    const payload = {
      tabs: tabs.map((tab) => ({
        cwd: cwdMap[tab.id] ?? tab.initialCwd,
        shellPath: tab.shellPath,
        shellType: tab.shellType,
      })),
      activeIndex: Math.max(
        0,
        tabs.findIndex((tab) => tab.id === activeId),
      ),
    };
    try {
      localStorage.setItem(SESSION_KEY, JSON.stringify(payload));
    } catch {
      // 配额满之类的极端情况，恢复不了就下次开默认标签，不值得打断用户
    }
  }, [tabs, cwdMap, activeId]);

  return {
    tabs,
    activeId,
    setActiveId,
    addTab,
    removeTab,
    switchTab,
    cwdMap,
    handleCwd,
    activeCwd,
  };
}
