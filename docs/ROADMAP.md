# Brace Roadmap

Scope, acceptance criteria, and the reasoning behind a few load-bearing calls — so
that the same discussion does not have to be had again in a few weeks.

Effort figures are working days of actual work, not calendar time.

## Version overview

| Version | Theme | Effort | Skippable? |
| --- | --- | --- | --- |
| 0.1.8 ✅ | Stabilization: silent failures, usability defects, engineering baseline | 4-6d | No — 0.1.7 actively misleads users |
| 0.2.0 ✅ | SSH remote sessions (scratch-your-own-itch) | 2-3d | Yes, but the author needs it daily |
| 0.2.1 | Fixes: Ctrl+V paste into Git Bash, Git Bash detection, Alt+click with CJK | 0.5d | No — paste is broken every day |
| 0.3.0 | Shell integration + command history | 2-3d | Yes |
| 0.4.0 | AI command suggestions + third-party API balance | 5-6d | Yes — this is a selling point, not a blocker |
| 0.4.x | Custom shells/WSL + per-tab profile binding | ~3d | Ships alongside, no release of its own |

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

## v0.2.0 · SSH remote sessions — shipped

**Positioning: a tool for the author.** Connecting to servers daily without opening a
second application is a legitimate reason. It is not expected to become a selling
point, and it does not get more than a couple of days.

Built as planned: wraps the system OpenSSH client, stores no passwords, and marks
remote tabs so the local file tree / git decorations / profile badge get switched off
rather than displaying local state that looks remote.

Two things worth recording, because both are the kind of thing that gets "helpfully"
undone later:

- **A blank field is left off the command line.** No port → no `-p`, no key → no `-i`,
  no user → no `user@`. `~/.ssh/config` decides, aliases and `ProxyJump` included.
  Nine tests pin this. Filling in defaults here is how you get "connects in my
  terminal, not in Brace".
- **There is no password field, and a test asserts there never will be.** `ssh` asks
  in the PTY, so Brace holds no SSH credentials and has no duty to protect them.
  "Add a password field so I don't have to type it" is the obvious next request; CI
  refuses it rather than a reviewer having to remember.

Argument assembly lives in Rust with the frontend forwarding the result — one rule,
one implementation.

### Prerequisite — done

**D3: split `lib.rs`** ✅ Nine modules (`pty` / `fs_ops` / `git` / `preview` /
`profiles` / `usage` / `shells` / `system` / `diagnostics`) plus a 78-line assembly
layer. A pure move: 30 passed / 2 ignored, identical to before.

Splitting surfaced one hidden coupling — `usage_stats` was reaching into
`PtyManager.sessions` directly, which cost nothing while both lived in the same
file. It now goes through `PtyManager::pid_of()`. That is most of the point of
splitting a file this size.

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

## v0.3.0 · Shell integration + command history

Split out of the original 0.3.0 — see the decision log for why it now goes first.

### Shell integration

A terminal only sees a byte stream; Brace does not know which command ran.
**Recording keystrokes does not work**: Tab completion, arrow-key editing and history
recall all make the typed bytes differ from what actually executed.

The prompt injection that already reports cwd (OSC 9;9) is the mechanism. Extend it so
that after each command the shell reports **the command line and its exit code**:

- bash: in `PROMPT_COMMAND`, from `history 1` and `$?`
- PowerShell: in `function prompt`, from `Get-History -Count 1` and `$?` /
  `$LASTEXITCODE`
- cmd: no hook to report the command from — **cmd gets no history**, and the UI says so
  rather than showing an empty list that looks broken

The reporting must not change what the user sees: no extra output, no visible delay at
the prompt, `$?` preserved for the user's own prompt logic.

### Command history

Cross-tab, searchable, favouritable. Purely local, so no cost and no latency.

- Source is shell integration only — never keystrokes
- Remote (SSH) tabs record nothing: the injection does not run remotely
- Stored next to the other app data, size-capped; a command that looks like it carries a
  secret (the same patterns as the redaction below) is not stored
- Exit code shown per entry, so "the one that failed" is findable

### Acceptance criteria

- bash and PowerShell: the recorded command equals what executed, including after Tab
  completion and history recall — verified by hand, listed in `RELEASING.md`
- The parser for the reported sequence has unit tests, including commands containing
  quotes, `;`, non-ASCII and newlines
- No visible change at the prompt

---

## v0.4.0 · AI command suggestions + API balance

Both share one foundation (HTTP client on the Rust side, Profile credential access,
redaction), so they ship together. Splitting them means building that layer twice.

### AI command suggestions

**The big lever:** Profiles already carry `ANTHROPIC_BASE_URL` /
`ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_MODEL`, complete with DPAPI encryption and
one-click system-proxy fill. **Credential management, endpoint config, proxying and
encrypted storage need zero new code — the feature is usable the day it lands.**

- Trigger: `Ctrl+Shift+K` opens a prompt, natural language → 1-3 candidates → `Enter`
  **inserts into the terminal without executing**. Not `Ctrl+K` — see the decision log.
  **No autocomplete-as-you-type** — an LLM answers in seconds, typing happens in
  fractions of one; suggesting continuously is slow, expensive and distracting
- **Requests go through Rust; the frontend never touches the network.** `connect-src`
  is currently `'self'` plus ipc, so a frontend fetch is blocked by CSP — and opening
  CSP for one feature trades global safety for local convenience. The cost is one new
  HTTP dependency (`reqwest` or `tauri-plugin-http`)
