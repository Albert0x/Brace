import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useT } from "../i18n";

interface FilePreview {
  kind: string; // text | image | binary | toolarge
  content: string;
  size: number;
}

function fmtSize(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + " MB";
  if (n >= 1000) return Math.round(n / 1000) + " KB";
  return n + " B";
}

// 侧边文件预览 + 轻量编辑：文本可改 Ctrl+S 保存，图片显示，其它兜底提示
export default function PreviewPanel({
  path,
  name,
  onClose,
}: {
  path: string;
  name: string;
  onClose: () => void;
}) {
  const t = useT();
  const [preview, setPreview] = useState<FilePreview | null>(null);
  const [draft, setDraft] = useState("");
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setPreview(null);
    setDirty(false);
    invoke<FilePreview>("read_file", { path })
      .then((p) => {
        setPreview(p);
        if (p.kind === "text") setDraft(p.content);
      })
      .catch(() => setPreview({ kind: "binary", content: "", size: 0 }));
  }, [path]);

  const save = () => {
    if (!dirty || saving) return;
    setSaving(true);
    invoke("write_file", { path, content: draft })
      .then(() => setDirty(false))
      .catch(() => {})
      .finally(() => setSaving(false));
  };

  const [confirmClose, setConfirmClose] = useState(false);
  const handleClose = () => {
    if (dirty) setConfirmClose(true);
    else onClose();
  };
  const saveAndClose = () => {
    invoke("write_file", { path, content: draft })
      .then(() => onClose())
      .catch(() => {});
  };

  return (
    <div className="preview-panel">
      <div className="preview-head">
        <span className="preview-name" title={path}>
          {name}
          {dirty && <span className="preview-dot" />}
        </span>
        {preview?.kind === "text" && (
          <button
            className="preview-save"
            disabled={!dirty || saving}
            onClick={save}
          >
            {t("preview.save")}
          </button>
        )}
        {preview && preview.size > 0 && (
          <span className="preview-size">{fmtSize(preview.size)}</span>
        )}
        <button className="preview-close" onClick={handleClose} title={t("preview.close")}>
          ×
        </button>
      </div>
      <div className="preview-body">
        {!preview && <div className="preview-hint">…</div>}
        {preview?.kind === "text" && (
          <textarea
            className="preview-editor"
            value={draft}
            spellCheck={false}
            onChange={(e) => {
              setDraft(e.target.value);
              setDirty(true);
            }}
            onKeyDown={(e) => {
              if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
                e.preventDefault();
                save();
              }
            }}
          />
        )}
        {preview?.kind === "image" && (
          <div className="preview-image-wrap">
            <img className="preview-img" src={preview.content} alt={name} />
          </div>
        )}
        {preview?.kind === "binary" && (
          <div className="preview-hint">{t("preview.binary")}</div>
        )}
        {preview?.kind === "toolarge" && (
          <div className="preview-hint">{t("preview.toolarge")}</div>
        )}
      </div>
      {confirmClose && (
        <div className="preview-confirm-overlay">
          <div className="preview-confirm">
            <div className="preview-confirm-msg">{t("preview.unsaved")}</div>
            <div className="preview-confirm-actions">
              <button onClick={() => setConfirmClose(false)}>
                {t("update.cancel")}
              </button>
              <button onClick={onClose}>{t("preview.discard")}</button>
              <button className="primary" onClick={saveAndClose}>
                {t("preview.saveClose")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
