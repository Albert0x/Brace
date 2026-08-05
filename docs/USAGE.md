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
- [Git commit panel](#git-commit-panel)
- [AI usage display (the highlight)](#ai-usage-display-the-highlight)
- [Profiles: switching APIs and proxies (the highlight)](#profiles-switching-apis-and-proxies-the-highlight)
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
- **Session restore**: close Brace and reopen it, and your tab group comes back —
  each tab's shell and directory, plus which one was selected. Note that these
  are *fresh* shell processes; a command left running is not resumed.

---

## File tree sync

The file tree and terminal stay in **two-way sync**:

- `cd` in the terminal and the tree re-roots to that folder.
- **Double-click a folder** in the tree and the terminal `cd`s there.
- **Auto-refresh**: `mkdir`, `rm`, or a branch switch in the terminal updates
  the tree on its own.
- Click `⟳` to refresh by hand — that re-reads the root *and* every expanded
  subfolder.
- **Create, rename, delete**: the right-click menu covers new file / new folder,
  rename, delete, copy path, and reveal in Explorer. Right-clicking empty space
  targets the root folder. The `＋` in the header is a shortcut for a new file
  at the root.
- `.gitignore` and `.env` **show by default** — on Windows those aren't hidden
  files. Things Windows actually marks hidden (the `.git` folder, for one) stay
  out of sight until you turn on Settings → General → **Show hidden files**.

> How it works: Brace injects a prompt into each shell that reports the current
> directory via the OSC 9;9 escape sequence, with per-shell syntax for
> PowerShell / CMD / Git Bash.

> **Delete goes to the Recycle Bin**, not gone for good, so a misclick is
> recoverable. Brace deliberately offers no permanent delete — the terminal is
> right there if you mean it.

> Watching covers **the root plus every expanded folder**, each one level deep,
> never recursive. The working directory is often your home folder, and watching
> that recursively would pull in every AppData, OneDrive, and browser-cache
> write — high volume, no value. Collapsed folders aren't watched; you can't see
> them anyway.

---

## Git commit panel

The `⑂ branch ±N` badge in the status bar opens it. It does exactly one thing:
**commit the changes you have right now**.

- **Check the files to include.** Everything is checked by default; uncheck what
  you don't want in this commit.
- **Click a filename to expand its diff** — green for added lines, red for
  removed. An untracked file is shown as entirely new.
- **Pick a type** (optional): the defaults are the standard Conventional
  Commits set (`feat` `fix` `docs` `style` `refactor` `perf` `test` `build` `ci`
  `chore` `revert`), with an optional scope beside it. The result is composed as
  `feat(git): your description`, previewed on the line below so you see exactly
  what lands in git. Click the same type again to clear it — a throwaway commit
  shouldn't be forced into the convention.
- Your team uses a different vocabulary? **Settings → General → Git → Commit
  types** takes a comma-separated list; leave it empty to restore the
  defaults.
- Write a message and hit **Commit**, or press `Ctrl+Enter`. **Commit & Push**
  does both.
- Failures (no `user.name`, rejected push, no upstream…) show git's own wording,
  unedited.

> **Only the files you checked get committed.** If you ran `git add` on
> something else in the terminal earlier, it will not be swept into this commit —
> the panel scopes the commit to the selected paths. That differs from a plain
> `git commit`, which commits everything staged.

> **No line-by-line staging.** To commit part of a file, use `git add -p` in the
> terminal. Rebuilding VS Code's source control view inside a terminal sidebar
> isn't what this panel is for.

The file tree uses the same status colors: yellow `M` modified, green `A`/`?`
added or untracked, red `D` deleted, blue `R` renamed, plus a small yellow dot on
folders containing changes. Ignored files are dimmed. Turn the whole thing off in
Settings → General → **Git decorations**.

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

## Profiles: switching APIs and proxies (the highlight)

A profile is a named set of environment variables. Whichever profile is active gets
injected into **terminals you open from then on**. The usual reasons to want this are
switching between Claude API relays, or putting a proxy on your terminals only.

### Creating one

Settings `⚙` → **Profiles** → `＋ New profile`:

1. Name it — say "Work relay".
2. Hit **Templates → Claude Code** to prefill `ANTHROPIC_BASE_URL`, `ANTHROPIC_AUTH_TOKEN`, and `ANTHROPIC_MODEL`.
3. Fill in the values and press **Save**.

There are also **Codex / OpenAI** and **Proxy** templates. **Fill from system proxy**
reads whatever Windows is currently using (your Clash setup, for instance) straight into
`HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY`, so you don't have to dig through the registry.

### Switching

The `🔑` badge in the status bar shows the active profile. Click it to switch, or pick
**Off** to inject nothing.

> **New tabs only.** A running process cannot have its environment changed — that's an
> operating system rule, not a shortcut taken here. After switching, open a fresh tab
> with `Ctrl+T`.

### How secrets are stored

- The 🔒 on a row marks it as a secret. Names containing `TOKEN`, `KEY`, `SECRET`, or `PASSWORD` are locked automatically; you can toggle it either way.
- Locked values are encrypted with **Windows DPAPI under your user account** before hitting disk, so a config file that gets synced to cloud storage or shared by accident is not readable by anyone else.
- The file lives at `%APPDATA%\com.brace.dev\profiles.json`. Because encryption is tied to your Windows account, **copying it to another machine will not carry the secrets over** — you'll re-enter them there.
- Plaintext secrets exist only in memory at write and inject time. The UI never echoes them back, it only shows "saved" or "not set".
- Renaming a variable that already holds a secret detaches it from the stored value, so the field flips back to "not set" to prompt you to re-enter it.

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

The settings panel (`⚙`) has four tabs:

- **General**: appearance, interface language (en/zh), UI zoom, show hidden
  files, Git decorations, WebGL rendering, cursor blink, **Agent Usage** (the
  Claude usage toggle).
- **Themes**: theme selection and background image.
- **Profiles**: environment variable sets — see
  [the section above](#profiles-switching-apis-and-proxies-the-highlight).
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