- Context: shell type + cwd + git branch + **the last command and its exit code** (from
  0.3.0's shell integration) by default; optionally "include the last N
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

An accepted suggestion goes straight into history.

### Official usage vs third-party balance

The shape already exists: the `🔑 profile-name` status bar item and its switcher menu
are a compact version of the CC Switch list. What it shows next to it depends on what
kind of profile is active — **official shows official usage, third-party shows a
balance**. The two are never mixed: usage is quota, a balance is money.

Researched against CC Switch's source (`farion1231/cc-switch` @ 09bce30) rather than
designed from memory; the endpoints below are what it actually calls.

**Deciding which kind a profile is.** Official = the profile sets no
`ANTHROPIC_BASE_URL`, or sets it to `api.anthropic.com`. Anything else is third-party, and
the provider is picked by matching the base URL's host. CC Switch does the same
(empty base URL → official; balance provider by URL match).

**Official → the existing usage row, unchanged.** It already reads Claude Code's
statusline cache. CC Switch instead calls `GET api.anthropic.com/api/oauth/usage` with
the OAuth token from `~/.claude/.credentials.json` — an undocumented endpoint behind an
`anthropic-beta` header, and reading the user's Claude login token. Not worth either
risk while the cache works.

**Third-party → a balance line.** Built-in adapters, in order:

| Provider | Request (GET, `Authorization: Bearer <key>`) | Fields |
| --- | --- | --- |
| DeepSeek | `https://api.deepseek.com/user/balance` | `is_available`, `balance_infos[].currency`, `.total_balance` |
| OpenRouter | `https://openrouter.ai/api/v1/credits` | `data.total_credits - data.total_usage` |
| SiliconFlow | `https://api.siliconflow.cn/v1/user/info` (`.com` for USD) | `data.totalBalance` |

- **DeepSeek first** — the third-party provider most likely to be used. Its Claude Code
  base URL is believed to be `api.deepseek.com/anthropic` (unverified — confirm in the
  curl step), while the balance lives at the host root: match on host, not on the full
  URL
- **Still verify each with curl against a real key before implementing.** CC Switch's
  code is a strong lead, not proof; the user runs the curl, Brace's code never sees the
  key during research
- **OneAPI / NewAPI relays are not built in, and that is deliberate.** Their balance
  endpoint (`/api/user/self`) wants a *system access token* and a numeric
  `New-Api-User` id, not the API key the profile already holds — two more fields per
  profile, and quota in units of 1/500000 USD. Deferred until someone actually uses one
- **Custom: URL + JSON path, not scripts.** CC Switch runs user-written JS in an
  embedded QuickJS. That buys flexibility at the cost of a JS engine and a sandbox to
  maintain; a GET with a JSON path covers the common shape. Same rule CC Switch applies
  to its templates: HTTPS only, same host as the base URL — so a profile cannot be
  turned into a way to send its key somewhere else

**Behaviour**

- Requests go through Rust; `profile_balance(profileId)` resolves base URL and key
  itself via `unseal` and **never returns the key to the frontend**. CC Switch keeps
  third-party keys in frontend state; Brace's DPAPI design already does better, keep it
- Manual refresh by clicking the line, cached about 5 minutes, "updated N min ago" in
  the tooltip. **No polling**
- Keep the last good value for a while after a transient network failure rather than
  flickering the line away (CC Switch: 10 minutes)
- Three outcomes, kept distinct:
  - unsupported provider → **omit the line**, never `0` or `—`
  - definite failure (401, non-2xx, unparseable) → show it failed and why — hiding a
    revoked key is its own kind of lie
  - transient network error → keep last good value, retry once

**Providers with no balance endpoint:** neither Anthropic nor OpenAI offers one. That
is a ceiling, not an implementation gap.

### Optional: model configuration

Borrow CC Switch's three columns (menu label / actual model / context window). The
context window feeds usage percentages directly, and lets the user pick a cheap model
for command suggestions.

---

## v0.4.x · Ships alongside, no release of its own

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
| macOS / Linux | Out of scope. Brace is Windows-only; the macOS UI/DMG work (#23) was closed unmerged when that was decided, and the preview build `v0.1.8-macos-preview.1` is left up as-is but will not be updated |

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

**Why command history moved ahead of AI suggestions (decided after 0.2.0).**
The original plan bundled history, suggestions and balance into one 1.5-week release on
the grounds of a shared foundation — but history shares none of it: no network, no HTTP
dependency, no credentials. What it does need, shell integration, is exactly what makes
suggestions useful: "the last command failed with exit code 1" is reliable context for
"fix this", where scraping the output for an error is guesswork. And bundling would put
two high-risk changes — the prompt injection on every shell, and network plus
credentials — into one verification, the same reason 0.1.8 carried no features.

**Why the suggestion prompt is `Ctrl+Shift+K`, not `Ctrl+K`.**
`Ctrl+K` is kill-to-end-of-line in both readline and PSReadLine, and people use it.
Taking it would repeat the Ctrl+V lesson from 0.2.1 in reverse — there, the terminal
leaked a key to the shell; here, it would steal one. Terminal-level shortcuts carry
Shift, like `Ctrl+Shift+C` / `Ctrl+Shift+V`.

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
