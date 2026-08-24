# Releasing

Windows only. Every step below has bitten us at least once — the notes explain
why, not just what.

## Before you start

You need the **updater signing key**. It is not in this repository and must
never be committed here: this repo is public, and anyone holding that key can
sign an update package that every installed client will accept and install
automatically. It lives outside any git working tree; check your local notes or
password manager for the path and passphrase.

> **Never generate a fresh key pair to "fix" a lost key.** The public key is
> baked into `src-tauri/tauri.conf.json`. Replace it and every existing
> installation loses auto-update permanently — their client cannot verify
> anything signed by the new key, and each user has to reinstall by hand. If the
> key is genuinely lost, ship a transition release signed with the *old* key
> that tells users to reinstall, and only then rotate.

## Rotating the signing key

Only if the key is compromised. It costs users, so don't do it casually.

The public key is compiled into the app, which means **users can only verify an
update with the key their currently installed version knows about**. A direct
swap breaks every existing installation. The way through is a transition
release that is signed with the *old* key but ships the *new* public key:

1. Generate the new pair. Run this yourself — a password passed on a command
   line ends up in shell history and in any transcript:

   ```bash
   pnpm tauri signer generate -w <path outside any git tree>
   ```

2. Put the new public key in `tauri.conf.json` and bump the version.

3. **Build that release with the OLD key.** This is the step that matters:

   ```bash
   export TAURI_SIGNING_PRIVATE_KEY="<path to the OLD key>"
   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="<old passphrase>"
   pnpm tauri build
   ```

   Installed clients hold the old public key, so only an old-key signature
   verifies. The binary they install carries the new public key, and from then
   on they trust it.

4. Publish, and **leave that version up long enough for people to pick it up**.

5. Every release after it signs with the new key.

Signing the transition release with the *new* key is the failure mode to avoid:
every existing installation fails verification at once, the update channel goes
dead, and the only fix is asking each user to reinstall by hand.

Anyone who never installs the transition release is stranded the same way —
their client only ever trusts the old key. Keep the old key until you're
satisfied the long tail has moved.

## 0. Manual verification — run this before every release

CI covers types, lint, and pure logic. It cannot cover PTY lifecycle, ConPTY
behaviour, DPAPI, or real `git` remotes. Everything below has to be done by hand
in a real build (`pnpm tauri dev` is enough unless noted).

Do not skip a section because the change "looked small". Each of these
corresponds to a bug that shipped at least once.

### Terminal core

- [ ] Type in a tab, get output. Print Chinese text and an emoji — no mojibake.
      (Exercises the decode + output-buffering path.)
- [ ] Type Chinese through an IME — no duplicated or dropped characters.
- [ ] Run something noisy (`cargo build`, `pnpm install`, `type` a large file).
      Output should stay smooth and **complete** — check the tail is not
      truncated. This is the 16ms output aggregation window.
- [ ] Type `exit`. The tab must show `[process exited with code 0]`, the tab
      title dims, its dot goes hollow. Press **Enter** — a new shell starts **in
      the same directory**, and earlier output is still on screen.
      *(ConPTY does not return EOF on client exit; this only works because a
      separate thread polls `try_wait`. If the exit notice never appears, that
      mechanism regressed.)*
- [ ] After `exit` but before restarting, type random characters — they must be
      swallowed, not echoed into a dead pipe.
- [ ] Open several tabs, close a **non-active** one, then press `Ctrl+W`. The
      closed tab must not come back.
- [ ] Close and reopen the app — the previous tab group returns with each tab's
      shell and directory.

### Search, scrollback, layout

- [ ] `Ctrl+F`, type something present on screen: matches get highlighted, the
      counter shows `n/m`, `Enter` jumps forward, `Shift+Enter` backward.
- [ ] Search for something absent — the box turns red and reads "no match".
- [ ] Change scrollback in Settings, print more lines than the old limit, scroll
      back and confirm the history is there.
- [ ] Drag the sidebar edge. Release — the terminal reflows to the new width
      (no clipped or blank columns).

### Git panel

- [ ] Commit a file. Then **make the push fail** (disconnect the network, or
      point at a remote you cannot reach) and hit **Commit & Push**.
      The message must be amber and say the commit succeeded locally — *not* a
      plain red failure. Then verify with `git log` that exactly **one** commit
      was created.
- [ ] In a repo needing credentials, hit push. It must **fail quickly with a
      reason**, not hang the panel. (`GIT_TERMINAL_PROMPT=0`.)
- [ ] Toggle **Git decorations** off in a repo with `node_modules`. The tree
      stops showing ignored files, and `git status --ignored` stops being polled.

### Profiles and secrets

- [ ] Create a profile with an API key, save, reopen — the field reads "saved",
      not the value.
- [ ] **Rename** that variable, save, reopen: it must read "not set", and
      `%APPDATA%\com.brace.dev\profiles.json` must no longer contain the old
      ciphertext. Rename means the old secret is gone.
- [ ] Rename it **back** to the original name — it must still read "not set".
      (The old value must not silently reattach.)
- [ ] Confirm the panel's encryption notice matches reality: it must not claim
      DPAPI encryption unless values on disk actually carry the `enc:` prefix.
- [ ] Switch profiles, open a new tab, and check the injected variables inside
      it (`echo $env:ANTHROPIC_BASE_URL`).

### Misc

- [ ] Copy an image to the clipboard, press `Ctrl+Shift+V` in a terminal — a
      hint about `Alt+V` appears instead of nothing happening. Repeat via the
      right-click menu.
- [ ] Pick a background image larger than 8MB — it is rejected with a visible
      reason, and the previous background is still in place.
