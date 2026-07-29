# Brace

[![English](https://img.shields.io/badge/Language-English-0969da)](./README.md)
[![简体中文](https://img.shields.io/badge/语言-简体中文-d73a49)](./README.zh-CN.md)

Brace 是一款基于 Tauri、React、TypeScript、Rust 和 xterm.js
构建的桌面终端应用。它在轻量级原生窗口中提供多终端标签、Shell
选择、跟随工作目录的文件树、终端搜索、主题和自定义背景。

> [!IMPORTANT]
> Windows 是当前已经实现的平台。项目可以在 macOS 上编译，但 macOS
> 的 Shell 检测、工作目录上报、快捷键和原生窗口行为仍未完成。

## 功能

- 使用原生伪终端实现多终端标签
- 在 Windows 上检测 PowerShell、命令提示符和 Git Bash
- 文件树跟随当前 PowerShell 工作目录
- 支持搜索、链接跳转、复制粘贴和字体大小设置
- 内置主题和自定义背景
- 支持 Tauri 的 Windows 与 macOS 桌面打包

## 安装

从 [发布页](https://github.com/Albert0x/Brace/releases/latest) 下载最新的
`Brace_x.y.z_x64-setup.exe` 运行即可。

> 首次启动时，Windows SmartScreen 可能弹出「Windows 已保护你的电脑」——
> 因为安装包尚未做代码签名。点击 **更多信息 → 仍要运行** 继续。
> 装好后 Brace 会自动更新。

## 技术栈

| 分层 | 技术 |
| --- | --- |
| 桌面运行时 | Tauri 2 |
| 前端 | React 19、TypeScript 5、Vite 7 |
| 终端 | xterm.js 6 |
| 原生后端 | Rust、`portable-pty` |
| 包管理器 | pnpm 9.15.9 |

## 环境要求

- Node.js 22，版本参考 `.nvmrc`
- Corepack 和 pnpm 9.15.9
- 最新稳定版 Rust 工具链
- [Tauri 官方文档](https://v2.tauri.app/zh-cn/start/prerequisites/)要求的平台依赖

启用项目指定的包管理器：

```bash
corepack enable
corepack prepare pnpm@9.15.9 --activate
```

不要使用 `pnpm install --force` 绕过锁文件版本不兼容。该操作可能重写锁文件并
引入与当前需求无关的依赖变更。

## 本地开发

```bash
pnpm install --frozen-lockfile
pnpm tauri dev
```

可以使用 `pnpm dev` 只运行前端，但 PTY、文件系统、窗口和链接打开能力必须在
Tauri 运行时中验证。

## 验证与构建

每次变更必须执行与修改范围对应的检查：

```bash
pnpm build
cargo check --locked --manifest-path src-tauri/Cargo.toml
pnpm tauri build
```

发布平台安装包前必须执行 `pnpm tauri build`。仅前端构建通过不能证明原生终端
功能正常。

## 项目结构

```text
src/
  components/          终端、文件树和设置 React 组件
  App.tsx              应用状态、标签、快捷键和布局
  themes.ts            终端与应用主题
src-tauri/
  src/lib.rs           PTY 会话、文件系统命令和 Shell 检测
  capabilities/        Tauri 权限声明
  tauri.conf.json      窗口、安全和打包配置
```

## 平台支持情况

| 能力 | Windows | macOS |
| --- | --- | --- |
| 构建 | 支持 | 可以编译 |
| Shell 检测 | PowerShell、CMD、Git Bash | 未实现 |
| 默认终端启动 | 支持 | 未实现 |
| 工作目录同步 | PowerShell | 未实现 |
| 原生快捷键 | 基于 Ctrl | Command 映射未实现 |
| 原生窗口样式 | Acrylic | 未实现 |

macOS 维护者不应复制 Windows 专用逻辑。Shell 选择、命令引用、快捷键、状态文本
和窗口行为必须通过明确的平台边界实现。

## 常见问题

### pnpm 提示锁文件不兼容

使用项目指定的 pnpm 版本：

```bash
corepack pnpm@9.15.9 install --frozen-lockfile
```

### Cargo 无法连接本地代理

检查 `HTTP_PROXY`、`HTTPS_PROXY` 和 `ALL_PROXY`。如果代理客户端未运行，残留的
`127.0.0.1:<port>` 配置会导致 Cargo 无法下载依赖。应修复本地环境，而不是修改
依赖元数据。

## 参与维护

开发前必须阅读 [CONTRIBUTING.md](./CONTRIBUTING.md)，其中规定了分支、提交、
Pull Request、跨平台、安全和验证要求。

## 当前限制

- 尚未配置自动化测试和持续集成。
- Content Security Policy 当前处于关闭状态。
- 尚未按不同 Shell 安全封装命令引用。
- 应用元数据仍包含部分 Tauri 初始模板内容。

这些限制属于待解决的工程问题，不能将其理解为相关行为已经安全或得到支持。
