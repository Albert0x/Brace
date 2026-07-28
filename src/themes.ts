// 主题定义：UI 配色 + 终端配色。终端背景统一透明，透出 UI 底色（配合毛玻璃/背景图）。
export interface Theme {
  id: string;
  name: string;
  desc: string;
  ui: {
    base: string; // 不透明底色（hex），用于背景遮罩层
    bg: string; // 主背景（半透明，无背景图时的默认底）
    panel: string; // 顶栏/侧栏/状态栏叠加色
    fg: string; // 主文字
    dim: string; // 次要文字
    accent: string; // 强调色（标签圆点等）
    border: string; // 分隔线
  };
  terminal: {
    background: string;
    foreground: string;
    cursor: string;
  };
}

export const THEMES: Theme[] = [
  {
    id: "hyperdark",
    name: "HyperDark",
    desc: "默认近黑玻璃",
    ui: {
      base: "#0c0c0e",
      bg: "rgba(12, 12, 14, 0.92)",
      panel: "rgba(0, 0, 0, 0.22)",
      fg: "#c8c8c8",
      dim: "#888888",
      accent: "#4fd1c5",
      border: "rgba(255, 255, 255, 0.06)",
    },
    terminal: { background: "rgba(0,0,0,0)", foreground: "#d4d4d4", cursor: "#d4d4d4" },
  },
  {
    id: "tokyonight",
    name: "Tokyo Night",
    desc: "沉静偏蓝的暗色",
    ui: {
      base: "#1a1b26",
      bg: "rgba(26, 27, 38, 0.92)",
      panel: "rgba(0, 0, 0, 0.25)",
      fg: "#a9b1d6",
      dim: "#565f89",
      accent: "#7aa2f7",
      border: "rgba(122, 162, 247, 0.12)",
    },
    terminal: { background: "rgba(0,0,0,0)", foreground: "#a9b1d6", cursor: "#7aa2f7" },
  },
  {
    id: "dracula",
    name: "Dracula",
    desc: "经典高对比紫黑",
    ui: {
      base: "#282a36",
      bg: "rgba(40, 42, 54, 0.92)",
      panel: "rgba(0, 0, 0, 0.22)",
      fg: "#f8f8f2",
      dim: "#6272a4",
      accent: "#bd93f9",
      border: "rgba(189, 147, 249, 0.14)",
    },
    terminal: { background: "rgba(0,0,0,0)", foreground: "#f8f8f2", cursor: "#bd93f9" },
  },
  {
    id: "nord",
    name: "Nord",
    desc: "极地蓝灰冷色",
    ui: {
      base: "#2e3440",
      bg: "rgba(46, 52, 64, 0.92)",
      panel: "rgba(0, 0, 0, 0.2)",
      fg: "#d8dee9",
      dim: "#4c566a",
      accent: "#88c0d0",
      border: "rgba(136, 192, 208, 0.14)",
    },
    terminal: { background: "rgba(0,0,0,0)", foreground: "#d8dee9", cursor: "#88c0d0" },
  },
  {
    id: "gruvbox",
    name: "Gruvbox",
    desc: "复古暖土色调",
    ui: {
      base: "#282828",
      bg: "rgba(40, 40, 40, 0.92)",
      panel: "rgba(0, 0, 0, 0.22)",
      fg: "#ebdbb2",
      dim: "#928374",
      accent: "#fabd2f",
      border: "rgba(250, 189, 47, 0.14)",
    },
    terminal: { background: "rgba(0,0,0,0)", foreground: "#ebdbb2", cursor: "#fabd2f" },
  },
];

// 亮色主题：外观选 Light、或 System 处于浅色时使用
export const LIGHT_THEME: Theme = {
  id: "light",
  name: "Light",
  desc: "亮色",
  ui: {
    base: "#f4f4f6",
    bg: "rgba(244, 244, 246, 0.92)",
    panel: "rgba(0, 0, 0, 0.05)",
    fg: "#2a2a2e",
    dim: "#8a8a90",
    accent: "#0a84ff",
    border: "rgba(0, 0, 0, 0.12)",
  },
  terminal: { background: "rgba(0,0,0,0)", foreground: "#2a2a2e", cursor: "#0a84ff" },
};

// 把主题的 UI 配色写入 CSS 变量
export function applyTheme(theme: Theme) {
  const r = document.documentElement.style;
  r.setProperty("--ht-base", theme.ui.base);
  r.setProperty("--ht-panel", theme.ui.panel);
  // 悬停/激活叠加色：亮色主题用深色叠加，暗色主题用浅色叠加（保证浅底上也可见）
  const light = theme.id === "light";
  r.setProperty("--ht-hover", light ? "rgba(0,0,0,0.06)" : "rgba(255,255,255,0.08)");
  r.setProperty("--ht-active", light ? "rgba(0,0,0,0.1)" : "rgba(255,255,255,0.12)");
  r.setProperty("--ht-fg", theme.ui.fg);
  r.setProperty("--ht-dim", theme.ui.dim);
  r.setProperty("--ht-accent", theme.ui.accent);
  r.setProperty("--ht-border", theme.ui.border);
}
