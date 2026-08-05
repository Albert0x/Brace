import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { usePolling } from "./usePolling";

export interface UsageStats {
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

const PROMPT_KEY = "brace-usage-prompt";

// Claude / Codex 用量：按当前标签轮询后端（进程检测 + 读官方缓存），每 10s 一次。
// 外加「检测到在跑 claude 但用量没开」时的一次性引导提示。
export function useUsage(activeId: string) {
  const [usage, setUsage] = useState<UsageStats | null>(null);
  const [slConfigured, setSlConfigured] = useState(false);
  const [promptOff, setPromptOff] = useState(
    () => localStorage.getItem(PROMPT_KEY) === "off",
  );

  usePolling(
    () => {
      invoke<UsageStats>("usage_stats", { sessionId: activeId })
        .then(setUsage)
        .catch(() => {});
    },
    10000,
    [activeId],
  );

  // statusLine 是否已配置，只在「跑着 claude 却读不到数据」时才需要知道——
  // 它唯一的用途是决定要不要弹引导提示（不查的话，claude 刚启动的缓存空档期会误弹，
  // 缓存一写又自动变成用量条，看着像「没点却自动开了」）。
  // 早先它跟用量绑在一起每 10 秒读一次磁盘，正常有数据时那次读纯属白读
  const needSlCheck =
    usage?.agent === "claude" && !usage.hasData && !promptOff;
  useEffect(() => {
    if (!needSlCheck) return;
    invoke<{ configured: boolean }>("statusline_status")
      .then((s) => setSlConfigured(s.configured))
      .catch(() => {});
  }, [needSlCheck]);

  const enableUsage = () => {
    invoke("configure_statusline", { enable: true, force: false })
      .then(() => {
        setSlConfigured(true);
        return invoke<UsageStats>("usage_stats", { sessionId: activeId }).then(
          setUsage,
        );
      })
      .catch(() => {});
  };

  const dismissPrompt = () => {
    setPromptOff(true);
    localStorage.setItem(PROMPT_KEY, "off");
  };

  // 提示只在「确实在跑 claude、确实没数据、确实没配过、用户也没忽略过」时出现
  const showPrompt =
    usage?.agent === "claude" && !usage.hasData && !slConfigured && !promptOff;

  return { usage, showPrompt, enableUsage, dismissPrompt };
}
