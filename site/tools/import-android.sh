#!/usr/bin/env bash
# Copy the Android screenshots and videos into the website, optimised, and
# write site/media/android/manifest.json, which the page reads to build the
# Android gallery and add the Android videos. With no screens in the manifest
# the page shows a "coming soon" panel instead.
#
# usage: site/tools/import-android.sh [SOURCE_DIR]
#   SOURCE_DIR  folder with the PNGs and a videos/ folder of .mp4 files
#               (default: the android-redesign worktree's screenshots/android)
#
# Choosing files: put a list in site/tools/android-shots.txt, one per line:
#     file.png|Title|Optional alt text
#     videos/tour.mp4|Title|Optional caption
# Lines starting with # are ignored. With no video lines, every videos/*.mp4
# is imported. Without the list at all, every PNG and every videos/*.mp4 is
# imported, titled from its file name ("03-device-page.png" becomes "Device page").
#
# Output: WebP (or JPEG when cwebp isn't installed) at most 1080 px wide; MP4s
# copied as they are, or re-encoded (H.264, CRF 28, ≤ 1080 wide, no audio)
# when ffmpeg is present and a file is over 5 MB; a WebP poster per video.
# Needs: python3, plus cwebp or ImageMagick (magick); ffmpeg optional.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SITE="$(cd "$HERE/.." && pwd)"
DEST="$SITE/media/android"
if [ -n "${1:-}" ]; then
  SRC="$1"
else
  # The android-redesign worktree lives under the main checkout's .claude/worktrees.
  MAIN="$(cd "$(git -C "$SITE" rev-parse --git-common-dir)/.." && pwd)"
  SRC="$MAIN/.claude/worktrees/android-redesign/screenshots/android"
fi
LIST="$HERE/android-shots.txt"
MAXW=1080
FFMPEG="$(command -v ffmpeg || true)"
[ -z "$FFMPEG" ] && [ -x /opt/homebrew/bin/ffmpeg ] && FFMPEG=/opt/homebrew/bin/ffmpeg

if [ ! -d "$SRC" ]; then
  echo "No Android screenshots at $SRC" >&2
  echo "Pass the folder: $0 /path/to/screenshots/android" >&2
  exit 1
fi
SRC="$(cd "$SRC" && pwd)"
command -v python3 >/dev/null || { echo "python3 is needed" >&2; exit 1; }
if ! command -v cwebp >/dev/null && ! command -v magick >/dev/null; then
  echo "Install cwebp (libwebp) or ImageMagick" >&2; exit 1
fi

# Build the list of "kind|relative path|title|text" entries.
ENTRIES="$(mktemp)"
trap 'rm -f "$ENTRIES"' EXIT
title_of() { # 03-device-page.png -> Device page
  local b; b="$(basename "$1")"; b="${b%.*}"
  b="$(printf '%s' "$b" | sed -E 's/^[0-9]+[a-z]?[-_ ]*//; s/[-_]+/ /g')"
  printf '%s' "$(printf '%s' "${b:0:1}" | tr '[:lower:]' '[:upper:]')${b:1}"
}
if [ -f "$LIST" ]; then
  while IFS='|' read -r f t x; do
    case "$f" in ''|\#*) continue;; esac
    [ -f "$SRC/$f" ] || { echo "skip (missing): $f" >&2; continue; }
    kind=screen; case "$f" in *.mp4) kind=video;; esac
    printf '%s|%s|%s|%s\n' "$kind" "$f" "${t:-$(title_of "$f")}" "$x" >> "$ENTRIES"
  done < "$LIST"
  # No videos listed: take them all.
  if ! grep -q '^video|' "$ENTRIES" 2>/dev/null && [ -d "$SRC/videos" ]; then
    (cd "$SRC" && find videos -maxdepth 1 -type f -iname '*.mp4' | LC_ALL=C sort) | while read -r f; do
      printf 'video|%s|%s|\n' "$f" "$(title_of "$f")" >> "$ENTRIES"
    done
  fi
else
  (cd "$SRC" && find . -maxdepth 1 -type f -iname '*.png' | sed 's|^\./||' | LC_ALL=C sort) | while read -r f; do
    printf 'screen|%s|%s|\n' "$f" "$(title_of "$f")" >> "$ENTRIES"
  done
  if [ -d "$SRC/videos" ]; then
    (cd "$SRC" && find videos -maxdepth 1 -type f -iname '*.mp4' | LC_ALL=C sort) | while read -r f; do
      printf 'video|%s|%s|\n' "$f" "$(title_of "$f")" >> "$ENTRIES"
    done
  fi
