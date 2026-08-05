import { useEffect, useRef } from "react";

// 定时轮询。窗口最小化时自动停——没人看得见还每 10 秒扫一遍全系统进程纯属浪费电；
// 重新可见立刻补一次，不用干等下一个周期。
// 回调走 ref 拿最新版本，所以定时器不会因为组件重渲染反复重建；deps 变化（比如切标签）
// 则立刻触发一次，保持"切过去马上看到新数据"的手感
export function usePolling(
  fn: () => void,
  intervalMs: number,
  deps: unknown[] = [],
) {
  const saved = useRef(fn);
  saved.current = fn;
  useEffect(() => {
    const tick = () => {
      if (!document.hidden) saved.current();
    };
    tick();
    const id = setInterval(tick, intervalMs);
    document.addEventListener("visibilitychange", tick);
    return () => {
      clearInterval(id);
      document.removeEventListener("visibilitychange", tick);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [intervalMs, ...deps]);
}