- [ ] With Claude running, the usage row shows a context percentage. If it
      cannot be read it must show a dash with a tooltip, **never a bare 0%**.
- [ ] Both languages: switch to English and back, spot-check the panels.

### Packaged build

- [ ] `pnpm tauri build` succeeds and produces
      `Brace_x.y.z_x64-setup.exe`. Required whenever `[lib] name`,
      `[package] name`, bundle config, or dependencies changed.
- [ ] Install it and repeat the first item of **Terminal core** — a dev build
      passing is not proof the packaged one works.

## 1. Bump the version — three files, all must agree

```
package.json              "version": "x.y.z"
src-tauri/Cargo.toml      version = "x.y.z"
src-tauri/tauri.conf.json "version": "x.y.z"
```

The updater compares against `tauri.conf.json`; the About panel shows what it
reads at runtime. If they disagree, users get told they're up to date when they
aren't, or the reverse.

Run `cargo check --manifest-path src-tauri/Cargo.toml` so `Cargo.lock` picks up
the new version, then commit all four files.

## 2. Merge through a PR

Open a `chore/release-x.y.z` branch and let CI run. Merge commits are disabled
on this repository — use rebase.

## 3. Tag

```bash
git checkout main && git pull
git tag -a vx.y.z -m "Brace vx.y.z"
git push origin vx.y.z
```

## 4. The tag triggers the release workflow

Pushing the tag runs `.github/workflows/release.yml` on a Windows runner. It
checks the three version numbers against the tag, builds, signs, uploads the
installer, its `.sig`, and `latest.json`, then opens the release **as a draft**.

Only plain version tags trigger it — the pattern is `v[0-9]+.[0-9]+.[0-9]+`.
Pre-release tags such as `v0.1.8-macos-preview.1` are published by hand and
deliberately excluded: the version check would fail against them by definition,
and it fails *after* the draft has been created, leaving a stray draft nobody
wants to be the one to delete.

Two one-time prerequisites:

| Repository secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | The **contents** of the `.key` file, not a path. The runner has no such file. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The passphrase |

Set them under *Settings → Secrets and variables → Actions*. The key still never
enters this repository — a secret is not a commit.

The draft is deliberate. Drafts do not participate in `releases/latest`, so the
updater cannot serve one to anybody until you review the assets and press
**Publish** yourself. Nothing reaches users on a tag push alone.

The workflow fails loudly if `latest.json`, the installer, or the `.sig` is
missing. That check exists because the Tauri CLI does **not** produce
`latest.json` on its own (see step 6) — if the action ever stops filling that
gap, a silent 404 on the update endpoint is the failure mode, and this turns it
into a red pipeline instead.

**The first time you release through the workflow**, still run step 8 by hand
against the published release before trusting it.

## Manual fallback

The steps below are what the workflow automates. Use them when the workflow is
unavailable, or when you need a local build for some other reason.

### 5. Build with signing

```bash
export TAURI_SIGNING_PRIVATE_KEY="<path to the .key file>"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="<passphrase>"
pnpm tauri build
```

Pass the **path**, not the key contents — that keeps the key out of your shell
history and out of the environment of every child process.

Without these variables the bundle is still produced, but the command **exits
non-zero** with *"A public key has been found, but no private key"*. The
installer it leaves behind is perfectly usable for manual installation — it just
cannot be delivered through auto-update.

That non-zero exit is deliberate and useful: if the signing secrets are missing
in CI, the release workflow fails loudly instead of quietly publishing an
installer that no existing client can verify.

Artifacts land in `src-tauri/target/release/bundle/nsis/`.

### 6. Generate `latest.json` by hand

**Tauri 2 does not produce this file.** Tauri 1 did, which is exactly why it is
easy to forget. Without it the updater endpoint 404s.

```bash
python - <<'PY'
import io, json, datetime
B = 'src-tauri/target/release/bundle/nsis/'
VERSION = 'x.y.z'
sig = io.open(B + f'Brace_{VERSION}_x64-setup.exe.sig', encoding='utf-8').read().strip()
data = {
    "version": VERSION,
    "notes": "one-line summary shown in the update prompt",
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
    "platforms": {
        "windows-x86_64": {
            "signature": sig,
            "url": f"https://github.com/Albert0x/Brace/releases/download/v{VERSION}/Brace_{VERSION}_x64-setup.exe",
        }
    },
}
io.open(B + 'latest.json', 'w', encoding='utf-8', newline='\n').write(json.dumps(data, indent=2, ensure_ascii=False) + "\n")
PY
```

### 7. Publish — all three assets

```bash
gh release create vx.y.z --title "Brace vx.y.z" --notes-file notes.md \
  src-tauri/target/release/bundle/nsis/Brace_x.y.z_x64-setup.exe \
  src-tauri/target/release/bundle/nsis/Brace_x.y.z_x64-setup.exe.sig \
  src-tauri/target/release/bundle/nsis/latest.json
```

The endpoint configured in `tauri.conf.json` is:

```
https://github.com/Albert0x/Brace/releases/latest/download/latest.json
```

`releases/latest` resolves to whichever release is newest, so **publishing a
release without `latest.json` breaks auto-update for everyone** — the request
404s and every client's update check fails. That is worse than not releasing at
all. If you need to publish something incomplete, mark it as a **pre-release**:
GitHub excludes those from `latest`, so the updater never sees it.

### 8. Verify the live endpoint

Building the file is not the same as it being reachable. Check the real URL:

```bash
curl -sL "https://github.com/Albert0x/Brace/releases/latest/download/latest.json"
```

Confirm the version matches, the download URL points at the new tag, and
`signature` is non-empty. A broken update channel is silent — nobody reports it,
they just stop receiving updates.
