import { useT } from "../i18n";
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
  gitStatus,
  onOpenGit,
  profiles,
  usage: usageState,
  fontSize,
  tabCount,
  themeName,
  osVersion,
}: {
  gitStatus: GitStatus | null;
  onOpenGit: () => void;
  profiles: ReturnType<typeof useProfiles>;
  usage: ReturnType<typeof useUsage>;
  fontSize: number;
  tabCount: number;
  themeName: string;
  osVersion: string;
}) {
  const t = useT();
  const { store, active, switchTo, menuOpen, setMenuOpen } = profiles;
  const { usage, showPrompt, enableUsage, dismissPrompt } = usageState;

  return (
    <footer className="statusbar">
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

      <div className="status-right">
        <span>{fontSize}px</span>
        <span>{t("status.terminals", { n: tabCount })}</span>
        <span>{themeName}</span>
        <span>UTF-8</span>
        {osVersion && <span>{osVersion}</span>}
      </div>
    </footer>
  );
}
