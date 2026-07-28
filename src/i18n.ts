import { createContext, useContext } from "react";

export type Lang = "en" | "zh";

// 全部界面文案。key 扁平命名；{x} 为运行时替换的占位。
export const MESSAGES: Record<Lang, Record<string, string>> = {
  en: {
    // 顶栏 / 状态栏 / 主区
    "search.placeholder": "Search (Ctrl+F)",
    "tab.new": "New tab (Ctrl+T)",
    "tab.close": "Close tab (Ctrl+W)",
    "tab.selectShell": "Select shell",
    "toolbar.settings": "Settings",
    "win.minimize": "Minimize",
    "win.maximize": "Maximize",
    "win.close": "Close",
    "main.empty": "No terminal. Click ＋ or press Ctrl+T to open one.",
    "status.terminals": "{n} terminal(s)",
    // 侧栏 / 文件树
    "sidebar.refresh": "Refresh",
    "tree.enterDir": "Double-click to enter: {path}",
    // 终端右键菜单
    "ctx.copy": "Copy",
    "ctx.paste": "Paste",
    "ctx.selectAll": "Select all",
    "ctx.clear": "Clear",
    // 设置面板 tab
    "settings.general": "General",
    "settings.themes": "Themes",
    "settings.about": "About",
    "settings.close": "Close",
    // General
    "general.sub": "Mode, terminal, and startup.",
    "general.appearance": "Appearance",
    "appearance.system": "System",
    "appearance.light": "Light",
    "appearance.dark": "Dark",
    "general.themeHint": "For theme, background and colors, see the Themes tab.",
    "general.zoom": "Zoom",
    "general.uiZoom": "UI zoom",
    "general.explorer": "Explorer",
    "general.showHidden": "Show hidden files",
    "general.showHiddenDesc": "Include dot-prefixed files (.env, .gitignore).",
    "general.gitDeco": "Git decorations",
    "general.gitDecoDesc": "Mark changed files and dim ignored ones (todo).",
    "general.terminal": "Terminal",
    "general.webgl": "WebGL renderer",
    "general.webglDesc": "GPU acceleration; turn off if text corrupts (applies to new terminals).",
    "general.cursorBlink": "Cursor blinking",
    "general.cursorBlinkDesc": "Whether the terminal cursor blinks.",
    "general.language": "Language",
    "general.languageDesc": "Interface language.",
    // Themes
    "themes.sub": "Theme, background and customization.",
    "themes.theme": "Theme",
    "themes.background": "Background image",
    "themes.pickImage": "Choose image…",
    "themes.clear": "Clear",
    "themes.overlay": "Overlay opacity",
    // About
    "about.tagline": "A modern terminal built with Tauri + React",
    "about.build": "Build",
    "about.bundleId": "Bundle ID",
    "about.license": "License",
    "about.source": "Source",
    "about.checkUpdate": "Check for updates",
    "about.checking": "Checking…",
    "about.viewGithub": "View on GitHub",
    "about.reportIssue": "Report an issue",
    // 更新弹窗
    "update.latest": "You are on the latest version v{v}.",
    "update.found": "New version v{v} available.",
    "update.confirm": "Update",
    "update.cancel": "Cancel",
    "update.ok": "OK",
    "update.failed": "Update check failed: {e}",
  },
  zh: {
    "search.placeholder": "搜索 (Ctrl+F)",
    "tab.new": "新建标签 (Ctrl+T)",
    "tab.close": "关闭标签 (Ctrl+W)",
    "tab.selectShell": "选择 Shell",
    "toolbar.settings": "设置",
    "win.minimize": "最小化",
    "win.maximize": "最大化",
    "win.close": "关闭",
    "main.empty": "没有终端，点右上角 ＋ 或按 Ctrl+T 新建。",
    "status.terminals": "{n} 个终端",
    "sidebar.refresh": "刷新",
    "tree.enterDir": "双击进入：{path}",
    "ctx.copy": "复制",
    "ctx.paste": "粘贴",
    "ctx.selectAll": "全选",
    "ctx.clear": "清屏",
    "settings.general": "常规",
    "settings.themes": "主题",
    "settings.about": "关于",
    "settings.close": "关闭",
    "general.sub": "模式、终端与启动。",
    "general.appearance": "外观",
    "appearance.system": "跟随系统",
    "appearance.light": "浅色",
    "appearance.dark": "深色",
    "general.themeHint": "主题、背景与配色请到「主题」页。",
    "general.zoom": "缩放",
    "general.uiZoom": "UI 缩放",
    "general.explorer": "资源管理器",
    "general.showHidden": "显示隐藏文件",
    "general.showHiddenDesc": "包含 .env / .gitignore 等以点开头的文件。",
    "general.gitDeco": "Git 装饰",
    "general.gitDecoDesc": "标记改动、淡化被忽略的文件（待实现）。",
    "general.terminal": "终端",
    "general.webgl": "WebGL 渲染",
    "general.webglDesc": "GPU 加速；文字花屏时可关（对新终端生效）。",
    "general.cursorBlink": "光标闪烁",
    "general.cursorBlinkDesc": "终端光标是否闪烁。",
    "general.language": "语言",
    "general.languageDesc": "界面语言。",
    "themes.sub": "主题、背景图与自定义。",
    "themes.theme": "主题",
    "themes.background": "背景图",
    "themes.pickImage": "选择图片…",
    "themes.clear": "清除",
    "themes.overlay": "遮罩浓度",
    "about.tagline": "Tauri + React 打造的现代终端",
    "about.build": "构建",
    "about.bundleId": "Bundle ID",
    "about.license": "许可证",
    "about.source": "源码",
    "about.checkUpdate": "检查更新",
    "about.checking": "检查中…",
    "about.viewGithub": "在 GitHub 查看",
    "about.reportIssue": "反馈问题",
    "update.latest": "当前已是最新版本 v{v}。",
    "update.found": "发现新版本 v{v}。",
    "update.confirm": "更新",
    "update.cancel": "取消",
    "update.ok": "确定",
    "update.failed": "检查更新失败：{e}",
  },
};

// 生成翻译函数，支持 {x} 占位替换
export function createT(lang: Lang) {
  return (key: string, params?: Record<string, string | number>) => {
    let s = MESSAGES[lang][key] ?? MESSAGES.en[key] ?? key;
    if (params) {
      for (const k in params) s = s.replace(`{${k}}`, String(params[k]));
    }
    return s;
  };
}

export type TFn = ReturnType<typeof createT>;

export const LangContext = createContext<{
  lang: Lang;
  setLang: (l: Lang) => void;
  t: TFn;
}>({ lang: "en", setLang: () => {}, t: createT("en") });

// 可选语言列表（加新语言只需在此追加 + 补 MESSAGES）
export const LANGS: { code: Lang; label: string }[] = [
  { code: "en", label: "English" },
  { code: "zh", label: "中文" },
];

export const useLang = () => useContext(LangContext);
export const useT = () => useContext(LangContext).t;
