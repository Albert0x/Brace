import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { THEMES, LIGHT_THEME, applyTheme, type Theme } from "../themes";
import { usePersistedNumber, usePersistedString } from "./usePersisted";

// 主题、明暗模式、缩放、背景图——所有影响"长什么样"的状态。
// 主题只持久化 id，对象每次从 THEMES 查回来，免得表里改了颜色老用户还在用存下来的旧值
export function useAppearance() {
  const [themeId, setThemeId] = usePersistedString("ht-theme", THEMES[0].id);
  const theme = THEMES.find((x) => x.id === themeId) ?? THEMES[0];
  const setTheme = (t: Theme) => setThemeId(t.id);

  const [appearance, setAppearance] = usePersistedString("ht-appearance", "dark");
  const [uiZoom, setUiZoom] = usePersistedNumber("ht-zoom", 1);
  const [fontSize, setFontSize] = usePersistedNumber("ht-fontsize", 14);

  // 跟随系统明暗
  const [systemDark, setSystemDark] = useState(
    () => window.matchMedia("(prefers-color-scheme: dark)").matches,
  );
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const h = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener("change", h);
    return () => mq.removeEventListener("change", h);
  }, []);

  const effectiveTheme =
    appearance === "light"
      ? LIGHT_THEME
      : appearance === "system"
        ? systemDark
          ? theme
          : LIGHT_THEME
        : theme;
  useEffect(() => {
    applyTheme(effectiveTheme);
  }, [effectiveTheme]);

  // 缩放改了要让 xterm 重新 fit，否则行列数还是按旧尺寸算的
  useEffect(() => {
    (document.documentElement.style as any).zoom = String(uiZoom);
    requestAnimationFrame(() => window.dispatchEvent(new Event("resize")));
  }, [uiZoom]);

  // 背景图落盘到应用配置目录，不走 localStorage——那边 5MB 配额一超就静默失败，
  // 用户设完壁纸重启发现没了还不知道为什么
  const [bgImage, setBgImage] = useState("");
  const [overlay, setOverlay] = usePersistedNumber("ht-overlay", 0.5);
  useEffect(() => {
    // 老版本存在 localStorage 里的顺手搬到文件，别让升级的人丢背景
    const legacy = localStorage.getItem("ht-bg");
    if (legacy) {
      setBgImage(legacy);
      invoke("save_bg_image", { dataUrl: legacy })
        .then(() => localStorage.removeItem("ht-bg"))
        .catch(() => {});
      return;
    }
    invoke<string | null>("load_bg_image")
      .then((d) => d && setBgImage(d))
      .catch(() => {});
  }, []);
  const pickBg = (dataUrl: string) => {
    setBgImage(dataUrl);
    invoke("save_bg_image", { dataUrl }).catch(console.error);
  };

  return {
    theme,
    setTheme,
    effectiveTheme,
    appearance,
    setAppearance,
    uiZoom,
    setUiZoom,
    fontSize,
    setFontSize,
    bgImage,
    pickBg,
    overlay,
    setOverlay,
  };
}
