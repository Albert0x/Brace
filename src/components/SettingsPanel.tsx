import { useState, useRef } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { THEMES, type Theme } from "../themes";

const REPO_URL = "https://github.com/Albert0x/HyperTerminal";
const APP_VERSION = "0.1.0";

interface Props {
  open: boolean;
  onClose: () => void;
  // 主题
  currentTheme: string;
  onSelectTheme: (t: Theme) => void;
  // 背景
  hasBg: boolean;
  overlay: number;
  onPickBg: (dataUrl: string) => void;
  onClearBg: () => void;
  onOverlay: (v: number) => void;
  // General
  appearance: string;
  onAppearance: (a: string) => void;
  uiZoom: number;
  onUiZoom: (z: number) => void;
  showHidden: boolean;
  onShowHidden: (v: boolean) => void;
  gitDeco: boolean;
  onGitDeco: (v: boolean) => void;
  webgl: boolean;
  onWebgl: (v: boolean) => void;
  cursorBlink: boolean;
  onCursorBlink: (v: boolean) => void;
}

// 开关
function Toggle({ on, onChange }: { on: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      className={"toggle" + (on ? " on" : "")}
      onClick={() => onChange(!on)}
      aria-pressed={on}
    >
      <span className="toggle-knob" />
    </button>
  );
}

// 一行设置项（标题 + 描述 + 右侧控件）
function Row({
  title,
  desc,
  children,
}: {
  title: string;
  desc?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="set-row">
      <div className="set-row-text">
        <div className="set-row-title">{title}</div>
        {desc && <div className="set-row-desc">{desc}</div>}
      </div>
      <div className="set-row-ctrl">{children}</div>
    </div>
  );
}

type Tab = "general" | "themes" | "about";

