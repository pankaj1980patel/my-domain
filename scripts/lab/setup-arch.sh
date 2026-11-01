#!/usr/bin/env bash
# One-time toolchain setup for the arch host. Idempotent: re-running only
# installs what is missing. Run it FROM THE MAC: scripts/lab/setup-arch.sh
# (it will ask for your sudo password on the laptop for pacman).
#
# Installs: JDK 17 (for the Android Gradle plugin), Android cmdline-tools +
# platform/build-tools + the NDK version apps/android pins, the Rust Android
# targets, rsync, and the Tauri/WebKit build deps for the Linux desktop app.
source "$(dirname "$0")/common.sh"

NDK_VERSION="$(grep -oE 'ndkVersion = "[^"]+"' "$MD_REPO_ROOT/apps/android/app/build.gradle.kts" | cut -d'"' -f2)"
CMDLINE_TOOLS_ZIP="commandlinetools-linux-${MD_CMDLINE_TOOLS_VERSION:-13114758}_latest.zip"

say "Setting up $MD_ARCH_HOST (NDK $NDK_VERSION)"

# Separate ssh -t call: sudo needs a real terminal for the password prompt,
# and the heredoc below occupies stdin.
say "pacman packages (asks for sudo password)"
ssh -t "$MD_ARCH_HOST" "sudo pacman -S --needed --noconfirm jdk17-openjdk rsync unzip \
  webkit2gtk-4.1 gtk3 libayatana-appindicator librsvg xdotool base-devel nodejs npm"

ssh "$MD_ARCH_HOST" bash -s <<EOF
set -euo pipefail
$MD_REMOTE_ENV

echo "--- Android cmdline-tools"
if [ ! -x "\$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" ]; then
  mkdir -p "\$ANDROID_HOME/cmdline-tools"
  tmp=\$(mktemp -d)
  curl -fsSL "https://dl.google.com/android/repository/$CMDLINE_TOOLS_ZIP" -o "\$tmp/tools.zip"
  unzip -q "\$tmp/tools.zip" -d "\$tmp"
  mv "\$tmp/cmdline-tools" "\$ANDROID_HOME/cmdline-tools/latest"
  rm -rf "\$tmp"
fi

echo "--- SDK packages"
yes | sdkmanager --licenses >/dev/null || true
sdkmanager "platform-tools" "platforms;android-35" "build-tools;35.0.0" "ndk;$NDK_VERSION"

echo "--- Rust Android targets"
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android

echo "--- Persist env for login shells"
grep -q 'ANDROID_HOME' ~/.profile 2>/dev/null || cat >> ~/.profile <<'PROFILE'
# my-domain Android build env (added by scripts/lab/setup-arch.sh)
export ANDROID_HOME="\$HOME/Android/Sdk"
export PATH="\$ANDROID_HOME/platform-tools:\$ANDROID_HOME/cmdline-tools/latest/bin:\$PATH"
PROFILE

echo "--- Done. Versions:"
java -version 2>&1 | head -1
"\$JAVA_HOME/bin/java" -version 2>&1 | head -1
sdkmanager --list_installed | sed -n '1,20p'
adb devices -l
EOF
