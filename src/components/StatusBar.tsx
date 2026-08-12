import { useRef } from "react";
import { useT } from "../i18n";
import { usePersistedNumber } from "../hooks/usePersisted";
import type { GitStatus } from "./FileTree";
import type { useProfiles } from "../hooks/useProfiles";
import type { useUsage } from "../hooks/useUsage";

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

// 底部状态栏。profiles / usage 直接收 hook 的整个返回值——它们本来就是一整块
// 内聚状态，拆成十几个 props 只会让调用处更长
export default function StatusBar({
  cwd,
  gitStatus,
  onOpenGit,
  profiles,
  usage: usageState,
}: {
  cwd: string;
  gitStatus: GitStatus | null;
  onOpenGit: () => void;
  profiles: ReturnType<typeof useProfiles>;
  usage: ReturnType<typeof useUsage>;
}) {
  const t = useT();
  const { store, active, switchTo, menuOpen, setMenuOpen } = profiles;
  const { usage, showPrompt, enableUsage, dismissPrompt } = usageState;
  const [height, setHeight] = usePersistedNumber("brace-status-height", 46);
  const dragStart = useRef<{ y: number; height: number } | null>(null);
  const pathParts = cwd.split(/[\\/]/).filter(Boolean);
  const isHomePath = /^\/Users\/[^/]+(?:\/|$)/.test(cwd);
  const pathLabels = isHomePath
    ? ["Home", ...pathParts.slice(2)]
    : pathParts.length > 0
      ? pathParts
      : [cwd || "—"];
  const visiblePathLabels = pathLabels.length > 6
    ? ["…", ...pathLabels.slice(-5)]
    : pathLabels;

  const startResize = (e: React.PointerEvent<HTMLDivElement>) => {
    dragStart.current = { y: e.clientY, height };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const resize = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!dragStart.current) return;
    setHeight(Math.max(46, Math.min(92, dragStart.current.height + dragStart.current.y - e.clientY)));
  };

  return (
    <footer className="statusbar" style={{ height: Math.max(46, height) }}>
      <div
        className="status-resize-handle"
        onPointerDown={startResize}
        onPointerMove={resize}
        onPointerUp={() => { dragStart.current = null; }}
        onPointerCancel={() => { dragStart.current = null; }}
        title="Drag to resize"
      />
      <div className="status-left">
        <span>◧ Files</span>
        <span
          className={"status-git" + (gitStatus?.isRepo ? " clickable" : "")}
          onClick={() => gitStatus?.isRepo && onOpenGit()}
          title={gitStatus?.isRepo ? t("git.title") : ""}
        >
          ⑂ {gitStatus?.branch || "—"}
          {gitStatus && gitStatus.changedCount > 0
            ? ` ±${gitStatus.changedCount}`
            : ""}
        </span>

        {/* 一个配置组都没有时不显示，免得状态栏挂个没用的图标 */}
        {store.profiles.length > 0 && (
          <span className="status-profile-wrap">
            <span
              className="status-profile clickable"
              title={t("status.profile")}
              onClick={(e) => {
                e.stopPropagation();
                setMenuOpen((v) => !v);
              }}
            >
              🔑 {active?.name.trim() || t("profiles.none")}
            </span>
            {menuOpen && (
              <div className="profile-menu" onClick={(e) => e.stopPropagation()}>
                <div className="profile-menu-hint">{t("profiles.newTabOnly")}</div>
                <div
                  className={
                    "profile-item" + (store.activeId === "" ? " selected" : "")
                  }
                  onClick={() => switchTo("")}
                >
                  <span>{t("profiles.none")}</span>
                  <span className="profile-item-desc">{t("profiles.noneDesc")}</span>
                </div>
                {store.profiles.map((p) => (
                  <div
                    key={p.id}
                    className={
                      "profile-item" + (p.id === store.activeId ? " selected" : "")
                    }
                    onClick={() => switchTo(p.id)}
                  >
                    <span>{p.name.trim() || t("profiles.untitled")}</span>
                    <span className="profile-item-desc">{p.vars.length}</span>
                  </div>
                ))}
              </div>
            )}
          </span>
        )}

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
              <span className="usage-win">
                {fmtTokens(usage.codexTotalTokens)} tok
              </span>
            )}
          </div>
        )}

        {showPrompt && (
          <div className="usage-prompt">
            <span>⚡ {t("usage.prompt")}</span>
            <button className="usage-prompt-btn" onClick={enableUsage}>
              {t("usage.promptEnable")}
            </button>
            <button
              className="usage-prompt-x"
              onClick={dismissPrompt}
              title={t("usage.promptDismiss")}
            >
              ✕
            </button>
          </div>
        )}
      </div>
      <div className="status-path" title={cwd} aria-label={cwd}>
        {visiblePathLabels.map((part, index) => (
          <span className="status-path-part" key={`${part}-${index}`}>
            {index > 0 && <span className="status-path-separator" aria-hidden="true">›</span>}
            <span className={index === 0 && part === "Home" ? "status-path-home" : ""}>
              {index === 0 && part === "Home" && <span aria-hidden="true">⌂ </span>}
              {part}
            </span>
          </span>
        ))}
      </div>
    </footer>
  );
}
