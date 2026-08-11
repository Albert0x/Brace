import { describe, expect, it } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useTabs } from "./useTabs";

const SESSION_KEY = "brace-session";
const HOME = "C:\\Users\\me";

// 存档必须在 renderHook 之前写好：restoreSession 只在 useState 初始化时跑一次
const seed = (payload: unknown) =>
  localStorage.setItem(SESSION_KEY, JSON.stringify(payload));

const tab = (cwd: string, shellType = "powershell", shellPath = "pwsh.exe") => ({
  cwd,
  shellPath,
  shellType,
});

describe("会话恢复", () => {
  it("没有存档时开一个默认标签", () => {
    const { result } = renderHook(() => useTabs(HOME));
    expect(result.current.tabs).toHaveLength(1);
    expect(result.current.activeId).toBe(result.current.tabs[0].id);
    expect(result.current.tabs[0].shellType).toBe("powershell");
  });

  it("按存档恢复每个标签的 shell 与目录", () => {
    seed({
      tabs: [tab("C:\\a", "bash", "bash.exe"), tab("C:\\b", "cmd", "cmd.exe")],
      activeIndex: 1,
    });
    const { result } = renderHook(() => useTabs(HOME));
    expect(result.current.tabs).toHaveLength(2);
    expect(result.current.tabs[0].initialCwd).toBe("C:\\a");
    expect(result.current.tabs[0].shellType).toBe("bash");
    expect(result.current.tabs[1].shellPath).toBe("cmd.exe");
    // activeIndex 指向第二个
    expect(result.current.activeId).toBe(result.current.tabs[1].id);
  });

  it("每个标签拿到各自的 id，不是共用一个", () => {
    seed({ tabs: [tab("C:\\a"), tab("C:\\b"), tab("C:\\c")], activeIndex: 0 });
    const { result } = renderHook(() => useTabs(HOME));
    const ids = new Set(result.current.tabs.map((t) => t.id));
    expect(ids.size).toBe(3);
  });

  it("存档不是合法 JSON 时照常开默认标签，不抛错", () => {
    localStorage.setItem(SESSION_KEY, "{ 这不是 json");
    const { result } = renderHook(() => useTabs(HOME));
    expect(result.current.tabs).toHaveLength(1);
    expect(result.current.activeId).toBeTruthy();
  });

  it("存档里 tabs 不是数组时回退默认标签", () => {
    seed({ tabs: "nope", activeIndex: 0 });
    const { result } = renderHook(() => useTabs(HOME));
    expect(result.current.tabs).toHaveLength(1);
  });

  it("存档里的非对象条目被过滤掉", () => {
    seed({ tabs: [tab("C:\\a"), null, 42, "x"], activeIndex: 0 });
    const { result } = renderHook(() => useTabs(HOME));
    expect(result.current.tabs).toHaveLength(1);
    expect(result.current.tabs[0].initialCwd).toBe("C:\\a");
  });

  it("activeIndex 越界时回退到第一个标签", () => {
    seed({ tabs: [tab("C:\\a"), tab("C:\\b")], activeIndex: 99 });
    const { result } = renderHook(() => useTabs(HOME));
    expect(result.current.activeId).toBe(result.current.tabs[0].id);
  });

  it("存档条目超过上限时截断，不会一口气拉起几百个 shell", () => {
    seed({
      tabs: Array.from({ length: 50 }, (_, i) => tab(`C:\\d${i}`)),
      activeIndex: 0,
    });
    const { result } = renderHook(() => useTabs(HOME));
    expect(result.current.tabs).toHaveLength(20);
  });
});

