import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useT } from "../i18n";

interface FilePreview {
  kind: string; // text | image | binary | toolarge
  content: string;
  size: number;
  encoding: string; // text 时的原始编码，保存时原样写回
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
  const [error, setError] = useState("");

  useEffect(() => {
    setPreview(null);
    setDirty(false);
    setError("");
    invoke<FilePreview>("read_file", { path })
      .then((p) => {
        setPreview(p);
        if (p.kind === "text") setDraft(p.content);
      })
      .catch(() =>
        setPreview({ kind: "binary", content: "", size: 0, encoding: "" }),
      );
  }, [path]);

  // 按读取时识别的编码写回，不把 GBK/UTF-16 文件静默转成 UTF-8。
  // 保存失败必须让用户看见——原来这里是空 catch，磁盘满/只读/编码放不下都悄无声息
  const doSave = () => {
    setSaving(true);
    setError("");
    return invoke("write_file", {
      path,
      content: draft,
      encoding: preview?.encoding ?? "",
    })
      .then(() => {
        setDirty(false);
        return true;
      })
      .catch((e) => {
        setError(String(e));
        return false;
      })
      .finally(() => setSaving(false));
  };

  const save = () => {
    if (!dirty || saving) return;
    doSave();
  };

  const [confirmClose, setConfirmClose] = useState(false);
  const handleClose = () => {
    if (dirty) setConfirmClose(true);
    else onClose();
  };
  const saveAndClose = () => {
    doSave().then((ok) => {
      if (ok) onClose();
      else setConfirmClose(false); // 保存失败就别关，让错误留在面板上
    });
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
        {/* UTF-8 是默认，不占地方；GBK / UTF-16 这类才提示，让用户知道会按原编码存回 */}
        {preview?.kind === "text" && preview.encoding &&
          preview.encoding !== "UTF-8" && (
            <span className="preview-size">{preview.encoding}</span>
          )}
        {preview && preview.size > 0 && (
          <span className="preview-size">{fmtSize(preview.size)}</span>
        )}
        <button className="preview-close" onClick={handleClose} title={t("preview.close")}>
          ×
        </button>
      </div>
      {error && <div className="preview-error">{error}</div>}
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
