import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useT } from "../i18n";
import type { GitStatus } from "./FileTree";

// 状态码 → 装饰色 class（复用文件树那套 git-* 颜色）
const STATUS_CLASS: Record<string, string> = {
  M: "git-modified",
  A: "git-added",
  "?": "git-untracked",
  D: "git-deleted",
  R: "git-renamed",
};

// 轻量 Git 提交面板：列改动 → 写信息 → Commit / Commit & Push。
// 刻意不做逐行暂存/diff —— 保持"小而独特"，重活留给终端里的 git 命令
export default function GitPanel({
  cwd,
  gitStatus,
  onClose,
  onDone,
}: {
  cwd: string;
  gitStatus: GitStatus | null;
  onClose: () => void;
  onDone: () => void; // 提交后刷新 gitStatus
}) {
  const t = useT();
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; msg: string } | null>(null);

  // 只列真正的改动，忽略 ignored（"!"）
  const entries = Object.entries(gitStatus?.files ?? {}).filter(
    ([, code]) => code !== "!",
  );
  const canCommit = !busy && !!message.trim() && entries.length > 0;
  const fileName = (abs: string) => abs.split(/[\\/]/).pop() || abs;

  const commit = async (push: boolean) => {
    if (!canCommit) return;
    setBusy(true);
    setResult(null);
    try {
      await invoke<string>("git_commit", { cwd, message, push });
      setResult({ ok: true, msg: push ? t("git.pushed") : t("git.committed") });
      setMessage("");
      onDone();
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
    setBusy(false);
  };

  return (
    <div className="git-overlay" onClick={onClose}>
      <div className="git-panel" onClick={(e) => e.stopPropagation()}>
        <div className="git-head">
          <span className="git-title">
            {t("git.title")}
            {gitStatus?.branch && (
              <span className="git-branch">⑂ {gitStatus.branch}</span>
            )}
          </span>
          <button className="git-x" onClick={onClose} title={t("git.close")}>
            ×
          </button>
        </div>

        {!gitStatus?.isRepo ? (
          <div className="git-empty">{t("git.notRepo")}</div>
        ) : (
          <>
            <div className="git-changes-head">
              {entries.length > 0
                ? t("git.changes", { n: entries.length })
                : t("git.noChanges")}
            </div>
            <div className="git-list">
              {entries.map(([abs, code]) => (
                <div className="git-item" key={abs}>
                  <span className={"git-badge " + (STATUS_CLASS[code] || "")}>
                    {code}
                  </span>
                  <span className="git-file" title={abs}>
                    {fileName(abs)}
                  </span>
                </div>
              ))}
            </div>
            <textarea
              className="git-message"
              placeholder={t("git.messagePlaceholder")}
              value={message}
              spellCheck={false}
              onChange={(e) => setMessage(e.target.value)}
              onKeyDown={(e) => {
                if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
                  e.preventDefault();
                  commit(false);
                }
              }}
            />
            {result && (
              <div className={"git-result" + (result.ok ? " ok" : " err")}>
                {result.msg}
              </div>
            )}
            <div className="git-actions">
              <button
                className="git-btn"
                disabled={!canCommit}
                onClick={() => commit(false)}
              >
                {busy ? t("git.committing") : t("git.commit")}
              </button>
              <button
                className="git-btn primary"
                disabled={!canCommit}
                onClick={() => commit(true)}
              >
                {t("git.commitPush")}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