describe("增删切", () => {
  it("addTab 追加到末尾并立刻激活", () => {
    const { result } = renderHook(() => useTabs(HOME));
    const first = result.current.tabs[0].id;
    act(() => {
      result.current.addTab({
        cwd: "C:\\new",
        shellPath: "bash.exe",
        shellType: "bash",
      });
    });
    expect(result.current.tabs).toHaveLength(2);
    expect(result.current.tabs[1].initialCwd).toBe("C:\\new");
    expect(result.current.activeId).toBe(result.current.tabs[1].id);
    expect(result.current.activeId).not.toBe(first);
  });

  it("关掉活动标签后激活它前面那个", () => {
    seed({ tabs: [tab("C:\\a"), tab("C:\\b"), tab("C:\\c")], activeIndex: 2 });
    const { result } = renderHook(() => useTabs(HOME));
    const [, second, third] = result.current.tabs;
    act(() => result.current.removeTab(third.id));
    expect(result.current.tabs).toHaveLength(2);
    expect(result.current.activeId).toBe(second.id);
  });

  it("关掉第一个（也是活动的）标签后激活新的第一个", () => {
    seed({ tabs: [tab("C:\\a"), tab("C:\\b")], activeIndex: 0 });
    const { result } = renderHook(() => useTabs(HOME));
    const [first, second] = result.current.tabs;
    act(() => result.current.removeTab(first.id));
    expect(result.current.activeId).toBe(second.id);
  });

  // 这条是回归测试：曾经出现过「鼠标关掉一个非活动标签后，再按 Ctrl+W 把它复活」
  it("关掉非活动标签不改变 activeId", () => {
    seed({ tabs: [tab("C:\\a"), tab("C:\\b")], activeIndex: 1 });
    const { result } = renderHook(() => useTabs(HOME));
    const [first, second] = result.current.tabs;
    act(() => result.current.removeTab(first.id));
    expect(result.current.tabs).toHaveLength(1);
    expect(result.current.activeId).toBe(second.id);
  });

  it("关掉最后一个标签后 activeId 清空，不悬空指向已销毁的会话", () => {
    const { result } = renderHook(() => useTabs(HOME));
    act(() => result.current.removeTab(result.current.tabs[0].id));
    expect(result.current.tabs).toHaveLength(0);
    expect(result.current.activeId).toBe("");
  });

  it("关掉不存在的 id 时什么都不做", () => {
    const { result } = renderHook(() => useTabs(HOME));
    const before = result.current.tabs[0].id;
    act(() => result.current.removeTab("no-such-id"));
    expect(result.current.tabs).toHaveLength(1);
    expect(result.current.activeId).toBe(before);
  });

  it("switchTab 正向和反向都循环", () => {
    seed({ tabs: [tab("C:\\a"), tab("C:\\b"), tab("C:\\c")], activeIndex: 0 });
    const { result } = renderHook(() => useTabs(HOME));
    const ids = result.current.tabs.map((t) => t.id);

    act(() => result.current.switchTab(1));
    expect(result.current.activeId).toBe(ids[1]);
    act(() => result.current.switchTab(1));
    act(() => result.current.switchTab(1));
    expect(result.current.activeId, "走到末尾要绕回第一个").toBe(ids[0]);
    act(() => result.current.switchTab(-1));
    expect(result.current.activeId, "反向从第一个绕到最后").toBe(ids[2]);
  });

  it("只有一个标签时 switchTab 不动", () => {
    const { result } = renderHook(() => useTabs(HOME));
    const only = result.current.activeId;
    act(() => result.current.switchTab(1));
    expect(result.current.activeId).toBe(only);
  });
});

describe("每标签目录", () => {
  it("activeCwd 在没有上报时回退到 home", () => {
    const { result } = renderHook(() => useTabs(HOME));
    expect(result.current.activeCwd).toBe(HOME);
  });

  it("handleCwd 上报后 activeCwd 跟着变", () => {
    const { result } = renderHook(() => useTabs(HOME));
    const id = result.current.activeId;
    act(() => result.current.handleCwd(id, "D:\\work"));
    expect(result.current.activeCwd).toBe("D:\\work");
    expect(result.current.cwdMap[id]).toBe("D:\\work");
  });

  it("上报相同路径不会产出新的 cwdMap 对象", () => {
    const { result } = renderHook(() => useTabs(HOME));
    const id = result.current.activeId;
    act(() => result.current.handleCwd(id, "D:\\work"));
    const snapshot = result.current.cwdMap;
    act(() => result.current.handleCwd(id, "D:\\work"));
    expect(result.current.cwdMap).toBe(snapshot);
  });

  it("关标签时连它的 cwd 记录一起清掉，不留垃圾", () => {
    seed({ tabs: [tab("C:\\a"), tab("C:\\b")], activeIndex: 0 });
    const { result } = renderHook(() => useTabs(HOME));
    const [first] = result.current.tabs;
    act(() => result.current.handleCwd(first.id, "D:\\gone"));
    expect(result.current.cwdMap[first.id]).toBe("D:\\gone");
    act(() => result.current.removeTab(first.id));
    expect(first.id in result.current.cwdMap).toBe(false);
  });
});