export default function SettingsPanel(props: Props) {
  const [tab, setTab] = useState<Tab>("general");
  const fileRef = useRef<HTMLInputElement>(null);

  if (!props.open) return null;

  const onFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => props.onPickBg(reader.result as string);
    reader.readAsDataURL(file);
    e.target.value = "";
  };

  const TABS: { id: Tab; label: string; icon: string }[] = [
    { id: "general", label: "General", icon: "⚙" },
    { id: "themes", label: "Themes", icon: "🎨" },
    { id: "about", label: "About", icon: "ⓘ" },
  ];

  return (
    <div className="settings-overlay" onClick={props.onClose}>
      <div className="settings-panel" onClick={(e) => e.stopPropagation()}>
        {/* 顶部 tab 栏 */}
        <div className="settings-tabs">
          {TABS.map((t) => (
            <button
              key={t.id}
              className={"settings-tab" + (tab === t.id ? " active" : "")}
              onClick={() => setTab(t.id)}
            >
              <span className="settings-tab-icon">{t.icon}</span>
              {t.label}
            </button>
          ))}
          <button className="settings-close" onClick={props.onClose} title="关闭">
            ×
          </button>
        </div>

        <div className="settings-body">
          {/* ---------- General ---------- */}
          {tab === "general" && (
            <>
              <h2 className="settings-h2">General</h2>
              <p className="settings-sub">模式、终端与启动。</p>

              <div className="settings-section-title">外观</div>
              <div className="appearance-grid">
                {["System", "Light", "Dark"].map((a) => (
                  <div
                    key={a}
                    className={
                      "appearance-card" +
                      (props.appearance === a.toLowerCase() ? " selected" : "")
                    }
                    onClick={() => props.onAppearance(a.toLowerCase())}
                  >
                    <div className="appearance-icon">
                      {a === "System" ? "🖥" : a === "Light" ? "☀" : "🌙"}
                    </div>
                    <div>{a}</div>
                  </div>
                ))}
              </div>
              <p className="settings-hint">
                主题、背景与配色请到 <b>Themes</b> 页。
              </p>

              <div className="settings-section-title">缩放</div>
              <Row title="UI 缩放" desc={`${Math.round(props.uiZoom * 100)}%`}>
                <input
                  type="range"
                  min={0.7}
                  max={1.5}
                  step={0.05}
                  value={props.uiZoom}
                  onChange={(e) => props.onUiZoom(Number(e.target.value))}
                  className="set-range"
                />
              </Row>

              <div className="settings-section-title">资源管理器</div>
              <Row title="显示隐藏文件" desc="包含 .env / .gitignore 等以点开头的文件">
                <Toggle on={props.showHidden} onChange={props.onShowHidden} />
              </Row>
              <Row title="Git 装饰" desc="标记改动、淡化被忽略的文件（待实现）">
                <Toggle on={props.gitDeco} onChange={props.onGitDeco} />
              </Row>

              <div className="settings-section-title">终端</div>
              <Row title="WebGL 渲染" desc="GPU 加速；文字花屏时可关（切换后新终端生效）">
                <Toggle on={props.webgl} onChange={props.onWebgl} />
              </Row>
              <Row title="光标闪烁" desc="终端光标是否闪烁">
                <Toggle on={props.cursorBlink} onChange={props.onCursorBlink} />
              </Row>
            </>
          )}

          {/* ---------- Themes ---------- */}
          {tab === "themes" && (
            <>
              <h2 className="settings-h2">Themes</h2>
              <p className="settings-sub">主题、背景图与自定义。</p>

              <div className="settings-section-title">主题</div>
              <div className="theme-grid">
                {THEMES.map((t) => (
                  <div
                    key={t.id}
                    className={
                      "theme-card" + (t.id === props.currentTheme ? " selected" : "")
                    }
                    onClick={() => props.onSelectTheme(t)}
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
                {props.hasBg && (
                  <button className="bg-btn bg-btn-clear" onClick={props.onClearBg}>
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
              {props.hasBg && (
                <div className="bg-slider">
                  <span>遮罩浓度</span>
                  <input
                    type="range"
                    min={0.2}
                    max={0.95}
                    step={0.05}
                    value={props.overlay}
                    onChange={(e) => props.onOverlay(Number(e.target.value))}
                  />
                  <span className="bg-slider-val">
                    {Math.round(props.overlay * 100)}%
                  </span>
                </div>
              )}
            </>
          )}

          {/* ---------- About ---------- */}
          {tab === "about" && (
            <>
              <h2 className="settings-h2">About</h2>

              <div className="about-card">
                <div className="about-logo">⌘</div>
                <div>
                  <div className="about-name">HyperTerminal</div>
                  <div className="about-tagline">
                    Tauri + React 打造的现代终端
                  </div>
                  <div className="about-ver">v{APP_VERSION}</div>
                </div>
              </div>

              <div className="about-rows">
                <div className="about-row">
                  <span>Build</span>
                  <span>Windows · x86_64 · v{APP_VERSION}</span>
                </div>
                <div className="about-row">
                  <span>Bundle ID</span>
                  <span>com.hyperterminal.dev</span>
                </div>
                <div className="about-row">
                  <span>License</span>
                  <span>MIT</span>
                </div>
                <div className="about-row">
                  <span>Source</span>
                  <span
                    className="about-link"
                    onClick={() => openUrl(REPO_URL).catch(console.error)}
                  >
                    Albert0x/HyperTerminal
                  </span>
                </div>
              </div>

              <div className="about-actions">
                <button
                  className="about-btn primary"
                  onClick={async () => {
                    try {
                      const update = await check();
                      if (update) {
                        if (
                          confirm(
                            `发现新版本 v${update.version}\n${update.body ?? ""}\n\n现在下载并安装？`,
                          )
                        ) {
                          await update.downloadAndInstall();
                          await relaunch();
                        }
                      } else {
                        alert(`当前已是最新版本 v${APP_VERSION}`);
                      }
                    } catch (e) {
                      alert(`检查更新失败：${e}`);
                    }
                  }}
                >
                  检查更新
                </button>
                <button
                  className="about-btn"
                  onClick={() => openUrl(REPO_URL).catch(console.error)}
                >
                  View on GitHub
                </button>
                <button
                  className="about-btn"
                  onClick={() =>
                    openUrl(`${REPO_URL}/issues`).catch(console.error)
                  }
                >
                  Report an issue
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
