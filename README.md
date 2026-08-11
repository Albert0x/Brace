# Brace

[![English](https://img.shields.io/badge/Language-English-0969da)](./README.md)
[![简体中文](https://img.shields.io/badge/语言-简体中文-d73a49)](./README.zh-CN.md)

Brace is a desktop terminal application built with Tauri, React,
TypeScript, Rust, and xterm.js. It provides multiple terminal tabs, shell
selection, a working-directory-aware file tree, terminal search, themes, and
custom backgrounds in a lightweight native window.

> [!IMPORTANT]
> **Brace is a Windows application.** Shell discovery, working-directory
> reporting, secret storage, and window chrome are all built against Windows
> APIs. macOS and Linux are not supported and not currently planned — the code
> may still compile there, but treat that as an accident, not a promise.

## Features

- Multiple terminal tabs backed by native pseudo terminals
- PowerShell, Command Prompt, and Git Bash discovery on Windows
- File tree synchronized with the active PowerShell working directory, with
  auto-refresh on filesystem changes
- Environment variable profiles for switching API endpoints and proxies per
  terminal, with secrets encrypted at rest via Windows DPAPI
- Git decorations in the tree, plus a commit panel with per-file selection and diffs
- Search across terminal output with match highlighting and a hit counter
- Clickable links, copy and paste, configurable font size and scrollback
- Resizable file tree pane
- Session restore for the previous tab group (shell + directory per tab)
- Built-in themes and optional custom backgrounds
- Tauri desktop packaging for Windows

## Installation

Download the latest `Brace_x.y.z_x64-setup.exe` from the
[releases page](https://github.com/Albert0x/Brace/releases/latest) and run it.

> On first launch, Windows SmartScreen may show "Windows protected your PC"
> because the installer isn't code-signed yet. Click **More info → Run anyway**
> to continue. Once installed, Brace updates itself automatically.

See the **[full user guide](docs/USAGE.md)** for tabs & shells, the two-way file
tree, AI usage display, keyboard shortcuts, themes, and settings.

## Technology

| Layer | Technology |
| --- | --- |
| Desktop runtime | Tauri 2 |
| Frontend | React 19, TypeScript 5, Vite 7 |
| Terminal | xterm.js 6 |
| Native backend | Rust, `portable-pty` |
| Package manager | pnpm 9.15.9 |

## Prerequisites

- Node.js 22 (see `.nvmrc`)
- Corepack with pnpm 9.15.9
- A current stable Rust toolchain
- Platform prerequisites from the
  [Tauri documentation](https://v2.tauri.app/start/prerequisites/)

Enable the repository package manager:

```bash
corepack enable
corepack prepare pnpm@9.15.9 --activate
```

Do not use `pnpm install --force` to bypass a lockfile version mismatch. It can
rewrite the lockfile and introduce unrelated dependency changes.

## Development

```bash
pnpm install --frozen-lockfile
pnpm desktop:dev
# macOS (explicitly loads the native title-bar configuration)
pnpm desktop:dev:macos
```

Frontend-only development is available with `pnpm dev`, but PTY, filesystem,
window, and opener APIs require the Tauri runtime.

## Validation and Build

Every change must pass the checks relevant to the modified code. CI runs all of
these on Windows for every push and pull request:

```bash
pnpm lint
pnpm test
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib
```

CI does **not** build a platform package. Do that before publishing one — a
frontend build alone does not prove that native terminal behavior works:

```bash
pnpm desktop:build          # Windows
pnpm desktop:build:macos    # macOS
```

Windows releases run it through
[`.github/workflows/release.yml`](.github/workflows/release.yml), triggered by a
tag. macOS packages are built locally for now — the release workflow does not
cover them.

## Project Structure

```text
src/
  components/          React terminal, file tree, and settings components
  App.tsx              Application state, tabs, shortcuts, and layout
  themes.ts            Terminal and application themes
src-tauri/
  src/lib.rs           PTY sessions, filesystem commands, and shell discovery
  capabilities/        Tauri permission declarations
  tauri.conf.json      Window, security, and packaging configuration
```

## Platform Status

Windows only. What "supported" actually covers:

| Capability | Status |
| --- | --- |
| Windows version | 10 and 11 (acrylic window chrome is 11-only) |
| Shell discovery | PowerShell 5.1, PowerShell 7, CMD, Git Bash |
| Working-directory sync | All detected shells, via OSC 9;9 prompt injection |
| Secret storage | DPAPI, scoped to the current Windows user account |
| Native shortcuts | Ctrl-based |

macOS and Linux are out of scope. Platform-specific code sits behind
`#[cfg(windows)]` boundaries so a future port would have seams to work with,
but no port is planned.

## Troubleshooting

### pnpm reports an incompatible lockfile

Use the declared pnpm version:

```bash
corepack pnpm@9.15.9 install --frozen-lockfile
```

### Cargo cannot connect to a localhost proxy

Inspect `HTTP_PROXY`, `HTTPS_PROXY`, and `ALL_PROXY`. A stale proxy such as
`127.0.0.1:<port>` prevents Cargo from downloading crates when no proxy client
is listening. Fix the environment rather than editing dependency metadata.

## Contributing

Read [CONTRIBUTING.md](./CONTRIBUTING.md) before starting work. It defines the
branch, commit, pull request, cross-platform, security, and validation rules for
this repository.

## Current Limitations

- `cmd.exe` expands `%VAR%` inside double-quoted arguments, and no amount of
  string-level quoting prevents it. Paths containing `%` can therefore behave
  unexpectedly in CMD tabs. PowerShell and Git Bash tabs are unaffected.
- Environment profiles only apply to **newly created** terminals. A running
  process cannot have its environment changed — that is an OS rule, not an
  oversight.
- Images cannot be displayed inline. Terminal image protocols (sixel, iTerm2)
  are not implemented; Brace only tells you when the clipboard holds one.
- Auto-update requires the release to carry a valid `latest.json`. See
  [docs/RELEASING.md](docs/RELEASING.md) — a broken update channel fails
  silently.

These are known and accepted, not evidence that the surrounding behavior is
unsafe. Engineering work in flight is tracked in
[docs/ROADMAP.md](docs/ROADMAP.md).
