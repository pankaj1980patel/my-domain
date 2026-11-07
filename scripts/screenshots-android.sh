#!/usr/bin/env bash
# Screenshots of every Android screen, with SAMPLE data only.
#
# Needs a DEBUG build of the app installed on a phone connected over adb
# (this script never builds, installs, uninstalls, grants permissions or
# touches the lock screen: unlock the phone first). Each screen is opened
# with the debug-only launch extras (MainActivity.debugLaunch):
#
#   --ez md_sample true        made-up devices, in memory only (SampleData.kt)
#   --es md_screen <name>      the screen (DebugScreens.kt)
#   --es md_device <id>        a device's page
#   --es md_topic / md_query   a Help topic / what Search starts with
#   --es md_theme light|dark   the app's theme for now (not saved)
#
# and saved as screenshots/android/<NNN>-<name>-<theme>.png. Long pages are
# scrolled with a swipe and shot again. The system's dark mode
# (`cmd uimode night`) follows the theme for the status bar, and is put
# back as it was at the end. Afterwards the app is opened once without
# the extras, which ends sample mode (the real devices come back).
#
# Usage:
#   scripts/screenshots-android.sh               all of them
#   scripts/screenshots-android.sh device        only shots whose name contains "device"
#   ANDROID_SERIAL=<serial> scripts/screenshots-android.sh

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/screenshots/android"
PKG="com.mydomain.android"
ACTIVITY="$PKG/.MainActivity"
ONLY="${1:-}"
WAIT="${WAIT:-2.5}"

command -v adb >/dev/null || { echo "adb not found" >&2; exit 1; }
[ "$(adb get-state 2>/dev/null)" = device ] || { echo "No phone connected over adb." >&2; exit 1; }
adb shell pm path "$PKG" >/dev/null 2>&1 || { echo "$PKG isn't installed (install a debug build first)." >&2; exit 1; }
if ! adb shell dumpsys package "$PKG" | grep -q "DEBUGGABLE"; then
  echo "$PKG is a release build: the screenshot extras only work in debug builds." >&2
  exit 1
fi

# The system's dark mode now, to put back after.
NIGHT_WAS="$(adb shell cmd uimode night | tr -d '\r' | awk '{print $NF}')"
restore() {
  [ -n "${NIGHT_WAS:-}" ] && adb shell cmd uimode night "$NIGHT_WAS" >/dev/null 2>&1 || true
  # Open the app normally once: sample mode ends, the real devices come back.
  adb shell am start -W --activity-clear-task -n "$ACTIVITY" >/dev/null 2>&1 || true
}
trap restore EXIT

mkdir -p "$OUT"
[ -n "$ONLY" ] || rm -f "$OUT"/*.png

# The screen's size, for the scrolling swipe.
SIZE="$(adb shell wm size | tr -d '\r' | awk '/size/ {print $NF}' | tail -1)"
W="${SIZE%x*}"
H="${SIZE#*x}"

N=0
THEME=dark
# shot <name> <extras...>: open the screen and save it.
shot() {
  local name="$1"
  shift
  N=$((N + 1))
  local file
  file="$(printf '%03d' "$N")-$name-$THEME.png"
  if [ -n "$ONLY" ] && [[ "$name" != *"$ONLY"* ]]; then return; fi
  adb shell am start -W --activity-clear-task -n "$ACTIVITY" \
    --ez md_sample true --es md_theme "$THEME" "$@" >/dev/null
  sleep "$WAIT"
  adb exec-out screencap -p >"$OUT/$file"
  echo "  $file"
}

# scrolled <name> <pages> <extras...>: the screen, then a swipe further
# down each time.
scrolled() {
  local name="$1" pages="$2"
  shift 2
  shot "$name" "$@"
  if [ -n "$ONLY" ] && [[ "$name" != *"$ONLY"* ]]; then
    N=$((N + pages))
    return
  fi
  local i
  for i in $(seq 1 "$pages"); do
    N=$((N + 1))
    adb shell input swipe "$((W / 2))" "$((H * 3 / 4))" "$((W / 2))" "$((H / 4))" 400
    sleep 1
    local file
    file="$(printf '%03d' "$N")-$name-$i-$THEME.png"
    adb exec-out screencap -p >"$OUT/$file"
    echo "  $file"
  done
}

all_screens() {
  shot sign-in --es md_screen signIn
  shot home --es md_screen home
  scrolled device-mac 5 --es md_screen device --es md_device sample-macbook
  scrolled device-mac-mini 3 --es md_screen device --es md_device sample-mac-mini
  scrolled device-linux 3 --es md_screen device --es md_device sample-lab-laptop
  shot device-windows-asleep --es md_screen device --es md_device sample-office-pc
  shot links-map --es md_screen linksMap
  shot health --es md_screen health
  shot onboarding --es md_screen onboarding
  shot notes --es md_screen notes
  shot timers --es md_screen timers
  shot read-later --es md_screen readLater
  shot activity --es md_screen activity
  shot transfers --es md_screen transfers
  shot search --es md_screen search --es md_query pass
  scrolled settings 6 --es md_screen settings
  shot automations --es md_screen automations
  shot setup-phone --es md_screen setup
  shot help --es md_screen help
  shot help-topic --es md_screen help --es md_topic getting-started
  shot developer --es md_screen developer
}

echo "Taking screenshots into $OUT ..."
for THEME in dark light; do
  if [ "$THEME" = dark ]; then adb shell cmd uimode night yes >/dev/null; else adb shell cmd uimode night no >/dev/null; fi
  all_screens
done

echo "Done: $(find "$OUT" -name '*.png' | wc -l | tr -d ' ') screenshots, $(du -sh "$OUT" | cut -f1)."
