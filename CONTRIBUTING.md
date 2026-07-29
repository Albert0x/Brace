# Contributing to Brace

This document defines the minimum engineering standard for every contributor.
Platform-specific work must preserve behavior on platforms the contributor
cannot test and must state any unverified behavior explicitly.

## 1. Before Starting

1. Create or select an issue with a clear problem statement and acceptance
   criteria.
2. Confirm whether the change affects the frontend, native backend, packaging,
   or more than one platform.
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
feat(mac): detect the user's default shell
fix(pty): terminate child processes when closing a tab
docs: document macOS prerequisites
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
- A list of affected platforms
- Commands executed and their results
- Manual verification steps for terminal behavior
- Screenshots for visible UI changes
- Known limitations and behavior that was not verified

At least one maintainer review is required. Native behavior should be reviewed
by someone familiar with the affected platform.

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

## 7. Cross-Platform Rules

Never assume:

- A specific executable such as `powershell.exe`, `/bin/zsh`, or `/bin/bash`
- Windows path separators or drive letters
- `$HOME` or `%USERPROFILE%` exists
- `Ctrl` is the native shortcut modifier on every platform
- Shell quoting syntax is shared by PowerShell, CMD, zsh, bash, and fish
- A transparent undecorated window behaves consistently on every platform

Platform behavior must be selected by an explicit Rust target boundary or a
tested runtime abstraction. Keep shared behavior shared and isolate only the
parts that truly differ.

Changes affecting PTY lifecycle, shell startup, quoting, cwd reporting,
clipboard behavior, keyboard shortcuts, window chrome, packaging, signing, or
filesystem paths must list Windows and macOS verification separately.

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

Run at minimum:

```bash
pnpm build
cargo check --locked --manifest-path src-tauri/Cargo.toml
```

For native or release-related changes, also run:

```bash
pnpm tauri build
```

Tests must cover the happy path, edge cases, invalid input, and cleanup or
failure behavior where applicable. Until automated tests exist, document
repeatable manual steps and their observed results.

Build success is not proof that PTY behavior works. Manually verify creating,
switching, resizing, and closing tabs; entering input; rendering output; cwd
changes; file-tree navigation; copy and paste; and application shutdown.

## 11. Definition of Done

A change is complete only when:

- Acceptance criteria are met
- Relevant checks pass
- Supported-platform behavior is verified
- Unverified behavior is disclosed
- Documentation is updated when setup or behavior changes
- The working tree contains no unrelated or generated files

Do not describe a change as complete when validation is failing or was skipped.
