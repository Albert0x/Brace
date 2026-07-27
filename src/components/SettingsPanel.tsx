import { useRef } from "react";
import { THEMES, type Theme } from "../themes";

interface Props {
  open: boolean;
  current: string; // 当前主题 id
  onSelect: (theme: Theme) => void;
  onClose: () => void;
  // 背景图
  hasBg: boolean;
  overlay: number;
  onPickBg: (dataUrl: string) => void;
  onClearBg: () => void;
  onOverlay: (v: number) => void;
}

// 设置面板：主题 + 背景图，后续扩展字体/快捷键/代理等
export default function SettingsPanel({
  open,
  current,
  onSelect,
  onClose,
  hasBg,
  overlay,
  onPickBg,
  onClearBg,
  onOverlay,
}: Props) {
  const fileRef = useRef<HTMLInputElement>(null);

  if (!open) return null;

  const onFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => onPickBg(reader.result as string);
    reader.readAsDataURL(file);
    e.target.value = ""; // 允许重复选同一文件
  };

  return (
    <div className="settings-overlay" onClick={onClose}>
      <div className="settings-panel" onClick={(e) => e.stopPropagation()}>
        <div className="settings-head">
          <span>设置</span>
          <button className="settings-close" onClick={onClose} title="关闭">
            ×
          </button>
        </div>

        <div className="settings-section-title">主题</div>
        <div className="theme-grid">
          {THEMES.map((t) => (
            <div
              key={t.id}
              className={"theme-card" + (t.id === current ? " selected" : "")}
              onClick={() => onSelect(t)}
            >
              <div className="theme-swatch" style={{ background: t.ui.bg }}>
                <span style={{ background: t.ui.accent }} />
                <span style={{ background: t.ui.fg }} />
                <span style={{ background: t.ui.dim }} />
              </div>
              <div className="theme-meta">
                <div className="theme-name">{t.name}</div>
                <div className="theme-desc">{t.desc}</div>
              </div>
            </div>
          ))}
        </div>

        <div className="settings-section-title" style={{ marginTop: 20 }}>
          背景图
        </div>
        <div className="bg-controls">
          <button className="bg-btn" onClick={() => fileRef.current?.click()}>
            选择图片…
          </button>
          {hasBg && (
            <button className="bg-btn bg-btn-clear" onClick={onClearBg}>
              清除
            </button>
          )}
          <input
            ref={fileRef}
            type="file"
            accept="image/*"
            style={{ display: "none" }}
            onChange={onFile}
          />
        </div>
        {hasBg && (
          <div className="bg-slider">
            <span>遮罩浓度</span>
            <input
              type="range"
              min={0.2}
              max={0.95}
              step={0.05}
              value={overlay}
              onChange={(e) => onOverlay(Number(e.target.value))}
            />
            <span className="bg-slider-val">{Math.round(overlay * 100)}%</span>
          </div>
        )}
      </div>
    </div>
  );
}
