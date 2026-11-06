# Releasing with automatic updates

The desktop app updates itself with Tauri's updater plugin
(`apps/desktop/src-tauri/src/updates.rs`). It reads

    https://github.com/pankaj1980patel/my-domain/releases/latest/download/latest.json

from the newest **published** release, and installs a download only if its
signature matches the public key built into the app. The release workflow
(`.github/workflows/desktop-release.yml`) signs the update files and writes
`latest.json`. For the rest of the release process (versions, what each file
is, unsigned apps) see [release.md](release.md).

## One-time setup: the signing key

Do this once, on your own computer. The private key never goes into the repo.

1. Generate the key pair (choose a password when asked, and keep it):

   ```bash
   cd apps/desktop
   npx tauri signer generate -w ~/.tauri/my-domain.key
   ```

   This writes `~/.tauri/my-domain.key` (private) and
   `~/.tauri/my-domain.key.pub` (public).

2. Put the **public** key into the app's config. Open
   `apps/desktop/src-tauri/tauri.conf.json` and replace the placeholder in
   `plugins > updater > pubkey`:

   ```json
   "plugins": {
     "updater": {
       "pubkey": "REPLACE_WITH_UPDATER_PUBLIC_KEY",
   ```

   with the whole one-line contents of `~/.tauri/my-domain.key.pub`
   (`cat ~/.tauri/my-domain.key.pub`; it starts with `dW50cnVzdGVk`). Commit
   that change. `cargo test -p my-domain config_sanity` checks it's a public
   key and not a private one.

   (Instead of committing it you can set a repository **variable**
   `TAURI_UPDATER_PUBKEY` with the same text: the workflow uses it over the
   file. Keeping it in `tauri.conf.json` is simpler.)

3. Add two repository **secrets** (GitHub → Settings → Secrets and variables →
   Actions → New repository secret):

   - `TAURI_SIGNING_PRIVATE_KEY`: the contents of `~/.tauri/my-domain.key`
     (`cat ~/.tauri/my-domain.key`)
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the password from step 1 (an
     empty secret isn't allowed; if you chose no password, leave this one out)

4. Back up `~/.tauri/my-domain.key` and its password somewhere safe. If
   they're lost, installed apps can't take updates any more: you'd have to
   ship a new key and everyone would reinstall by hand.

Without the secret or with the placeholder still in place, the workflow
still builds everything; it prints a warning, makes no update files and no
`latest.json`, and those apps say "This build can't update itself".

## Each release

1. Bump the version in `tauri.conf.json`, `src-tauri/Cargo.toml` and the
   Android `build.gradle.kts` (see [release.md](release.md)), commit, push.
2. Tag with an **annotated** tag; its message becomes the release notes the
   app shows under "What's new":

   ```bash
   git tag -a v0.2.0 -m "Faster file transfers. Fixes reconnecting after sleep."
   git push origin v0.2.0
   ```

3. "App release" builds macOS (Apple silicon and Intel), Windows and Linux,
   signs each update file (`.app.tar.gz`, `-setup.exe`, `.msi`, `.AppImage`,
   `.deb`, each with a `.sig`), and the release job writes `latest.json`
   (`scripts/updater-manifest.py`) and attaches everything to a **draft**
   release.
4. Check the draft has `latest.json` and the `.sig` files, then **publish**
   it. Only then does `releases/latest/download/latest.json` point at it, and
   installed apps see it within 6 hours (or at once with Settings → Updates →
   Check now). Pre-releases are never offered: the app takes stable versions
   only, and GitHub's "latest" skips releases marked pre-release.

## How the app behaves

- Checks 30 s after start, then every 6 h; debug builds never check.
- A new version shows a banner (What's new / Install and restart / Later).
  Nothing installs by itself unless Settings → Updates → "Install updates
  automatically" is on; then it downloads in the background and installs on
  the next quit.
- Network or server errors only show in Settings → Updates.
- Windows installs with the installer's passive mode (a progress bar, no
  questions); macOS replaces the .app (asking for a password only if it
  can't write there); the AppImage replaces itself; a .deb asks for a
  password through pkexec.

## Rotating the key

Generate a new pair, put the new public key in `tauri.conf.json`, replace both
secrets and release. Apps already installed only trust the old key, so sign
one release with the **old** key that carries the **new** public key first,
then switch the secrets.