describe("会话退出状态", () => {
  it("默认没有任何标签处于已退出", () => {
    const { result } = renderHook(() => useTabs(HOME));
    expect(result.current.exitedMap).toEqual({});
  });

  it("markExited 记下退出码", () => {
    const { result } = renderHook(() => useTabs(HOME));
    const id = result.current.activeId;
    act(() => result.current.markExited(id, 1));
    expect(result.current.exitedMap[id]).toBe(1);
  });

  it("退出码为 0 也算已退出，不能被当成「没退出」", () => {
    // 正常敲 exit 退出的退出码就是 0，这是最常见的情况。
    // 用 `id in map` 判断而不是取值真假，就是为了不掉进这个坑
    const { result } = renderHook(() => useTabs(HOME));
    const id = result.current.activeId;
    act(() => result.current.markExited(id, 0));
    expect(id in result.current.exitedMap).toBe(true);
  });

  it("拿不到退出码时记 null，同样算已退出", () => {
    const { result } = renderHook(() => useTabs(HOME));
    const id = result.current.activeId;
    act(() => result.current.markExited(id, null));
    expect(id in result.current.exitedMap).toBe(true);
    expect(result.current.exitedMap[id]).toBeNull();
  });

  it("clearExited 把标签恢复成活的（原地重启后）", () => {
    const { result } = renderHook(() => useTabs(HOME));
    const id = result.current.activeId;
    act(() => result.current.markExited(id, 1));
    act(() => result.current.clearExited(id));
    expect(id in result.current.exitedMap).toBe(false);
  });

  it("重启后再退出能再次标记", () => {
    const { result } = renderHook(() => useTabs(HOME));
    const id = result.current.activeId;
    act(() => result.current.markExited(id, 1));
    act(() => result.current.clearExited(id));
    act(() => result.current.markExited(id, 130));
    expect(result.current.exitedMap[id]).toBe(130);
  });

  it("clearExited 对本来就没记录的 id 不产生新对象", () => {
    const { result } = renderHook(() => useTabs(HOME));
    const snapshot = result.current.exitedMap;
    act(() => result.current.clearExited("no-such-id"));
    expect(result.current.exitedMap).toBe(snapshot);
  });

  it("给已经关掉的标签记退出状态不会留下孤儿条目", () => {
    // 关标签时后端会 kill 进程，那也会发一次退出事件。它要是比组件卸载先到，
    // 这张表就会攒一条永远显示不出来、也永远清不掉的记录
    const { result } = renderHook(() => useTabs(HOME));
    const id = result.current.activeId;
    act(() => result.current.removeTab(id));
    act(() => result.current.markExited(id, 1));
    expect(id in result.current.exitedMap).toBe(false);
  });

  it("关标签时连它的退出记录一起清掉", () => {
    seed({ tabs: [tab("C:\\a"), tab("C:\\b")], activeIndex: 0 });
    const { result } = renderHook(() => useTabs(HOME));
    const [first] = result.current.tabs;
    act(() => result.current.markExited(first.id, 1));
    act(() => result.current.removeTab(first.id));
    expect(first.id in result.current.exitedMap).toBe(false);
  });
});

describe("存档写回", () => {
  it("存的是实时 cwd 与 activeIndex，不是 tab id", () => {
    const { result } = renderHook(() => useTabs(HOME));
    const id = result.current.activeId;
    act(() => result.current.handleCwd(id, "D:\\live"));

    const saved = JSON.parse(localStorage.getItem(SESSION_KEY) ?? "{}");
    expect(saved.tabs).toHaveLength(1);
    expect(saved.tabs[0].cwd).toBe("D:\\live");
    expect(saved.tabs[0]).not.toHaveProperty("id");
    expect(saved.activeIndex).toBe(0);
  });

  it("没有实时 cwd 时存回 initialCwd", () => {
    seed({ tabs: [tab("C:\\start")], activeIndex: 0 });
    const { result } = renderHook(() => useTabs(HOME));
    expect(result.current.tabs[0].initialCwd).toBe("C:\\start");

    const saved = JSON.parse(localStorage.getItem(SESSION_KEY) ?? "{}");
    expect(saved.tabs[0].cwd).toBe("C:\\start");
  });

  it("activeIndex 跟着当前激活的标签走", () => {
    seed({ tabs: [tab("C:\\a"), tab("C:\\b")], activeIndex: 0 });
    const { result } = renderHook(() => useTabs(HOME));
    act(() => result.current.switchTab(1));

    const saved = JSON.parse(localStorage.getItem(SESSION_KEY) ?? "{}");
    expect(saved.activeIndex).toBe(1);
  });
});
