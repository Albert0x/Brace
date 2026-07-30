import { useState, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { THEMES, type Theme } from "../themes";
import { useLang, LANGS, type Lang } from "../i18n";

const REPO_URL = "https://github.com/Albert0x/Brace";
const APP_VERSION = "0.1.3";

interface StatuslineStatus {
  configured: boolean;
  occupiedByOther: boolean;
  otherCommand: string;
  nodeAvailable: boolean;
}

interface Props {
  open: boolean;
  onClose: () => void;
  currentTheme: string;
  onSelectTheme: (t: Theme) => void;
  hasBg: boolean;
  overlay: number;
  onPickBg: (dataUrl: string) => void;
  onClearBg: () => void;
  onOverlay: (v: number) => void;
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
  const { lang, setLang, t } = useLang();
  const [tab, setTab] = useState<Tab>("general");
  const [checking, setChecking] = useState(false);
  const [updateStatus, setUpdateStatus] = useState<{
    type: "info" | "confirm";
    message: string;
    onConfirm?: () => void;
  } | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const [slStatus, setSlStatus] = useState<StatuslineStatus | null>(null);
  const [slBusy, setSlBusy] = useState(false);
  const [slError, setSlError] = useState("");
  useEffect(() => {
    if (props.open)
      invoke<StatuslineStatus>("statusline_status")
        .then(setSlStatus)
        .catch(() => {});
  }, [props.open]);
  const toggleStatusline = async (on: boolean) => {
    setSlBusy(true);
    setSlError("");
    try {
      await invoke("configure_statusline", { enable: on });
    } catch (e) {
      setSlError(String(e));
    }
    try {
      setSlStatus(await invoke<StatuslineStatus>("statusline_status"));
    } catch {
      /* ignore */
    }
    setSlBusy(false);
  };

  if (!props.open) return null;

  const onFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => props.onPickBg(reader.result as string);
    reader.readAsDataURL(file);
    e.target.value = "";
  };

  const TABS: { id: Tab; key: string; icon: string }[] = [
    { id: "general", key: "settings.general", icon: "⚙" },
    { id: "themes", key: "settings.themes", icon: "🎨" },
    { id: "about", key: "settings.about", icon: "ⓘ" },
  ];

  return (
    <div className="settings-overlay" onClick={props.onClose}>
      <div className="settings-panel" onClick={(e) => e.stopPropagation()}>
        {updateStatus && (
          <div
            className="update-modal-overlay"
            onClick={() => setUpdateStatus(null)}
          >
            <div className="update-modal" onClick={(e) => e.stopPropagation()}>
              <div className="update-modal-msg">{updateStatus.message}</div>
              <div className="update-modal-actions">
                {updateStatus.type === "confirm" && (
                  <button
                    className="update-modal-btn"
                    onClick={() => setUpdateStatus(null)}
                  >
                    {t("update.cancel")}
                  </button>
                )}
                <button
                  className="update-modal-btn primary"
                  onClick={() => {
                    const c = updateStatus.onConfirm;
                    setUpdateStatus(null);
                    c?.();
                  }}
                >
                  {updateStatus.type === "confirm" ? t("update.confirm") : t("update.ok")}
                </button>
              </div>
            </div>
          </div>
        )}

        <div className="settings-tabs">
          {TABS.map((tb) => (
            <button
              key={tb.id}
              className={"settings-tab" + (tab === tb.id ? " active" : "")}
              onClick={() => setTab(tb.id)}
            >
              <span className="settings-tab-icon">{tb.icon}</span>
              {t(tb.key)}
            </button>
          ))}
          <button className="settings-close" onClick={props.onClose} title={t("settings.close")}>
            ×
          </button>
        </div>

        <div className="settings-body">
          {/* ---------- General ---------- */}
          {tab === "general" && (
            <>
              <h2 className="settings-h2">{t("settings.general")}</h2>
              <p className="settings-sub">{t("general.sub")}</p>

              <div className="settings-section-title">{t("general.appearance")}</div>
              <div className="appearance-grid">
                {[
                  { id: "system", key: "appearance.system", icon: "🖥" },
                  { id: "light", key: "appearance.light", icon: "☀" },
                  { id: "dark", key: "appearance.dark", icon: "🌙" },
                ].map((a) => (
                  <div
                    key={a.id}
                    className={
                      "appearance-card" + (props.appearance === a.id ? " selected" : "")
                    }
                    onClick={() => props.onAppearance(a.id)}
                  >
                    <div className="appearance-icon">{a.icon}</div>
                    <div>{t(a.key)}</div>
                  </div>
                ))}
              </div>
              <p className="settings-hint">{t("general.themeHint")}</p>

              <div className="settings-section-title">{t("general.language")}</div>
              <Row title={t("general.language")} desc={t("general.languageDesc")}>
                <select
                  className="lang-select"
                  value={lang}
                  onChange={(e) => setLang(e.target.value as Lang)}
                >
                  {LANGS.map((l) => (
                    <option key={l.code} value={l.code}>
                      {l.label}
                    </option>
                  ))}
                </select>
              </Row>

              <div className="settings-section-title">{t("general.zoom")}</div>
              <Row title={t("general.uiZoom")} desc={`${Math.round(props.uiZoom * 100)}%`}>
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

              <div className="settings-section-title">{t("general.explorer")}</div>
              <Row title={t("general.showHidden")} desc={t("general.showHiddenDesc")}>
                <Toggle on={props.showHidden} onChange={props.onShowHidden} />
              </Row>
              <Row title={t("general.gitDeco")} desc={t("general.gitDecoDesc")}>
                <Toggle on={props.gitDeco} onChange={props.onGitDeco} />
              </Row>

              <div className="settings-section-title">{t("general.terminal")}</div>
              <Row title={t("general.webgl")} desc={t("general.webglDesc")}>
                <Toggle on={props.webgl} onChange={props.onWebgl} />
              </Row>
              <Row title={t("general.cursorBlink")} desc={t("general.cursorBlinkDesc")}>
                <Toggle on={props.cursorBlink} onChange={props.onCursorBlink} />
              </Row>

              <div className="settings-section-title">{t("usage.title")}</div>
              <Row title={t("usage.enable")} desc={t("usage.enableDesc")}>
                <Toggle
                  on={!!slStatus?.configured}
                  onChange={(v) => !slBusy && toggleStatusline(v)}
                />
              </Row>
              {slStatus && !slStatus.nodeAvailable && (
                <p className="settings-hint">{t("usage.noNode")}</p>
              )}
              {slStatus?.occupiedByOther && (
                <p className="settings-hint">
                  {t("usage.occupied", { cmd: slStatus.otherCommand })}
                </p>
              )}
              {slError && (
                <p className="settings-hint" style={{ color: "#e06c75" }}>
                  {slError}
                </p>
              )}
            </>
          )}

          {/* ---------- Themes ---------- */}
          {tab === "themes" && (
            <>
              <h2 className="settings-h2">{t("settings.themes")}</h2>
              <p className="settings-sub">{t("themes.sub")}</p>

              <div className="settings-section-title">{t("themes.theme")}</div>
              <div className="theme-grid">
                {THEMES.map((th) => (
                  <div
                    key={th.id}
                    className={
                      "theme-card" + (th.id === props.currentTheme ? " selected" : "")
                    }
                    onClick={() => props.onSelectTheme(th)}
                  >
                    <div className="theme-swatch" style={{ background: th.ui.bg }}>
                      <span style={{ background: th.ui.accent }} />
                      <span style={{ background: th.ui.fg }} />
                      <span style={{ background: th.ui.dim }} />
                    </div>
                    <div className="theme-meta">
                      <div className="theme-name">{th.name}</div>
                      <div className="theme-desc">{th.desc}</div>
                    </div>
                  </div>
                ))}
              </div>

              <div className="settings-section-title" style={{ marginTop: 20 }}>
                {t("themes.background")}
              </div>
              <div className="bg-controls">
                <button className="bg-btn" onClick={() => fileRef.current?.click()}>
                  {t("themes.pickImage")}
                </button>
                {props.hasBg && (
                  <button className="bg-btn bg-btn-clear" onClick={props.onClearBg}>
                    {t("themes.clear")}
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
                  <span>{t("themes.overlay")}</span>
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
              <h2 className="settings-h2">{t("settings.about")}</h2>

              <div className="about-card">
                <div className="about-logo">⌘</div>
                <div>
                  <div className="about-name">Brace</div>
                  <div className="about-tagline">{t("about.tagline")}</div>
                  <div className="about-ver">v{APP_VERSION}</div>
                </div>
              </div>

              <div className="about-rows">
                <div className="about-row">
                  <span>{t("about.build")}</span>
                  <span>Windows · x86_64 · v{APP_VERSION}</span>
                </div>
                <div className="about-row">
                  <span>{t("about.bundleId")}</span>
                  <span>com.brace.dev</span>
                </div>
                <div className="about-row">
                  <span>{t("about.license")}</span>
                  <span>MIT</span>
                </div>
                <div className="about-row">
                  <span>{t("about.source")}</span>
                  <span
                    className="about-link"
                    onClick={() => openUrl(REPO_URL).catch(console.error)}
                  >
                    Albert0x/Brace
                  </span>
                </div>
              </div>

              <div className="about-actions">
                <button
                  className="about-btn primary"
                  disabled={checking}
                  onClick={async () => {
                    if (checking) return;
                    setChecking(true);
                    try {
                      const update = await check();
                      if (update) {
                        setUpdateStatus({
                          type: "confirm",
                          message:
                            t("update.found", { v: update.version }) +
                            (update.body ? "\n\n" + update.body : ""),
                          onConfirm: async () => {
                            await update.downloadAndInstall();
                            await relaunch();
                          },
                        });
                      } else {
                        setUpdateStatus({
                          type: "info",
                          message: t("update.latest", { v: APP_VERSION }),
                        });
                      }
                    } catch (e) {
                      setUpdateStatus({
                        type: "info",
                        message: t("update.failed", { e: String(e) }),
                      });
                    } finally {
                      setChecking(false);
                    }
                  }}
                >
                  {checking ? t("about.checking") : t("about.checkUpdate")}
                </button>
                <button
                  className="about-btn"
                  onClick={() => openUrl(REPO_URL).catch(console.error)}
                >
                  {t("about.viewGithub")}
                </button>
                <button
                  className="about-btn"
                  onClick={() => openUrl(`${REPO_URL}/issues`).catch(console.error)}
                >
                  {t("about.reportIssue")}
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
