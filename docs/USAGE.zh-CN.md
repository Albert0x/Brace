# Brace 使用文档

[English](./USAGE.md) · 简体中文

Brace 是一款基于 Tauri + Rust 的轻量 Windows 终端,最大特色是**在状态栏实时显示 Claude / Codex 的用量**。本文覆盖日常使用的全部功能。

---

## 目录

- [安装](#安装)
- [界面概览](#界面概览)
- [标签与 Shell](#标签与-shell)
- [文件树联动](#文件树联动)
- [AI 用量显示（特色）](#ai-用量显示特色)
- [快捷键](#快捷键)
- [主题与背景](#主题与背景)
- [设置](#设置)
- [自动更新](#自动更新)

---

## 安装

1. 从 [Releases](https://github.com/Albert0x/Brace/releases/latest) 下载 `Brace_x.y.z_x64-setup.exe`,双击运行。
2. **首次安装** Windows SmartScreen 会弹「Windows 已保护你的电脑」——因为安装包尚未做代码签名。点 **更多信息 → 仍要运行** 即可。
3. 装好后 Brace 会自动检查并推送更新,无需再手动下载。

---

## 界面概览

Brace 采用三栏 + 状态栏布局:

- **顶栏**:标签页、新建标签 `＋`(旁边 `▾` 选 Shell)、搜索框、设置 `⚙`、窗口控制。
- **左侧**:文件树(跟随当前终端目录)。
- **中间**:终端。
- **底部状态栏**:左边是 **AI 用量**(跑 claude/codex 时),右边是字体大小、终端数、主题、编码、系统。

---

## 标签与 Shell

- **新建标签**:点 `＋` 或按 `Ctrl+T`。新标签**自动继承当前标签的目录**。
- **选择 Shell**:点 `＋` 旁的 `▾`,可选 PowerShell、PowerShell 7、命令提示符(CMD)、Git Bash。Brace 会自动检测系统里装了哪些(Git Bash 从 `git.exe` 反推路径)。
- **关闭标签**:点标签上的 `×` 或按 `Ctrl+W`。
- **切换标签**:`Ctrl+Tab` / `Ctrl+Shift+Tab`。
- 标签标题会智能显示当前目录名。

---

## 文件树联动

文件树和终端**双向同步**:

- 在终端 `cd` 到某目录,左侧文件树自动跟着切换根目录。
- 在文件树里**双击文件夹**,终端自动 `cd` 过去。
- 点右上角 `⟳` 刷新。
- 想显示 `.env` / `.gitignore` 等隐藏文件:设置 → 常规 → 打开「显示隐藏文件」。

> 原理:Brace 给每个 Shell 注入了 prompt,用 OSC 9;9 转义序列上报当前目录,PowerShell / CMD / Git Bash 各有对应写法。

---

## AI 用量显示（特色）

这是 Brace 独有的功能——**在状态栏实时显示 Claude / Codex 的用量**,只在当前标签**真的在跑** claude 或 codex 时才显示(进程检测)。

### Claude

Claude 的官方用量(context / 5小时 / 周额度 + 重置时间)走 Claude Code 的 **statusLine** 通道,需要先启用一次:

1. 设置 → **常规** → 拉到底「**Agent Usage**」→ 打开「在状态栏显示 Claude 用量」开关。
   - 这会往你的 `~/.claude/settings.json` 挂一个采集脚本(如果你已经配过别的 statusLine,Brace 不会覆盖,会提示)。
2. **启动 / 重启 claude**(statusLine 是 claude 启动时读配置的,已经开着的要 `/exit` 重开)。
3. 发一条消息。官方额度数据**要 API 响应后才有**,发过消息状态栏就会显示。

状态栏读法:`[模型]  Context 61%  5h 35% 4h7m  7d 48% 35h`
- **Context**:当前会话上下文窗口占用。
- **5h / 7d**:5小时窗口、周窗口的额度用量 %,后面是重置倒计时。

### Codex

Codex **无需任何配置**——直接跑 `codex`,状态栏自动显示 `Codex  Context x%  xxK tok`(上下文占用 + 会话累计 token)。

> Codex 只有 context 和 token,**没有** 5小时/周额度——因为 codex 本地会话文件里的 rate_limits 字段通常为空,数据源就没有。

### 给 Claude 粘贴图片

在 Brace(以及任何终端)里给 claude 粘图片,用 **`Alt+V`**,**不是 `Ctrl+V`**。因为 `Ctrl+V` 被终端占用当「粘贴文本」了,图片传不到 claude;`Alt+V` 是 Claude Code 在 Windows 上专门的图片粘贴键,识别后会插入 `[Image #1]` 占位。

---

## 快捷键

| 快捷键 | 作用 |
| --- | --- |
| `Ctrl+T` | 新建标签 |
| `Ctrl+W` | 关闭当前标签 |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | 下一个 / 上一个标签 |
| `Ctrl+F` | 聚焦搜索框 |
| `Ctrl+Shift+C` | 复制选中内容 |
| `Ctrl+Shift+V` | 粘贴(带 bracketed paste,claude/vim 能区分粘贴与手打) |
| `Ctrl+=` / `Ctrl+-` | 放大 / 缩小字体 |
| `Ctrl+0` | 字体恢复默认(14px) |
| `Alt+V` | (在 claude 里)粘贴剪贴板图片 |
| 右键 | 复制 / 粘贴 / 全选 / 清屏菜单 |

---

## 主题与背景

设置 → **主题**:

- **五款内置主题**:HyperDark(默认近黑玻璃)、Tokyo Night、Dracula、Nord、Gruvbox,点一下即时切换。
- **背景图**:点「选择图片…」设任意图片为毛玻璃背景,拖动「遮罩浓度」滑块调整明暗;点「清除」还原。
- **外观模式**:设置 → 常规 → 外观,可选跟随系统 / 浅色 / 深色。

---

## 设置

设置面板(点 `⚙`)分三个标签:

- **常规**:外观模式、界面语言(中/英)、UI 缩放、显示隐藏文件、Git 装饰、WebGL 渲染、光标闪烁、**Agent Usage**(Claude 用量开关)。
- **主题**:主题选择、背景图。
- **关于**:版本信息、检查更新、GitHub / 反馈链接。

---

## 自动更新

Brace 内置签名版自动更新:

- 设置 → 关于 → **检查更新**。有新版会提示,确认后自动下载并重启。
- 从 v0.1.2(改名 Brace 后的首版)开始,更新链路正常;更早的 HyperTerminal 因为 bundle id 变了,需要手动装一次 Brace。

---

有问题或建议,欢迎到 [GitHub Issues](https://github.com/Albert0x/Brace/issues) 反馈。