fi

# Start clean: remove what an earlier import put here.
mkdir -p "$DEST"
find "$DEST" -maxdepth 1 -type f \( -name '*.webp' -o -name '*.jpg' -o -name '*.mp4' \) -delete

slug_of() { local b; b="$(basename "$1")"; b="${b%.*}"; printf '%s' "$b" | tr '[:upper:] ' '[:lower:]-' | tr -cd 'a-z0-9._-'; }
width_of() { python3 - "$1" <<'PY'
import struct, sys
with open(sys.argv[1], "rb") as f:
    h = f.read(24)
print(struct.unpack(">II", h[16:24])[0] if h[:8] == b"\x89PNG\r\n\x1a\n" else 0)
PY
}

MANIFEST_ROWS="$(mktemp)"
trap 'rm -f "$ENTRIES" "$MANIFEST_ROWS"' EXIT
while IFS='|' read -r kind f title text; do
  in="$SRC/$f"; slug="$(slug_of "$f")"
  if [ "$kind" = screen ]; then
    w="$(width_of "$in")"; resize=()
    if command -v cwebp >/dev/null; then
      [ "$w" -gt "$MAXW" ] && resize=(-resize "$MAXW" 0)
      cwebp -quiet -q 80 -m 6 ${resize[@]+"${resize[@]}"} "$in" -o "$DEST/$slug.webp"
      out="$slug.webp"
    else
      magick "$in" -resize "${MAXW}x>" -strip -quality 82 "$DEST/$slug.jpg"
      out="$slug.jpg"
    fi
    dims="$(python3 - "$DEST/$out" <<'PY'
import subprocess, sys
try:
    from PIL import Image
    w, h = Image.open(sys.argv[1]).size
    print(f"{w} {h}")
except Exception:
    print("0 0")
PY
)"
    printf 'screen\t%s\t%s\t%s\t%s\n' "$out" "$title" "$text" "$dims" >> "$MANIFEST_ROWS"
    echo "  screen  $out"
  else
    out="$slug.mp4"; size=$(wc -c < "$in")
    if [ -n "$FFMPEG" ] && [ "$size" -gt 5000000 ]; then
      "$FFMPEG" -loglevel error -y -i "$in" -an -c:v libx264 -preset slow -crf 28 -pix_fmt yuv420p \
        -vf "scale='min($MAXW,iw)':-2" -movflags +faststart "$DEST/$out"
    else
      cp "$in" "$DEST/$out"
    fi
    poster=""
    if [ -n "$FFMPEG" ]; then
      "$FFMPEG" -loglevel error -y -ss 1 -i "$DEST/$out" -frames:v 1 -vf "scale='min($MAXW,iw)':-2" "$DEST/$slug-poster.png" \
        && { if command -v cwebp >/dev/null; then cwebp -quiet -q 78 "$DEST/$slug-poster.png" -o "$DEST/$slug-poster.webp"; poster="$slug-poster.webp";
             else magick "$DEST/$slug-poster.png" -quality 80 "$DEST/$slug-poster.jpg"; poster="$slug-poster.jpg"; fi
             rm -f "$DEST/$slug-poster.png"; }
    fi
    printf 'video\t%s\t%s\t%s\t%s\n' "$out" "$title" "$text" "$poster" >> "$MANIFEST_ROWS"
    echo "  video   $out ($(( $(wc -c < "$DEST/$out") / 1024 )) KB)"
  fi
done < "$ENTRIES"

python3 - "$MANIFEST_ROWS" "$DEST/manifest.json" <<'PY'
import json, sys
screens, videos = [], []
for line in open(sys.argv[1], encoding="utf-8"):
    kind, file, title, text, extra = line.rstrip("\n").split("\t")
    if kind == "screen":
        item = {"file": file, "title": title}
        if text: item["alt"] = text
        w, h = (extra.split() + ["0", "0"])[:2]
        if w != "0": item["width"], item["height"] = int(w), int(h)
        screens.append(item)
    else:
        item = {"file": file, "title": title}
        if text: item["caption"] = text
        if extra: item["poster"] = extra
        videos.append(item)
json.dump({"screens": screens, "videos": videos}, open(sys.argv[2], "w", encoding="utf-8"), indent=2, ensure_ascii=False)
open(sys.argv[2], "a").write("\n")
print(f"manifest: {len(screens)} screens, {len(videos)} videos")
PY
du -sh "$DEST"
