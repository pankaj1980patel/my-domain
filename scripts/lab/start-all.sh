#!/usr/bin/env bash
# Start my-domain on every lab device, logged in with the test account from
# test-creds.env: the macOS desktop here, the Linux desktop on the laptop, and
# the Android app on the phone attached to it.
#   scripts/lab/start-all.sh
# Logs: macOS  -> $MD_REPO_ROOT/target/my-domain-mac.log
#       Linux  -> ~/my-domain-desktop.log on the laptop
#       Android-> scripts/lab/android.sh logcat
source "$(dirname "$0")/common.sh"

[[ -n "${MD_DEV_USER:-}" ]] || { echo "no test account: create scripts/lab/test-creds.env"; exit 1; }

say "macOS desktop"
PIDF="$MD_REPO_ROOT/target/my-domain-mac.pid"
[[ -f "$PIDF" ]] && { pkill -P "$(cat "$PIDF")" 2>/dev/null; kill "$(cat "$PIDF")" 2>/dev/null; sleep 1; } || true
(cd "$MD_REPO_ROOT/apps/desktop" && npm install --silent &&
  MD_DEV_SERVER="$MD_DEV_SERVER" MD_DEV_USER="$MD_DEV_USER" MD_DEV_PASS="$MD_DEV_PASS" MD_DEV_KEY="$MD_DEV_KEY" \
  nohup npm run tauri dev > "$MD_REPO_ROOT/target/my-domain-mac.log" 2>&1 < /dev/null & echo $! > "$PIDF")

say "Linux desktop"
"$(dirname "$0")/linux.sh" start

say "Android"
"$(dirname "$0")/android.sh" start
say "All started."
