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
#   --es md_tab <tab>          a device page's tab (overview, messages,
#                              files, control, media, settings), the Files
#                              hub's (files, transfers, sync), Messages'
#                              (chats, notifications) or a Settings group
#                              (account, phone, files, automation, battery,
#                              helper, health, backup, appearance)
#   --es md_topic / md_query   a Help topic / what Search starts with
#   --es md_theme light|dark   the app's theme for now (not saved)
#
# Only sample data may show: Do Not Disturb goes on first (no heads-up
# notification from a real app lands in a shot), the shade is collapsed,
# and the status bar's demo mode hides the real notification icons. All
# three are put back as they were at the end, even after a failure.
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

# The system's dark mode, Do Not Disturb and status-bar demo mode now, to
# put back after.
NIGHT_WAS="$(adb shell cmd uimode night | tr -d '\r' | awk '{print $NF}')"
ZEN_WAS="$(adb shell settings get global zen_mode | tr -d '\r')"
DEMO_WAS="$(adb shell settings get global sysui_demo_allowed | tr -d '\r')"
restore() {
  [ -n "${NIGHT_WAS:-}" ] && adb shell cmd uimode night "$NIGHT_WAS" >/dev/null 2>&1 || true
  adb shell am broadcast -a com.android.systemui.demo -e command exit >/dev/null 2>&1 || true
  case "${DEMO_WAS:-}" in
    1) ;;
    0) adb shell settings put global sysui_demo_allowed 0 >/dev/null 2>&1 || true ;;
    *) adb shell settings delete global sysui_demo_allowed >/dev/null 2>&1 || true ;;
  esac
  # zen_mode: 0 off, 1 priority only, 2 total silence, 3 alarms only.
  case "${ZEN_WAS:-}" in
    1) adb shell cmd notification set_dnd priority >/dev/null 2>&1 || true ;;
    2) adb shell cmd notification set_dnd none >/dev/null 2>&1 || true ;;
    3) adb shell cmd notification set_dnd alarms >/dev/null 2>&1 || true ;;
    *) adb shell cmd notification set_dnd off >/dev/null 2>&1 || true ;;
  esac
  # Open the app normally once: sample mode ends, the real devices come back.
  adb shell am start -W --activity-clear-task -n "$ACTIVITY" >/dev/null 2>&1 || true
}
trap restore EXIT
trap 'exit 130' INT TERM

# Nothing real on screen: no heads-up notifications, the shade shut, and
# the status bar without the real notification icons (a fixed clock too).
adb shell cmd notification set_dnd priority >/dev/null
adb shell cmd statusbar collapse >/dev/null 2>&1 || true
adb shell settings put global sysui_demo_allowed 1 >/dev/null
demo() { adb shell am broadcast -a com.android.systemui.demo -e command "$@" >/dev/null 2>&1 || true; }
# Again before each shot: some phones drop parts of demo mode on the way.
demo_bar() {
  demo enter
  demo clock -e hhmm 0941
  demo notifications -e visible false
  demo battery -e level 84 -e plugged false
}

mkdir -p "$OUT"
[ -n "$ONLY" ] || rm -f "$OUT"/*.png

# The screen's size, for the scrolling swipe.
SIZE="$(adb shell wm size | tr -d '\r' | awk '/size/ {print $NF}' | tail -1)"
W="${SIZE%x*}"
H="${SIZE#*x}"

# A checksum of the screen below the status bar (raw pixels, 4 bytes
# each; the top eighth skipped).
app_sum() { adb exec-out screencap | tail -c +$((16 + W * 4 * (H / 8))) | cksum; }

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
  demo_bar
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
  local i last now file
  last="$(app_sum)"
  for i in $(seq 1 "$pages"); do
    N=$((N + 1))
    adb shell input swipe "$((W / 2))" "$((H * 3 / 4))" "$((W / 2))" "$((H / 4))" 400
    sleep 1
    # The page ended (the app looks the same, whatever the status bar
    # did): no more shots of it.
    now="$(app_sum)"
    if [ "$now" = "$last" ]; then
      N=$((N + pages - i))
      return
    fi
    last="$now"
    file="$OUT/$(printf '%03d' "$N")-$name-$i-$THEME.png"
    adb exec-out screencap -p >"$file"
    echo "  $(basename "$file")"
  done
}

all_screens() {
  shot sign-in --es md_screen signIn
  # The four bottom tabs.
  scrolled home 2 --es md_screen home
  shot devices --es md_screen devices
  scrolled activity 1 --es md_screen activity
  shot more --es md_screen more
  # A device page, each of its tabs.
  scrolled device-mac 1 --es md_screen device --es md_device sample-macbook --es md_tab overview
  scrolled device-mac-messages 1 --es md_screen device --es md_device sample-macbook --es md_tab messages
  scrolled device-mac-files 2 --es md_screen device --es md_device sample-macbook --es md_tab files
  scrolled device-mac-control 2 --es md_screen device --es md_device sample-macbook --es md_tab control
  scrolled device-mac-media 1 --es md_screen device --es md_device sample-macbook --es md_tab media
  scrolled device-mac-settings 2 --es md_screen device --es md_device sample-macbook --es md_tab settings
  shot device-mac-mini --es md_screen device --es md_device sample-mac-mini --es md_tab overview
  shot device-linux --es md_screen device --es md_device sample-lab-laptop --es md_tab overview
  shot device-linux-control --es md_screen device --es md_device sample-lab-laptop --es md_tab control
  shot device-windows-asleep --es md_screen device --es md_device sample-office-pc --es md_tab overview
  # More's screens.
  shot files --es md_screen files --es md_tab files
  shot files-transfers --es md_screen files --es md_tab transfers
  shot files-sync --es md_screen files --es md_tab sync
  shot messages --es md_screen messages --es md_tab chats
  shot messages-notifications --es md_screen messages --es md_tab notifications
  shot notes --es md_screen notes
  shot timers --es md_screen timers
  shot read-later --es md_screen readLater
  shot automations --es md_screen automations
  shot search --es md_screen search --es md_query pass
  shot links-map --es md_screen linksMap
  scrolled health 1 --es md_screen health
  shot onboarding --es md_screen onboarding
  # Settings and its groups.
  scrolled settings 1 --es md_screen settings
  for group in account phone files automation battery helper health backup appearance; do
    scrolled "settings-$group" 1 --es md_screen settings --es md_tab "$group"
  done
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
