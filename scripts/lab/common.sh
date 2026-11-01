#!/usr/bin/env bash
# Shared settings for the device-lab scripts. Sourced, not executed.
#
# Override any of these per shell, e.g.
#   MD_ARCH_HOST=md-arch scripts/lab/linux.sh test
set -euo pipefail

MD_ARCH_HOST="${MD_ARCH_HOST:-pankaj@192.168.0.176}"
MD_ARCH_DIR="${MD_ARCH_DIR:-my-domain}"            # relative to the remote $HOME
MD_ANDROID_SERIAL="${MD_ANDROID_SERIAL:-10BF5N0JQH000ZA}"
MD_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Environment every remote command runs with. JDK 17 is what the Android
# Gradle plugin in apps/android supports (the host's default JDK is newer).
MD_REMOTE_ENV='export ANDROID_HOME="$HOME/Android/Sdk";
export JAVA_HOME="${MD_JAVA_HOME:-/usr/lib/jvm/java-17-openjdk}";
export PATH="$HOME/.cargo/bin:$ANDROID_HOME/platform-tools:$ANDROID_HOME/cmdline-tools/latest/bin:$PATH";'

say() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }

# Run a command on the arch host inside the repo checkout.
remote() {
  ssh -o ConnectTimeout=10 "$MD_ARCH_HOST" "$MD_REMOTE_ENV cd \"\$HOME/$MD_ARCH_DIR\" && $*"
}
