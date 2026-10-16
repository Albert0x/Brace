// 命令历史面板的纯逻辑：筛选、去重、时间显示。和界面分开放，方便测试

export interface HistoryEntry {
  id: number;
  command: string;
  exitCode: number;
  cwd: string;
  shellType: string;
  // unix 毫秒
  at: number;
  favorite: boolean;
}

// entries 已按新到旧排好（后端就是这么给的）。
// 同一条命令只显示最近的一次：隔几条又敲了一遍 git status，列表里不该出现两行。
// 收藏跟着命令走——任何一次被收藏，这一行就算收藏
export function visibleEntries(
  entries: HistoryEntry[],
  query: string,
  favoritesOnly: boolean,
): HistoryEntry[] {
  const favorites = new Set(entries.filter((e) => e.favorite).map((e) => e.command));
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const seen = new Set<string>();
  const out: HistoryEntry[] = [];
  for (const e of entries) {
    if (seen.has(e.command)) continue;
    seen.add(e.command);
    const favorite = favorites.has(e.command);
    if (favoritesOnly && !favorite) continue;
    // 多个词都要出现，顺序不限：「git push」能搜到「git push origin main」，
    // 「main push」也能
    const lower = e.command.toLowerCase();
    if (!words.every((w) => lower.includes(w))) continue;
    out.push({ ...e, favorite });
  }
  return out;
}

// 「3 分钟前」这种。key 和参数交给 i18n 去拼
export function relativeTime(at: number, now: number): { key: string; n?: number } {
  const s = Math.max(0, Math.floor((now - at) / 1000));
  if (s < 60) return { key: "history.justNow" };
  const m = Math.floor(s / 60);
  if (m < 60) return { key: "history.minutesAgo", n: m };
  const h = Math.floor(m / 60);
  if (h < 24) return { key: "history.hoursAgo", n: h };
  return { key: "history.daysAgo", n: Math.floor(h / 24) };
}

// 当前标签的 shell 能不能记历史。cmd 没有能取到上一条命令的钩子；
// SSH 是远端的 shell，集成脚本不在那边跑
export function historyUnsupportedReason(shellType: string | undefined): string | null {
  if (shellType === "cmd") return "history.noCmd";
  if (shellType === "ssh") return "history.noSsh";
  return null;
}
