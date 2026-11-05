# Releasing the apps

`.github/workflows/desktop-release.yml` ("App release") builds the
installable desktop apps and the Android APK.

- **Run it by hand** (Actions > App release > Run workflow, any branch): the
  files are attached to the run as downloadable artifacts. No release is made.
- **Push a tag `vX.Y.Z`**: the same build, plus a **draft** GitHub Release with
  every file attached. Nothing is public until you publish the draft.

The server binary has its own workflow (`server-release.yml`), which also
attaches its file to the release of a `v*` tag.

## Cutting a release

1. Bump the version in all three places (they must match):
   - `apps/desktop/src-tauri/tauri.conf.json` — `"version"` (the source of truth)
   - `apps/desktop/src-tauri/Cargo.toml` — `version` (then run `cargo check -p my-domain` so `Cargo.lock` follows)
   - `apps/android/app/build.gradle.kts` — `versionName` (and raise `versionCode` by one)
2. Commit and push to `main`.
3. Tag that commit and push the tag:
   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```
4. Wait for "App release" to finish, open the draft on the Releases page,
   write the notes, and publish it.

The first job checks that the tag is `v` + the `tauri.conf.json` version and
that the desktop `Cargo.toml` and the Android `versionName` agree; if not, it
fails with a message saying which one is off and nothing is built. To fix a
wrong tag: `git push --delete origin vX.Y.Z && git tag -d vX.Y.Z`, fix the
versions, and tag again.

## What gets built

| File | Platform | What it is |
| --- | --- | --- |
| `my-domain_<ver>_macos_aarch64.dmg` | macOS, Apple silicon | Disk image: open it and drag my-domain to Applications. |
| `my-domain_<ver>_macos_aarch64.app.tar.gz` | macOS, Apple silicon | The same `.app`, zipped as a tarball (for scripts or manual install). |
| `my-domain_<ver>_linux_x86_64.AppImage` | Linux x86_64 | Single file, runs on most distributions: `chmod +x` and run it. |
| `my-domain_<ver>_linux_amd64.deb` | Debian/Ubuntu x86_64 | `sudo apt install ./my-domain_<ver>_linux_amd64.deb` |
| `my-domain_<ver>_windows_x64.msi` | Windows x64 | WiX installer (good for managed/silent installs). |
| `my-domain_<ver>_windows_x64-setup.exe` | Windows x64 | NSIS installer (the usual "download and double-click"). |
| `my-domain_<ver>_android-debug.apk` | Android | Debug build, see below. |

On a manual run each platform's files are in their own artifact
(`my-domain-<ver>-macos-aarch64`, `-linux-x86_64`, `-windows-x86_64`,
`-android`); GitHub hands them out as zip files.

Notes:

- **No Intel Mac build.** The macOS runner is Apple silicon, and the desktop
  app links OpenSSL (FCM receiver: `fcm_receiver_rs` → `ece` → `openssl`);
  cross-building for `x86_64-apple-darwin` would need an x86_64 OpenSSL too.
  Add one later with a `macos-13` (Intel) runner in the matrix if needed.
- **macOS OpenSSL is linked statically** (`OPENSSL_STATIC=1` against
  Homebrew's `openssl@3`), so the app does not need Homebrew on the user's Mac.
- **Linux is built on Ubuntu 22.04** so the AppImage runs on distributions
  with glibc 2.35 or newer. Both Linux files need WebKitGTK 4.1 (Ubuntu 22.04+,
  Debian 12+, Fedora 37+). The `.deb` declares Tauri's standard dependencies;
  it also uses ALSA and D-Bus, which any desktop install already has.
- **Builds from CI have no Firebase config.** `google-services*.json` and
  `fcm-sa.json` are git-ignored and not in CI, so the APK is built without FCM
  ids (the same as CI's own build). Wire them in as secrets before shipping
  builds that need push signaling.

## Windows and OpenSSL

The desktop crate needs OpenSSL, which Windows doesn't have. Like CI's Windows
job, the workflow installs it from the runner's vcpkg and points the build at
it:

```powershell
vcpkg install openssl:x64-windows-static-md
echo "VCPKG_ROOT=$env:VCPKG_INSTALLATION_ROOT" >> $env:GITHUB_ENV
```

`static-md` = OpenSSL linked statically into the app, with the dynamic MSVC
runtime (what Rust uses by default), so the installed app needs no OpenSSL
DLLs. Building on your own Windows machine needs the same: install vcpkg, run
that `vcpkg install`, and set `VCPKG_ROOT`.

## Unsigned builds

Nothing is code-signed yet; the repo has no signing secrets.

**macOS.** The app is neither signed nor notarized, so Gatekeeper says it
"can't be opened" or "is damaged". Either:

- In Finder, right-click (or Control-click) my-domain in Applications → Open →
  Open. On macOS 15+, if there's no Open button: try to open it once, then
  System Settings → Privacy & Security → "Open Anyway".
- Or remove the quarantine flag in Terminal:
  ```bash
  xattr -dr com.apple.quarantine /Applications/my-domain.app
  ```

Unsigned apps also get re-asked for permissions (Bluetooth, microphone,
location) after each update, because macOS can't tell the new build is the
same app.

**Windows.** SmartScreen shows "Windows protected your PC": click More info →
Run anyway. Some antivirus tools are suspicious of unsigned installers.

**Linux.** No signing needed. For the AppImage: `chmod +x` it first; on
distributions without FUSE 2, run it with `--appimage-extract-and-run`.

**Android.** The APK is the **debug** build, signed with the Android debug key
(the one Gradle generates on the CI runner, different on every run). Users
must allow "Install unknown apps" for their browser/file manager. Because the
key changes between runs, an APK from one run can't update an install from
another: uninstall first (this deletes the app's data). A proper release
keystore (`assembleRelease` with a signing config fed from secrets) is a later
step.

## Turning on signing later

Tauri's CLI reads these environment variables during `tauri build`; add them as
repository secrets and pass them as `env:` to the "Build and bundle" step.

macOS (needs an Apple Developer account, $99/year):

- `APPLE_CERTIFICATE` — the "Developer ID Application" certificate exported as
  `.p12`, base64-encoded
- `APPLE_CERTIFICATE_PASSWORD` — the `.p12` password
- `APPLE_SIGNING_IDENTITY` — e.g. `Developer ID Application: Name (TEAMID)`
- Notarization, either `APPLE_ID` + `APPLE_PASSWORD` (an app-specific
  password) + `APPLE_TEAM_ID`, or an App Store Connect API key:
  `APPLE_API_ISSUER` + `APPLE_API_KEY` + `APPLE_API_KEY_PATH`

Windows: an OV/EV code-signing certificate (or Azure Trusted Signing). Set
`bundle.windows.certificateThumbprint` / `digestAlgorithm` / `timestampUrl` in
`tauri.conf.json` with the certificate imported on the runner, or a
`bundle.windows.signCommand` that calls your signing tool.

Android: a release keystore, base64-encoded as a secret, plus its passwords
and key alias; decode it on the runner and add a `signingConfigs.release` to
`apps/android/app/build.gradle.kts` that reads them from the environment.

If the app ever gets Tauri's updater, the updater key (`TAURI_SIGNING_PRIVATE_KEY`
and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) is a separate secret.
