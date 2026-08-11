# Brace Roadmap

Scope, acceptance criteria, and the reasoning behind a few load-bearing calls — so
that the same discussion does not have to be had again in a few weeks.

At the time of writing, 0.1.8 is in review. Effort figures are working days of actual
work, not calendar time.

## Version overview

| Version | Theme | Effort | Skippable? |
| --- | --- | --- | --- |
| 0.1.8 | Stabilization: silent failures, usability defects, engineering baseline | 4-6d | No — 0.1.7 actively misleads users |
| 0.2.0 | SSH remote sessions (scratch-your-own-itch) | 2-3d | Yes, but the author needs it daily |
| 0.3.0 | AI command suggestions + command history + third-party API balance | ~1.5w | Yes — this is a selling point, not a blocker |
| 0.3.x | Custom shells/WSL + per-tab profile binding | ~3d | Ships alongside, no release of its own |

Versions are cut where **a batch of changes reaches a boundary at which it can be
verified and rolled back independently** — not where enough features have piled up.
Far apart in completion time and independently valuable → ship separately. Finished
together and mutually coupled → ship together.

---

## v0.1.8 · Stabilization

**Goal:** when something on a core path fails, the user can tell what happened and is
not misled into doing something destructive.

No new features. The entire value of this release comes from adding nothing.

### Batch 1 · Infrastructure (~1d)

| Item | Content |
| --- | --- |
| D1 | vitest + eslint, wired into CI. First tests cover `useTabs` session restore and closing, `usePersisted` bad-data fallbacks, `i18n` interpolation |
| B4 | `.gitattributes` (cures the phantom diffs caused by `core.autocrlf=true`, and keeps CI consistent); repo `.gitignore` gains `.claude/settings.local.json`, which until now was only masked by a personal global gitignore and would leak for anyone else cloning |
| R1 | Release automation: tag triggers build → sign → generate `latest.json` → create release. **Prerequisite: the signing key goes into GitHub repository secrets by hand; it never enters the repo** |

> The `eslint-disable-next-line` in `usePolling.ts` is currently decorative — eslint
> was never installed. D1 makes it real.

### Batch 2 · Failure paths (~1.5d)

What these share is not "something is broken" but "something breaks silently, or
reports the opposite of the truth". The latter is worse: it leads users into damage.
All covered by unit tests.

| Item | Problem | Fix |
| --- | --- | --- |
| A1 | Nothing in the UI when the process exits; the dead session stays in `sessions` and `pty_write` only `console.error`s. The user faces a box that accepts typing and never answers | `pty-exit` carries the exit code; `useTabs` gains `exitedMap` (same shape as `cwdMap`); tab dims, exit code printed in the terminal, input swallowed; **restart in place** (same sessionId, xterm instance kept so scrollback survives, cwd from `cwdMap[id]`); `pty_write` returns a recognizable error for dead sessions; `pty_close` idempotent |
| A2 | Commit succeeds, push fails → reported as total failure. The user clicks again and gets an empty commit or a polluted history | `git_commit` returns a structured outcome (`committed` / `pushed` / `push_error`); frontend renders three states |
| A3 | A git subprocess hangs forever on a credential prompt (no window + empty stdin + no timeout), freezing the commit panel | `git_cmd` sets `GIT_TERMINAL_PROMPT=0` and `GCM_INTERACTIVE=never`. **This changes behaviour**: a push needing credentials goes from "hangs" to "fails clearly", so it must ship with guidance ("run `git push` once in the terminal to authenticate") |
| A4 | After renaming a secret the UI says "not set", but the old ciphertext is still on disk and still injected into new terminals | Semantics fixed as **discard-and-recreate**: orphaned ciphertext is dropped on save so UI and disk can never disagree. **Behaviour change — some users must re-enter a key; the release notes must say so** |
| A5 | `encryption_available` is the compile-time constant `cfg!(windows)`, while `seal()` silently falls back to plaintext → tokens land unencrypted while the UI claims DPAPI | Probe at runtime (one seal+unseal round trip); when `seal` fails the UI says so. **A lying security indicator is worse than none** |
| A6 | With an image on the clipboard, `Ctrl+Shift+V` makes `readText()` return an empty string and nothing happens at all | Detect the image and say "use `Alt+V` to send it to Claude". Thumbnail preview is an enhancement, not in this release |

### Batch 3 · Usability, performance, docs (~1.5d)

