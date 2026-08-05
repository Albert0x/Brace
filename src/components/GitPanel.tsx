import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useT } from "../i18n";
import type { GitStatus } from "./FileTree";

// 默认的提交类型：Conventional Commits / Angular 那套通用集，不是本项目
// CONTRIBUTING 里那七个——Brace 是给别人用的工具，别人的仓库没义务守我们的规范。
// 团队有自己一套的，在 设置 → 常规 → Git 里覆盖掉
export const DEFAULT_COMMIT_TYPES =
  "feat, fix, docs, style, refactor, perf, test, build, ci, chore, revert";

// 自定义列表按逗号（中英文都认）或空白切开；全填了空格就退回默认，
// 免得误清空之后面板上一个类型都没有
export function parseCommitTypes(raw: string): string[] {
  const list = raw
    .split(/[,，\s]+/)
    .map((s) => s.trim())
    .filter(Boolean);
  return list.length ? list : parseCommitTypes(DEFAULT_COMMIT_TYPES);
}

// 拼成 type(scope): 描述。没选类型就原样用，不强制——
// 临时提交、WIP 这种场合硬套规范只会让人绕过面板去敲命令行
function composeMessage(type: string, scope: string, body: string): string {
  const text = body.trim();
  if (!type) return text;
  const head = scope.trim() ? `${type}(${scope.trim()}): ` : `${type}: `;
  // 多行时前缀只落在第一行（也就是 subject），空行之后的正文段落原样保留
  return head + text;
}

// 状态码 → 装饰色 class（复用文件树那套 git-* 颜色）
const STATUS_CLASS: Record<string, string> = {
  M: "git-modified",
  A: "git-added",
  "?": "git-untracked",
  D: "git-deleted",
  R: "git-renamed",
};

// diff 的一行归类，决定着色
function lineClass(line: string): string {
  if (line.startsWith("@@")) return "hunk";
  // +++ / --- 是文件头，不是增删行
  if (line.startsWith("+++") || line.startsWith("---")) return "meta";
  if (line.startsWith("diff ") || line.startsWith("index ")) return "meta";
  if (line.startsWith("+")) return "add";
  if (line.startsWith("-")) return "del";
  return "";
}

function DiffView({ text }: { text: string }) {
  const lines = text.split("\n");
  return (
    <pre className="git-diff">
      {lines.map((l, i) => (
        <div key={i} className={"git-diff-line " + lineClass(l)}>
          {l || " "}
        </div>
      ))}
    </pre>
  );
}

// Git 提交面板：勾选改动 → 看 diff → 写信息 → Commit / Commit & Push。
// 刻意不做逐行暂存——那是编辑器的活，在终端侧边栏里做只会把自己搞复杂。
// 但"提交前看得见自己在提交什么"是底线，所以 diff 和按文件勾选必须有
export default function GitPanel({
  cwd,
  gitStatus,
  commitTypes,
  onClose,
  onDone,
}: {
  cwd: string;
  gitStatus: GitStatus | null;
  commitTypes: string[];
  onClose: () => void;
  onDone: () => void; // 提交后刷新 gitStatus
}) {
  const t = useT();
  const [message, setMessage] = useState("");
  const [type, setType] = useState("");
  const [scope, setScope] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; msg: string } | null>(null);

  // 只列真正的改动，忽略 ignored（"!"）
  const entries = Object.entries(gitStatus?.files ?? {}).filter(
    ([, code]) => code !== "!",
  );

  // 默认全选，保持原来"点开就能提交"的手感；取消勾选是主动行为
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [initialized, setInitialized] = useState(false);
  useEffect(() => {
    if (initialized) return;
    if (entries.length) {
      setSelected(new Set(entries.map(([p]) => p)));
      setInitialized(true);
    }
  }, [entries, initialized]);

  // 展开中的文件 → 它的 diff（undefined 表示还在加载）
  const [openPath, setOpenPath] = useState("");
  const [diff, setDiff] = useState<string | undefined>(undefined);
  const toggleDiff = (path: string) => {
    if (openPath === path) {
      setOpenPath("");
      return;
    }
    setOpenPath(path);
    setDiff(undefined);
    invoke<string>("git_diff", { cwd, path })
      .then(setDiff)
      .catch((e) => setDiff(String(e)));
  };

  const toggleFile = (path: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  const allSelected = entries.length > 0 && selected.size === entries.length;
  const toggleAll = () =>
    setSelected(allSelected ? new Set() : new Set(entries.map(([p]) => p)));

  const composed = composeMessage(type, scope, message);
  const canCommit = !busy && !!message.trim() && selected.size > 0;
  const fileName = (abs: string) => abs.split(/[\\/]/).pop() || abs;

  const commit = async (push: boolean) => {
    if (!canCommit) return;
    setBusy(true);
    setResult(null);
    try {
      await invoke<string>("git_commit", {
        cwd,
        message: composed,
        push,
        paths: [...selected],
        // 全选时走 add -A，省得把几百个路径拼进命令行
        all: allSelected,
      });
      setResult({ ok: true, msg: push ? t("git.pushed") : t("git.committed") });
      setMessage("");
      // 类型和范围也清掉：下一次提交多半不是同一类改动，留着容易顺手误用
      setType("");
      setScope("");
      setOpenPath("");
      setInitialized(false); // 让下一批改动重新默认全选
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
              {entries.length > 0 ? (
                <>
                  <label className="git-all">
                    <input
                      type="checkbox"
                      checked={allSelected}
                      onChange={toggleAll}
                    />
                    {t("git.selected", {
                      n: selected.size,
                      total: entries.length,
                    })}
                  </label>
                </>
              ) : (
                t("git.noChanges")
              )}
            </div>

            <div className="git-list">
              {entries.map(([abs, code]) => (
                <div key={abs}>
                  <div className="git-item">
                    <input
                      type="checkbox"
                      checked={selected.has(abs)}
                      onChange={() => toggleFile(abs)}
                      onClick={(e) => e.stopPropagation()}
                    />
                    <span className={"git-badge " + (STATUS_CLASS[code] || "")}>
                      {code}
                    </span>
                    <span
                      className={
                        "git-file clickable" + (openPath === abs ? " open" : "")
                      }
                      title={abs}
                      onClick={() => toggleDiff(abs)}
                    >
                      {fileName(abs)}
                    </span>
                  </div>
                  {openPath === abs &&
                    (diff === undefined ? (
                      <div className="git-diff-loading">…</div>
                    ) : diff.trim() ? (
                      <DiffView text={diff} />
                    ) : (
                      <div className="git-diff-loading">{t("git.noDiff")}</div>
                    ))}
                </div>
              ))}
            </div>

            <div className="git-type-row">
              {commitTypes.map((ty) => (
                <button
                  key={ty}
                  className={"git-type" + (type === ty ? " on" : "")}
                  // 再点一次取消选中：不想守规范的时候不该被逼着守
                  onClick={() => setType(type === ty ? "" : ty)}
                >
                  {ty}
                </button>
              ))}
              <input
                className="git-scope"
                value={scope}
                spellCheck={false}
                placeholder={t("git.scopePlaceholder")}
                onChange={(e) => setScope(e.target.value)}
              />
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
            {/* 选了类型才预览——没选的话预览就等于原文，纯占地方。
                完整显示而不是只给第一行：多行信息里「标题和正文之间要不要空行」
                这种坑，只有把最终结果整个摆出来才看得见 */}
            {type && message.trim() && (
              <pre className="git-preview">{composed}</pre>
            )}
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
