# Contributing to Brace

This document defines the minimum engineering standard for every contributor.
Brace is a Windows-only application (see [Platform Scope](#7-platform-scope)).
Work that touches shell behavior must state which shells it was verified
against, and must state any unverified behavior explicitly.

## 1. Before Starting

1. Create or select an issue with a clear problem statement and acceptance
   criteria.
2. Confirm whether the change affects the frontend, native backend, packaging,
   or more than one shell.
3. Keep the change focused. Do not combine feature work, dependency upgrades,
   formatting sweeps, and unrelated refactoring in one pull request.

## 2. Branches

Create branches from the current integration branch:

- `feature/<short-description>`
- `fix/<short-description>`
- `refactor/<short-description>`
- `docs/<short-description>`
- `chore/<short-description>`

Use lowercase kebab-case. Do not develop directly on `main`.

## 3. Commits

Use Conventional Commit-style messages:

```text
feat(profiles): inject environment variables into new terminals
fix(pty): terminate child processes when closing a tab
docs: document the Git Bash quoting rules
```

Allowed common types are `feat`, `fix`, `refactor`, `test`, `docs`, `build`,
and `chore`. A commit must represent one coherent change and must not include
generated `dist`, `node_modules`, `target`, editor metadata, or secrets.

Do not bypass Git hooks or validation with `--no-verify` to make a failing
change pass.

## 4. Pull Requests

Every pull request must include:

- The problem and why it needs to be solved
- The implementation approach and important trade-offs
- Which shells (PowerShell 5.1 / 7, CMD, Git Bash) the change was verified on
- Commands executed and their results
- Manual verification steps for terminal behavior
- Screenshots for visible UI changes
- Known limitations and behavior that was not verified

At least one maintainer review is required. Changes to PTY or shell handling
should be reviewed by someone who has actually run them against the affected
shells.

## 5. Frontend Rules

- Keep TypeScript strict and do not suppress errors without a documented reason.
- Preserve terminal focus, resize, cleanup, and multi-tab behavior.
- Dispose event listeners, xterm addons, and Tauri resources during component
  cleanup.
- Prefer existing themes and CSS conventions over one-off inline styles.
- Lazy-load optional heavy features and keep stable vendor chunk boundaries.
- Do not change backend command contracts from the frontend without updating
  and validating both sides.

## 6. Rust and Tauri Rules

- Tauri commands must return actionable errors instead of silently succeeding
  when the requested session or path does not exist.
- Do not hold a global mutex while performing slow or blocking operations.
- A closed terminal must not leave a child process or reader thread running.
- Keep capability permissions minimal. New permissions require a security
  explanation in the pull request.
- Do not disable security controls merely to unblock development.

## 7. Platform Scope

**Brace targets Windows only.** Shell discovery, cwd reporting, secret storage
(DPAPI), process-tree termination (`taskkill /T`), acrylic window chrome, and
system-proxy lookup are all written against Windows APIs. macOS and Linux are
not supported and not currently planned.

Writing Windows-specific code is therefore expected, not a smell. Two rules
still apply:

- Guard platform-specific code with `#[cfg(windows)]` and give the other branch
  something honest — return `None`, return an empty result, or say plainly in a
  comment that it is unimplemented. Do not fake success on a platform where the
  feature does not work. `dpapi()` returning `None` off Windows is the pattern
  to follow.
- Do not delete a working `cfg` boundary just because only one branch runs
  today. They are the seams a future port would use, and they cost nothing.

**Windows is not one platform for shells.** PowerShell 5.1, PowerShell 7, CMD,
and Git Bash differ in quoting syntax, prompt injection, path format, and
default encoding. Anything touching shell startup, quoting, or cwd reporting
must be verified against all four — that is where the real portability work in
this codebase lives.

Changes affecting PTY lifecycle, shell startup, quoting, cwd reporting,
clipboard behavior, keyboard shortcuts, window chrome, packaging, or signing
must state which shells and which Windows versions were verified.

## 8. Security Rules

- Never concatenate untrusted paths or text into shell commands.
- Treat terminal output, URLs, filenames, clipboard contents, and background
  image data as untrusted input.
- Avoid broad Tauri capabilities and unrestricted external URL opening.
- Never commit credentials, signing identities, provisioning profiles, tokens,
  private keys, or local environment files.
- Changes to CSP, opener permissions, filesystem access, or command execution
  require explicit security review.

## 9. Dependencies and Lockfiles

- Use Node.js from `.nvmrc` and pnpm declared in `package.json`.
- Install with `pnpm install --frozen-lockfile`.
- Do not use `--force` to conceal a toolchain mismatch.
- Dependency upgrades must be intentional, scoped, and explained.
- Commit `pnpm-lock.yaml` or `Cargo.lock` only when dependency metadata was
  intentionally changed.

## 10. Required Validation

CI (`.github/workflows/ci.yml`) runs these on every pull request. Run them
locally first — a red CI is slower feedback than a red terminal:

```bash
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib
```

`clippy` and `rustfmt` are toolchain components; install them once with
`rustup component add clippy rustfmt`.

Release steps are in [docs/RELEASING.md](docs/RELEASING.md).

For native or release-related changes, also run:

```bash
pnpm tauri build
```

Tests must cover the happy path, edge cases, invalid input, and cleanup or
failure behavior where applicable. Pure logic (encoding detection, stream
decoding, status parsing, secret sealing) belongs in `#[cfg(test)]` unit tests.
Behavior that needs a live PTY, a real window, or the Windows credential store
cannot be covered there — document repeatable manual steps and their observed
results instead, and say plainly which parts were verified that way.

Build success is not proof that PTY behavior works. Manually verify creating,
switching, resizing, and closing tabs; entering input; rendering output; cwd
changes; file-tree navigation; copy and paste; and application shutdown.

## 11. Definition of Done

A change is complete only when:

- Acceptance criteria are met
- Relevant checks pass
- Behavior is verified on the shells the change can affect
- Unverified behavior is disclosed
- Documentation is updated when setup or behavior changes
- The working tree contains no unrelated or generated files

Do not describe a change as complete when validation is failing or was skipped.