| Item | Content |
| --- | --- |
| B1 | Configurable `scrollback`. xterm's default of 1000 lines means one build log wipes out everything above it |
| B2 | Resizable sidebar. Currently pinned at `flex: 0 0 220px`, so the file tree is all ellipses. The terminal must refit after the drag |
| B3 | Fix README's "Current Limitations" (claims CI is absent and CSP disabled — both false); `Cargo.toml` lib name `tauri_app_lib` → `brace_lib` (**must update `main.rs`, and it touches artifact naming, so a full `tauri build` is required**); document coexistence with CC Switch in USAGE.md (see decision log) |
| C1 | Tie `git status --ignored` to the `gitDeco` toggle. With decorations off the cost is pure waste — in a repo with `node_modules` that is tens of thousands of paths enumerated and shipped over IPC every 20s, then discarded |
| C2 | Size limit for background images (currently unchecked: a 20MB image becomes a 27MB string, read into memory in full on every launch and stuffed into a CSS `url()`) |
| S1 | Four things about search: pass `decorations`, show `n/m` via `onDidChangeResults`, say "search terminal output" in the placeholder, and highlight-only while typing (`Enter` jumps). **All four together** — no single one removes the "search doesn't work" feeling, and it is two sides of the same coin as B1, since the content had already scrolled away |

### Batch 4 · Measurement (0.5-1d)

Results in the "Measurements" section below. Both items exceeded the threshold and
were changed.

### Batch 5 · Release (0.5d)

- D4: manual verification checklist in `RELEASING.md` (PTY exit and restart, the three
  push outcomes, secret removal and machine change, encoding detection, updater)
- Walk the checklist → ship **v0.1.8** (first use of the R1 automation)

### Acceptance criteria

- Each of the six A items has a unit test or a reproducible verification step — none
  rest on "looks fine"
- CI green, including the newly added lint/test
- README matches the implementation line by line
- `git status` no longer shows phantom diffs
- The manual checklist passes end to end

---

## v0.2.0 · SSH remote sessions

**Positioning: a tool for the author.** Connecting to servers daily without opening a
second application is a legitimate reason. It is not expected to become a selling
point, and it does not get more than a couple of days.

### Prerequisite

**D3: split `lib.rs`** (2155 lines → `pty` / `fs` / `git` / `profiles` / `usage` /
`shells`; a pure move, with the existing Rust tests as the safety net). Both 0.2.0 and
0.3.0 add backend modules, which promotes this from optional to **mandatory**.

### Scope

**In:**
- Session list: host / port / user / key path / note. Reuses the Profile storage
  pattern (JSON + tmp rename)
- Assemble an `ssh` command and spawn it into the existing PTY — this is one more shell
  type, not a new subsystem
- Visible state for connection failure and disconnect (reuses A1's exit marker and
  restart, for free)

**Out:** key negotiation of our own, tunnels/port forwarding, jump hosts, SFTP,
session grouping trees.

### Two hard requirements

1. **Never store SSH passwords.** Password prompts work naturally inside the PTY;
   passwordless access goes through keys and `~/.ssh/config`. **Brace never touches SSH
   credentials, so it carries no responsibility for them** — the biggest advantage of
   the wrapper approach.
2. **Remote tabs must be marked as such.** The moment `ssh.exe` takes over the PTY the
   local shell is gone, the injected OSC 9;9 does not run remotely, and `cwdMap` freezes
   at its pre-connection value → **the file tree shows a local directory, git
   decorations show a local repo, the status bar shows a local branch — all of it
   lying**. That is the same class of problem as the A items; it must not be
   reintroduced immediately after being fixed. A remote tab disables the file tree, git
   decorations, the usage row, and the profile badge. **This cost is part of the MVP,
   not an option.**

---

## v0.3.0 · AI command suggestions + history + API balance

All three share one foundation (HTTP client on the Rust side, Profile credential
access, redaction), so they ship together. Splitting them means building that layer
twice.

### AI command suggestions

**The big lever:** Profiles already carry `ANTHROPIC_BASE_URL` /
`ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_MODEL`, complete with DPAPI encryption and
one-click system-proxy fill. **Credential management, endpoint config, proxying and
encrypted storage need zero new code — the feature is usable the day it lands.**

- Trigger: `Ctrl+K` opens a prompt, natural language → 1-3 candidates → `Enter`
  **inserts into the terminal without executing**.
  **No autocomplete-as-you-type** — an LLM answers in seconds, typing happens in
  fractions of one; suggesting continuously is slow, expensive and distracting
- **Requests go through Rust; the frontend never touches the network.** `connect-src`
  is currently `'self'` plus ipc, so a frontend fetch is blocked by CSP — and opening
  CSP for one feature trades global safety for local convenience. The cost is one new
  HTTP dependency (`reqwest` or `tauri-plugin-http`)
