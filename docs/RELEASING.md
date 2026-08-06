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

## 4. Build with signing

```bash
export TAURI_SIGNING_PRIVATE_KEY="<path to the .key file>"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="<passphrase>"
pnpm tauri build
```

Pass the **path**, not the key contents — that keeps the key out of your shell
history and out of the environment of every child process.

Without these variables the build still succeeds and produces a perfectly
installable `.exe`, but stops before signing with *"A public key has been found,
but no private key"*. An unsigned installer is fine for manual installation; it
just cannot be delivered through auto-update.

Artifacts land in `src-tauri/target/release/bundle/nsis/`.

## 5. Generate `latest.json` by hand

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

## 6. Publish — all three assets

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

## 7. Verify the live endpoint

Building the file is not the same as it being reachable. Check the real URL:

```bash
curl -sL "https://github.com/Albert0x/Brace/releases/latest/download/latest.json"
```

Confirm the version matches, the download URL points at the new tag, and
`signature` is non-empty. A broken update channel is silent — nobody reports it,
they just stop receiving updates.
