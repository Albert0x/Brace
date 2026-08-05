import { useEffect, useRef, useState } from "react";
import { useT } from "../i18n";

export type EntryAction = "newFile" | "newFolder" | "rename" | "delete";

const TITLE: Record<EntryAction, string> = {
  newFile: "tree.newFile",
  newFolder: "tree.newFolder",
  rename: "tree.rename",
  delete: "tree.delete",
};

// 新建 / 重命名 / 删除用的小对话框。删除是确认型（没有输入框），其余是输入型。
export default function FileEntryDialog({
  action,
  target,
  busy,
  error,
  onCancel,
  onSubmit,
}: {
  action: EntryAction;
  target: string; // 重命名/删除的对象名，新建时是父目录名
  busy: boolean;
  error: string;
  onCancel: () => void;
  onSubmit: (name: string) => void;
}) {
  const t = useT();
  const isDelete = action === "delete";
  const [name, setName] = useState(action === "rename" ? target : "");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.focus();
    // 重命名时只选中主干、不选扩展名——跟资源管理器一个手感，
    // 大多数重命名只想改名字那部分
    const dot = target.lastIndexOf(".");
    if (action === "rename" && dot > 0) el.setSelectionRange(0, dot);
    else el.select();
  }, [action, target]);

  const submit = () => {
    if (busy) return;
    if (!isDelete && !name.trim()) return;
    onSubmit(name.trim());
  };

  return (
    <div className="entry-dialog-overlay" onClick={onCancel}>
      <div className="entry-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="entry-dialog-title">{t(TITLE[action])}</div>

        {isDelete ? (
          <div className="entry-dialog-msg">
            {t("tree.deleteConfirm", { name: target })}
            <div className="entry-dialog-sub">{t("tree.deleteToTrash")}</div>
          </div>
        ) : (
          <input
            ref={inputRef}
            className="entry-dialog-input"
            value={name}
            spellCheck={false}
            autoComplete="off"
            placeholder={t("tree.namePlaceholder")}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submit();
              else if (e.key === "Escape") onCancel();
            }}
          />
        )}

        {error && <div className="entry-dialog-error">{error}</div>}

        <div className="entry-dialog-actions">
          <button className="entry-dialog-btn" onClick={onCancel}>
            {t("update.cancel")}
          </button>
          <button
            className={"entry-dialog-btn " + (isDelete ? "danger" : "primary")}
            disabled={busy || (!isDelete && !name.trim())}
            onClick={submit}
          >
            {isDelete ? t("tree.delete") : t("tree.confirm")}
          </button>
        </div>
      </div>
    </div>
  );
}
