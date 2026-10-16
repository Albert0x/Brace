import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useT } from "../i18n";
import {
  historyUnsupportedReason,
  relativeTime,
  visibleEntries,
  type HistoryEntry,
} from "./historyView";

interface Props {
  // 当前标签的 shell 类型：cmd / SSH 不记历史，要明说，而不是给一个看起来坏掉的空列表
  shellType: string | undefined;
  // 插入到当前终端，不执行。返回 false 表示没能安全插入（已改为复制到剪贴板）
  onInsert: (command: string) => Promise<boolean>;
  onClose: () => void;
}

export default function HistoryPanel({ shellType, onInsert, onClose }: Props) {
  const t = useT();
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [selected, setSelected] = useState(0);
  const [notice, setNotice] = useState("");
  const [confirmClear, setConfirmClear] = useState(false);
  // 「几分钟前」以打开面板那一刻为准。面板开着的这一会儿不需要它自己走动
  const [now] = useState(() => Date.now());
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const reload = () =>
    invoke<HistoryEntry[]>("history_list")
      .then((list) => {
        setEntries(list);
        setError("");
      })
      .catch((e) => setError(String(e)));

  useEffect(() => {
    reload();
    inputRef.current?.focus();
  }, []);

  const shown = useMemo(
    () => visibleEntries(entries, query, favoritesOnly),
    [entries, query, favoritesOnly],
  );
  // 列表一变，选中项可能越界
  const current = Math.min(selected, Math.max(0, shown.length - 1));

  useEffect(() => {
    listRef.current
      ?.querySelector<HTMLElement>(`[data-index="${current}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [current]);

  const insert = async (e: HistoryEntry | undefined) => {
    if (!e) return;
    if (await onInsert(e.command)) {
      onClose();
    } else {
      setNotice(t("history.copiedMultiline"));
    }
  };

  // 收藏和删除都按命令文本作用到所有同名条目：列表里一行代表一条命令，
  // 只改最近那一次的话，更早那次的收藏会让它「删不掉」或「取消不了」
  const sameCommand = (cmd: string) => entries.filter((x) => x.command === cmd);
  const toggleFavorite = async (e: HistoryEntry) => {
    const ids = e.favorite ? sameCommand(e.command).filter((x) => x.favorite) : [e];
    try {
      for (const x of ids) {
        await invoke("history_set_favorite", { id: x.id, favorite: !e.favorite });
      }
    } catch (err) {
      setError(String(err));
    }
    reload();
  };
  const remove = async (e: HistoryEntry) => {
    try {
      for (const x of sameCommand(e.command)) {
        await invoke("history_delete", { id: x.id });
      }
    } catch (err) {
      setError(String(err));
    }
    reload();
  };
  const clearAll = async () => {
    setConfirmClear(false);
    await invoke("history_clear").catch((err) => setError(String(err)));
    reload();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelected(Math.min(current + 1, shown.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelected(Math.max(current - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      insert(shown[current]);
    }
  };

  const unsupported = historyUnsupportedReason(shellType);
  const when = (at: number) => {
    const r = relativeTime(at, now);
    return t(r.key, r.n === undefined ? undefined : { n: String(r.n) });
  };

  return (
    <div className="git-overlay" onClick={onClose}>
      <div className="git-panel hist-panel" onClick={(e) => e.stopPropagation()} onKeyDown={onKeyDown}>
        <div className="git-head">
          <span className="git-title">{t("history.title")}</span>
          <button className="git-x" onClick={onClose} title={t("git.close")}>
            ×
          </button>
        </div>

        <div className="hist-bar">
          <input
            ref={inputRef}
            className="hist-search"
            value={query}
            placeholder={t("history.search")}
            onChange={(e) => {
              setQuery(e.target.value);
              setSelected(0);
            }}
          />
          <button
            className={"hist-filter" + (favoritesOnly ? " on" : "")}
            onClick={() => {
              setFavoritesOnly((v) => !v);
              setSelected(0);
            }}
            title={t("history.favoritesOnly")}
          >
            ★
          </button>
        </div>

        {unsupported && <div className="hist-note">{t(unsupported)}</div>}
        {notice && <div className="hist-note">{notice}</div>}
        {error && <div className="hist-note hist-error">{error}</div>}

        <div className="hist-list" ref={listRef}>
          {shown.length === 0 ? (
            <div className="git-empty">
              {entries.length === 0 ? t("history.empty") : t("history.noMatch")}
            </div>
          ) : (
            shown.map((e, i) => (
              <div
                key={e.id}
                data-index={i}
                className={"hist-row" + (i === current ? " selected" : "")}
                onMouseEnter={() => setSelected(i)}
                onClick={() => insert(e)}
                title={e.command}
              >
                <div className="hist-cmd">{e.command}</div>
                <div className="hist-meta">
                  {e.exitCode !== 0 && (
                    <span className="hist-exit" title={t("history.exitCode")}>
                      {e.exitCode}
                    </span>
                  )}
                  <span>{when(e.at)}</span>
                  <span className="hist-cwd">{e.cwd}</span>
                </div>
                <button
                  className={"hist-star" + (e.favorite ? " on" : "")}
                  title={e.favorite ? t("history.unfavorite") : t("history.favorite")}
                  onClick={(ev) => {
                    ev.stopPropagation();
                    toggleFavorite(e);
                  }}
                >
                  {e.favorite ? "★" : "☆"}
                </button>
                <button
                  className="hist-del"
                  title={t("history.delete")}
                  onClick={(ev) => {
                    ev.stopPropagation();
                    remove(e);
                  }}
                >
                  ×
                </button>
              </div>
            ))
          )}
        </div>

        <div className="hist-foot">
          <span>{t("history.hint")}</span>
          {confirmClear ? (
            <span className="hist-confirm">
              {t("history.clearConfirm")}
              <button onClick={clearAll}>{t("history.clearYes")}</button>
              <button onClick={() => setConfirmClear(false)}>{t("history.clearNo")}</button>
            </span>
          ) : (
            entries.length > 0 && (
              <button className="hist-clear" onClick={() => setConfirmClear(true)}>
                {t("history.clear")}
              </button>
            )
          )}
        </div>
      </div>
    </div>
  );
}
