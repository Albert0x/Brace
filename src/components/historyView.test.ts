import { describe, expect, it } from "vitest";
import {
  historyUnsupportedReason,
  relativeTime,
  visibleEntries,
  type HistoryEntry,
} from "./historyView";

let nextId = 100;
const e = (command: string, extra: Partial<HistoryEntry> = {}): HistoryEntry => ({
  id: nextId--,
  command,
  exitCode: 0,
  cwd: "D:/",
  shellType: "bash",
  at: 0,
  favorite: false,
  ...extra,
});

describe("命令历史列表", () => {
  // 后端给的顺序：新的在前
  const list = [
    e("git status"),
    e("git push origin main"),
    e("ls"),
    e("git status", { exitCode: 1 }),
    e("cargo test", { favorite: true }),
  ];

  it("同一条命令只留最近的一次", () => {
    const cmds = visibleEntries(list, "", false).map((x) => x.command);
    expect(cmds).toEqual(["git status", "git push origin main", "ls", "cargo test"]);
    // 留下的是最近那次（退出码 0），不是更早失败的那次
    expect(visibleEntries(list, "", false)[0].exitCode).toBe(0);
  });

  it("多个词都要出现，顺序不限，不分大小写", () => {
    expect(visibleEntries(list, "main PUSH", false).map((x) => x.command)).toEqual([
      "git push origin main",
    ]);
    expect(visibleEntries(list, "  git  ", false)).toHaveLength(2);
  });

  it("只看收藏", () => {
    expect(visibleEntries(list, "", true).map((x) => x.command)).toEqual(["cargo test"]);
  });

  it("更早的一次被收藏，最近那一行也算收藏", () => {
    const l = [e("make"), e("make", { favorite: true })];
    const v = visibleEntries(l, "", true);
    expect(v).toHaveLength(1);
    expect(v[0].favorite).toBe(true);
    expect(v[0].id).toBe(l[0].id);
  });
});

describe("相对时间", () => {
  const now = 10_000_000_000;
  it.each([
    [now - 5_000, { key: "history.justNow" }],
    [now - 3 * 60_000, { key: "history.minutesAgo", n: 3 }],
    [now - 2 * 3_600_000, { key: "history.hoursAgo", n: 2 }],
    [now - 3 * 86_400_000, { key: "history.daysAgo", n: 3 }],
    // 时钟往回拨过：不显示负数
    [now + 60_000, { key: "history.justNow" }],
  ])("%s", (at, want) => {
    expect(relativeTime(at, now)).toEqual(want);
  });
});

describe("哪些 shell 不记历史", () => {
  it("cmd 和 SSH 不记，其余都记（default 在 Windows 上就是 PowerShell）", () => {
    expect(historyUnsupportedReason("cmd")).toBe("history.noCmd");
    expect(historyUnsupportedReason("ssh")).toBe("history.noSsh");
    for (const st of ["bash", "powershell", "default", undefined]) {
      expect(historyUnsupportedReason(st)).toBeNull();
    }
  });
});
