#!/usr/bin/env bash
# Screenshots of every desktop screen, with SAMPLE data only.
#
# Builds the UI in screenshot mode (`vite build --mode mock`: the sample
# backend in apps/desktop/ui/src/dev/mockTauri.ts, never in the real app),
# serves it with `vite preview` on a free port, and opens each screen in
# headless Google Chrome:
#
#   screenshots/desktop/<NNN>-<view>[-<part>]-<theme>.jpg   (1280x820)
#
# JPEG (quality 80, macOS `sips`) because the star-field background makes
# each PNG ~0.5 MB; FORMAT=png keeps Chrome's PNGs (and is what you get
# where there's no sips).
#
# Usage:
#   scripts/screenshots-desktop.sh            all of them
#   scripts/screenshots-desktop.sh home       only shots whose name contains "home"
#   CHROME=/path/to/chrome FORMAT=png scripts/screenshots-desktop.sh
#
# No new dependencies: node/npm (already used for the UI) and Chrome.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UI="$ROOT/apps/desktop/ui"
OUT="$ROOT/screenshots/desktop"
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
ONLY="${1:-}"
SIZE="1280,820"
FORMAT="${FORMAT:-jpg}"
command -v sips >/dev/null || FORMAT=png

[ -x "$CHROME" ] || { echo "Google Chrome not found at: $CHROME (set CHROME=...)" >&2; exit 1; }

cd "$UI"
[ -d node_modules ] || npm ci
echo "Building the UI in screenshot mode..."
npx vite build --mode mock --logLevel warn

# A free port.
PORT="$(node -e 'const s=require("net").createServer();s.listen(0,()=>{console.log(s.address().port);s.close()})')"
npx vite preview --mode mock --port "$PORT" --strictPort >/dev/null 2>&1 &
SERVER=$!
PROFILE="$(mktemp -d)"
cleanup() {
  kill "$SERVER" 2>/dev/null || true
  rm -rf "$PROFILE"
}
trap cleanup EXIT
for _ in $(seq 1 50); do
  curl -sf "http://localhost:$PORT/" >/dev/null && break
  sleep 0.2
done

mkdir -p "$OUT"
[ -n "$ONLY" ] || rm -f "$OUT"/*.png "$OUT"/*.jpg

N=0
# shot <name> <query> [size]
shot() {
  local name="$1" query="$2" size="${3:-$SIZE}"
  N=$((N + 1))
  local file
  file="$(printf '%03d' "$N")-$name"
  if [ -n "$ONLY" ] && [[ "$name" != *"$ONLY"* ]]; then return; fi
  "$CHROME" --headless=new --disable-gpu --no-first-run --no-default-browser-check \
    --user-data-dir="$PROFILE" --hide-scrollbars --force-device-scale-factor=1 \
    --window-size="$size" --virtual-time-budget=5000 \
    --screenshot="$PROFILE/shot.png" "http://localhost:$PORT/?mock=1&$query" >/dev/null 2>&1
  if [ "$FORMAT" = jpg ]; then
    sips -s format jpeg -s formatOptions 80 "$PROFILE/shot.png" --out "$OUT/$file.jpg" >/dev/null
  else
    mv "$PROFILE/shot.png" "$OUT/$file.png"
  fi
  echo "  $file.$FORMAT"
}

# How tall a page's content is (the mock writes it on <body data-md-height>).
page_height() {
  "$CHROME" --headless=new --disable-gpu --no-first-run --user-data-dir="$PROFILE" \
    --window-size="$SIZE" --virtual-time-budget=5000 --dump-dom \
    "http://localhost:$PORT/?mock=1&$1" 2>/dev/null | grep -o 'data-md-height="[0-9]*"' | grep -o '[0-9]*' || echo 0
}
# One screen's worth of scrolling (a little overlap with the last one).
STEP=720

# Main screens: light and dark.
both() {
  local name="$1" query="$2"
  shot "$name-dark" "$query&theme=dark"
  shot "$name-light" "$query&theme=light"
}

echo "Taking screenshots into $OUT ..."
shot sign-in-dark "view=signIn&theme=dark"
shot sign-in-light "view=signIn&theme=light"
for step in name devices features permissions connection; do
  shot "setup-$step-dark" "view=setup&step=$step&theme=dark"
done
both home "view=home"

# Each device's page, top to bottom.
for dev in pixel8:phone macbook:mac officepc:windows lablaptop:linux; do
  id="${dev%%:*}"
  label="${dev##*:}"
  both "device-$label" "view=device&device=$id"
  # A screen further down each time, to the end of the page.
  height="$(page_height "view=device&device=$id")"
  px=$STEP
  while [ "$((px - STEP))" -lt "$((height - 820))" ] && [ "$px" -le 12000 ]; do
    shot "device-$label-$(printf '%05d' "$px")px-dark" "view=device&device=$id&scroll=$px&theme=dark"
    px=$((px + STEP))
  done
done

both links-map "view=linksMap"
both health "view=health"
both activity "view=activity"
both transfers "view=transfers"
both notes "view=notes"
shot notes-open-dark "view=notes&note=note-1&theme=dark"
both timers "view=timers"
both read-later "view=readLater"
both search "view=search&q=pass"
both help "view=help"
shot help-topic-dark "view=help&topic=getting-started&theme=dark"

# Settings: the top, then each section.
both settings "view=settings"
for section in account permissions removed-devices key-rotation health device shortcuts calls computer-notifs \
  wake file-manager sync downloads commands kvm insert print schedules automations webhooks \
  browser-extension key backup updates appearance; do
  shot "settings-$section-dark" "view=settings&section=$section&theme=dark"
done
shot developer-dark "view=developer&theme=dark"

# The menu-bar quick panel (its own small window).
shot quick-panel-dark "window=quick-panel&theme=dark" "360,540"
shot quick-panel-light "window=quick-panel&theme=light" "360,540"

# Dialogs and banners.
shot dialog-key-changed-dark "view=device&device=pixel8&dialog=keyChanged&theme=dark"
shot dialog-permission-dark "view=home&dialog=permission&theme=dark"
shot dialog-permission-light "view=home&dialog=permission&theme=light"
shot dialog-download-dark "view=home&dialog=download&theme=dark"
shot dialog-print-dark "view=home&dialog=print&theme=dark"
shot dialog-incoming-call-dark "view=home&dialog=call&theme=dark"
shot dialog-ring-dark "view=home&dialog=ring&theme=dark"
shot dialog-hotspot-dark "view=home&dialog=hotspot&theme=dark"

# One language other than English.
shot home-hindi-dark "view=home&lang=hi&theme=dark"
shot device-phone-hindi-dark "view=device&device=pixel8&lang=hi&theme=dark"

echo "Done: $(find "$OUT" -name '*.png' -o -name '*.jpg' | wc -l | tr -d ' ') screenshots, $(du -sh "$OUT" | cut -f1)."
