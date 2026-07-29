# Brace User Guide

English · [简体中文](./USAGE.zh-CN.md)

Brace is a lightweight Windows terminal built on Tauri + Rust. Its standout
feature is showing your **Claude / Codex usage live in the status bar**. This
guide covers everything you'll use day to day.

---

## Contents

- [Installation](#installation)
- [Interface overview](#interface-overview)
- [Tabs & shells](#tabs--shells)
- [File tree sync](#file-tree-sync)
- [AI usage display (the highlight)](#ai-usage-display-the-highlight)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Themes & backgrounds](#themes--backgrounds)
- [Settings](#settings)
- [Auto-updates](#auto-updates)

---

## Installation

1. Download `Brace_x.y.z_x64-setup.exe` from
   [Releases](https://github.com/Albert0x/Brace/releases/latest) and run it.
2. **On first launch** Windows SmartScreen may show "Windows protected your PC"
   because the installer isn't code-signed yet. Click **More info → Run anyway**.
3. Once installed, Brace checks for and delivers updates on its own.

---

## Interface overview

Brace uses a three-column layout plus a status bar:

- **Top bar**: tabs, new-tab `＋` (with a `▾` to pick a shell), search, settings
  `⚙`, and window controls.
- **Left**: file tree (follows the active terminal's directory).
- **Center**: the terminal.
- **Bottom status bar**: **AI usage** on the left (when claude/codex is running),
  font size / terminal count / theme / encoding / OS on the right.

---

## Tabs & shells

- **New tab**: click `＋` or press `Ctrl+T`. New tabs **inherit the current
  directory** automatically.
- **Pick a shell**: click the `▾` next to `＋` — PowerShell, PowerShell 7,
  Command Prompt, or Git Bash. Brace auto-detects what's installed (Git Bash is
  found by tracing back from `git.exe`).
- **Close tab**: click `×` on the tab or press `Ctrl+W`.
- **Switch tabs**: `Ctrl+Tab` / `Ctrl+Shift+Tab`.
- Tab titles show the current directory name.

---

## File tree sync

The file tree and terminal stay in **two-way sync**:

- `cd` in the terminal and the tree re-roots to that folder.
- **Double-click a folder** in the tree and the terminal `cd`s there.
- Click `⟳` to refresh.
- To show hidden files (`.env`, `.gitignore`): Settings → General → **Show
  hidden files**.

> How it works: Brace injects a prompt into each shell that reports the current
> directory via the OSC 9;9 escape sequence, with per-shell syntax for
> PowerShell / CMD / Git Bash.

---

## AI usage display (the highlight)

Brace's signature feature — your **Claude / Codex usage, live in the status
bar**. It only shows when the active tab is **actually running** claude or codex
(process detection).

### Claude

Claude's official usage (context / 5-hour / weekly, with reset times) comes
through Claude Code's **statusLine** channel, so enable it once:

1. Settings → **General** → scroll to **Agent Usage** → turn on "Show Claude
   usage in status bar".
   - This installs a collector into your `~/.claude/settings.json`. If you
     already have another statusLine configured, Brace won't overwrite it and
     will tell you.
2. **Start / restart claude** (statusLine is read at launch — if claude is
   already running, `/exit` and reopen).
3. Send a message. Quota data only arrives after an API response, so it appears
   once you've sent something.

Reading the bar: `[model]  Context 61%  5h 35% 4h7m  7d 48% 35h`
- **Context**: how full the current session's context window is.
- **5h / 7d**: 5-hour and weekly quota used %, with a reset countdown.

### Codex

Codex needs **no setup** — just run `codex` and the bar shows
`Codex  Context x%  xxK tok` (context usage + session token total).

> Codex shows only context and tokens, **not** 5-hour/weekly limits — its local
> session files leave the rate_limits fields empty, so the data simply isn't
> there.

### Pasting images to Claude

To paste an image into claude (in Brace or any terminal), use **`Alt+V`**, not
`Ctrl+V`. `Ctrl+V` is claimed by the terminal as "paste text", so the image
never reaches claude; `Alt+V` is Claude Code's dedicated image-paste key on
Windows, and inserts an `[Image #1]` placeholder.

---

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+T` | New tab |
| `Ctrl+W` | Close current tab |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Ctrl+F` | Focus the search box |
| `Ctrl+Shift+C` | Copy selection |
| `Ctrl+Shift+V` | Paste (bracketed paste, so claude/vim tell paste from typing) |
| `Ctrl+=` / `Ctrl+-` | Zoom font in / out |
| `Ctrl+0` | Reset font to default (14px) |
| `Alt+V` | (in claude) paste an image from the clipboard |
| Right-click | Copy / Paste / Select all / Clear menu |

---

## Themes & backgrounds

Settings → **Themes**:

- **Five built-in themes**: HyperDark (default near-black glass), Tokyo Night,
  Dracula, Nord, Gruvbox — switch instantly.
- **Background image**: click "Choose image…" to set any image as a
  frosted-glass background; drag the overlay slider to tune brightness; "Clear"
  to remove.
- **Appearance**: Settings → General → Appearance — System / Light / Dark.

---

## Settings

The settings panel (`⚙`) has three tabs:

- **General**: appearance, interface language (en/zh), UI zoom, show hidden
  files, Git decorations, WebGL rendering, cursor blink, **Agent Usage** (the
  Claude usage toggle).
- **Themes**: theme selection and background image.
- **About**: version info, check for updates, GitHub / issue links.

---

## Auto-updates

Brace ships signed auto-updates:

- Settings → About → **Check for updates**. If a newer version exists, confirm
  and it downloads and relaunches automatically.
- The update chain works from v0.1.2 onward (the first release after the rename
  to Brace). Older HyperTerminal installs have a different bundle id, so install
  Brace once by hand.

---

Questions or ideas? Open a [GitHub Issue](https://github.com/Albert0x/Brace/issues).
