# Brace

[![English](https://img.shields.io/badge/Language-English-0969da)](./README.md)
[![简体中文](https://img.shields.io/badge/语言-简体中文-d73a49)](./README.zh-CN.md)

Brace is a desktop terminal application built with Tauri, React,
TypeScript, Rust, and xterm.js. It provides multiple terminal tabs, shell
selection, a working-directory-aware file tree, terminal search, themes, and
custom backgrounds in a lightweight native window.

> [!IMPORTANT]
> Windows is the currently implemented platform. The project compiles on
> macOS, but macOS shell discovery, working-directory reporting, keyboard
> shortcuts, and native window behavior are not complete yet.

## Features

- Multiple terminal tabs backed by native pseudo terminals
- PowerShell, Command Prompt, and Git Bash discovery on Windows
- File tree synchronized with the active PowerShell working directory
- Search, clickable links, copy and paste, and configurable font size
- Built-in themes and optional custom backgrounds
- Tauri desktop packaging for Windows and macOS

## Installation

Download the latest `Brace_x.y.z_x64-setup.exe` from the
[releases page](https://github.com/Albert0x/Brace/releases/latest) and run it.

> On first launch, Windows SmartScreen may show "Windows protected your PC"
> because the installer isn't code-signed yet. Click **More info → Run anyway**
> to continue. Once installed, Brace updates itself automatically.

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
pnpm tauri dev
```

Frontend-only development is available with `pnpm dev`, but PTY, filesystem,
window, and opener APIs require the Tauri runtime.

## Validation and Build

Every change must pass the checks relevant to the modified code:

```bash
pnpm build
cargo check --locked --manifest-path src-tauri/Cargo.toml
pnpm tauri build
```

`pnpm tauri build` is required before publishing a platform package. A frontend
build alone does not prove that native terminal behavior works.

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

| Capability | Windows | macOS |
| --- | --- | --- |
| Build | Supported | Compiles |
| Shell discovery | PowerShell, CMD, Git Bash | Not implemented |
| Default terminal startup | Supported | Not implemented |
| Working-directory sync | PowerShell | Not implemented |
| Native shortcuts | Ctrl-based | Command mapping not implemented |
| Native window styling | Acrylic | Not implemented |

macOS contributors should not duplicate Windows-specific logic. Shell
selection, quoting, shortcuts, status text, and window behavior must be
implemented behind explicit platform-aware boundaries.

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

- Automated tests and continuous integration are not configured.
- Content Security Policy is currently disabled.
- Shell command quoting is not yet safely abstracted per shell.
- The application metadata still contains initial Tauri template values.

These limitations must be treated as tracked engineering work, not as evidence
that the corresponding behavior is safe or supported.