- Context: shell type + cwd + git branch by default; optionally "include the last N
  lines of output", **redacted on the Rust side before sending** (`sk-` / `ghp_` /
  `gho_` / `Bearer ` / `AKIA` / `ANTHROPIC_*` and `OPENAI_*` assignments /
  `password=`, `token=` pairs), with the checkbox stating plainly that output will be
  sent to the model.
  **Redaction must have unit tests** — it is a security promise, and eyeballing it does
  not qualify
- Out: auto-execution, RAG, multi-turn, agents

> Refusing to send any output at all would remove the single most valuable case ("I
> just got this error, give me the fix"). Safety comes from redaction, off-by-default,
> and saying so — not from dropping the feature.

### Command history

Cross-tab, searchable, favouritable. Purely local, so no cost and no latency. A natural
companion to suggestions: an accepted suggestion goes straight into history.

### Third-party API balance

The shape already exists: the `🔑 profile-name` status bar item and its switcher menu
are a compact version of the CC Switch list. **All that is missing is one balance
line.**

- Adapters: **DeepSeek official + OneAPI/NewAPI family** (the latter covers most
  relay providers), everything else via "custom URL + JSON path"
- **Verify the real response shape with curl before implementing** — do not write field
  names from memory
- Requests go through Rust; `profile_balance(profileId)` calls `unseal` internally and
  **never returns the token to the frontend** (consistent with the existing design)
- Cache + manual refresh (copy CC Switch's "4 minutes ago + refresh button"),
  **no polling** — a balance does not change every 20 seconds
- **Unknown or unsupported → omit the line entirely**, never `0` or `—`. A pile of
  lying UI was just removed in 0.1.8; do not add more
- UI semantics: the existing usage row is Claude's **token usage** (5h/7d windows), a
  balance is **money**. They must be laid out separately so nobody reads "usage" as
  "spend"

**Providers with no balance endpoint:** neither Anthropic nor OpenAI offers one. That
is a ceiling, not an implementation gap.

### Optional: model configuration

Borrow CC Switch's three columns (menu label / actual model / context window). The
context window feeds usage percentages directly, and lets the user pick a cheap model
for command suggestions.

---

## v0.3.x · Ships alongside, no release of its own

- **Custom shells / WSL**: only the four auto-detected shells work today, so WSL users
  cannot get in. Once 0.2.0 has paved the "one more shell type" path, this costs less
  than currently estimated
- **Per-tab profile binding**: choose a profile when opening a tab, show the active one
  on the tab. Turns the "new terminals only" OS constraint from a source of confusion
  into explicit product behaviour

---

## Explicitly not doing

| Item | Reason |
| --- | --- |
| SFTP | The `ssh.exe` wrapper route gives no SFTP channel, so it needs a separate `sftp.exe` (interactive, hard to parse) or a native library — **a second implementation, not a free extra on top of SSH**. Remote browsing + transfer + progress + retry is a product in itself. `scp` is one line and costs nothing |
| Managing `auth.json` / `config.toml` | Overwrites hand-written config and fights CC Switch. CC Switch is already complex enough to need a banner explaining "what you are editing is not what is live" — the inevitable end of that road. Environment injection touches nobody's config, consistent with this project's habit of not fighting the user's tools |
| Plugin system | Building a plugin system with zero users is self-entertainment |
| Split panes | Touches layout, focus and size synchronization all at once; payoff does not match |
| Generic git timeout | After A3 the remaining risk is small and the cost is not |
| Linux | Out of scope. macOS work is underway from another contributor (see `tauri.macos.conf.json`); this roadmap does not cover it, and README's platform statement needs to be reconciled with it |

---

## Measurements (v0.1.8, batch 4)

Reproduce (the benchmark is `#[ignore]` and stays out of normal runs):

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib -- --ignored --nocapture bench_pty_read_chunks
```

**PTY output (1.1MB, `cmd /c type` on a ~1MB text file)**

| Metric | Value |
| --- | --- |
| read calls | 8016 (= IPC events before the change) |
| Mean chunk | **142 bytes** (read buffer is 4096; 3.5% utilization) |
| Median chunk | 143 bytes |
| Peak event rate | 8229/s |
| Aggregated over a 16ms window | **57 events, a 99.3% reduction**, ~20KB per event |

Conclusion: far past any reasonable threshold, so it changed. Reading and sending are
now separate threads — the reader only fills a buffer, the sender drains it once per
16ms frame. That delay is imperceptible.

**ConPTY does not return EOF — a serious defect found along the way**

The same benchmark showed that after `cmd /c type` finishes and exits, `read()` on the
master side **does not return EOF**; it was still blocked after 12 seconds. That means
A1's exit detection was dead on arrival — `pty-exit` was emitted after the read loop
ended, and that loop never ends.

It also showed that `drop(master)` **does** wake the blocked `read()`. Hence the fix:
the sender thread discovers the exit with `try_wait()`, then removes the session (whose
destructor drops the master), which unblocks the reader so no thread leaks.

**The "occasional 0%" in statusline usage — cause identified, no longer guesswork**

This line in `usage_stats`:

```rust
stats.context_pct = cache["context_window"]["used_percentage"].as_f64().unwrap_or(0.0);
```

When the cache file exists and is fresh but lacks `context_window.used_percentage`,
`unwrap_or(0.0)` turns "cannot read it" into "0% used" — and `has_data` is already
`true` by then, so the frontend renders exactly that.

An earlier review of this only reached the 15-minute freshness gate (which does hide
the whole row) and never got to this line, concluding wrongly that the path did not
exist. The original report was right.

The fix is not a diagnostic field but separating "unknown" from "genuinely zero" in the
data: a new `has_context` field, with the frontend showing a placeholder rather than a
fabricated 0%. The `note` field (`cache-missing` / `cache-stale` / `context-missing` /
`codex-session-missing`) remains as a lead for next time.

## Technical debt register

Not scheduled. Recorded here to be picked up when touching the surrounding code, or to
have evidence on hand when it finally bites.

**`react-hooks` v7 rules: 11 violations across 7 files.**
`react-hooks/refs` (6) and `react-hooks/set-state-in-effect` (5) are React
Compiler readiness checks added in eslint-plugin-react-hooks v7. The violations are two
idioms: assigning the latest value to a ref during render for an effect to read
(`usePolling` / `useTabs` / `useFileTree` / `TerminalView`), and calling setState
synchronously in an effect to load data (`GitPanel` / `PreviewPanel` / `ProfilePanel` /
`useAppearance` / `useFileTree`). The concurrency hazard they point at is real but only
materializes under Suspense or `startTransition`, neither of which this project uses.
Fixing them means changing timing, which is a refactor of its own; set to `warn` for
now. Whenever it happens, the precondition is test coverage on the affected modules.

**The zero trap in `usePersistedNumber`.**
The implementation is `Number(raw) || initial`, and `0` is falsy — **a stored 0 reads
back as the default**. All three current call sites (`ht-fontsize` 8-28, `ht-zoom`,
`ht-overlay` 0.2-0.95) cannot reach 0, so nobody has hit it. Any future numeric setting
that allows 0 will, and the symptom — "I set it to 0 and it reverts on restart" — is
miserable to track down. A test in `usePersisted.test.ts` pins the current behaviour and
labels it as a trap, so it fails loudly whenever someone fixes it. The fix is one line:
`Number.isFinite(n) ? n : initial`.

## Decision log

Calls that took several rounds to settle, recorded so they do not get relitigated.

**Why 0.1.8 carries no new features.**
The updater is live and installed users sit on 0.1.7, where A5 writes tokens in
plaintext while claiming encryption and A2 leads users into polluting their history.
Those are **wrong signals already in production**, not items awaiting improvement. And
the A items are all things only a real machine can verify (PTY lifecycle, git
subprocesses, DPAPI); shipping them together with features would merge two batches of
high-risk change into a single verification, with no way to bisect a failure.

**Why SSH comes before AI suggestions.**
SSH takes 1-2 days and benefits the author daily; suggestions take a week and block
nobody. Making a finished, daily-use feature wait for one that has not started buys
nothing.

**Why the features are not batched into one release.**
All five (SSH / suggestions / history / WSL / profile binding) touch core paths; shipping
them together means not knowing which one broke things. "Don't version for the sake of
versioning" is the right principle, but **holding finished work back to fill out a
release is what actually lets version numbers dictate value delivery**. The real problem
to solve is release cost (→ R1 automation), not release count.

**Why wrap `ssh.exe` instead of using a native Rust SSH library.**
Windows 10+ ships an OpenSSH client; spawning it into the existing PTY reuses keys,
`known_hosts` and `~/.ssh/config` wholesale, and the MVP is a day or two. `russh` means
owning key negotiation, host fingerprints, keepalive and reconnection — weeks of work
reimplementing what OpenSSH already got right, plus custody of credentials.

**Why SSH stays a personal tool rather than a selling point.**
Everything that differentiates Brace — Claude usage, profiles, git-decorated file tree,
file preview — stops working in a remote session. Treating it as a selling point means
spending a week to obtain an SSH client with no advantages, against competitors with a
decade of polish.

**Coexisting with CC Switch** (documented in USAGE.md).
CC Switch rewrites config files, Brace injects environment variables, and **environment
variables win**. So a terminal opened in Brace uses Brace's profile and overrides the
provider just selected in CC Switch. Users hit "I switched in CC Switch, why is claude
still on the old endpoint", and tracing it back is hard.
