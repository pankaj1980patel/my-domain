#!/usr/bin/env python3
"""Write latest.json, the desktop app's update manifest, from a release's files.

The app (apps/desktop/src-tauri/src/updates.rs, Tauri's updater plugin) reads
https://github.com/<repo>/releases/latest/download/latest.json, picks its
platform's entry ("{os}-{arch}-{installer}", else "{os}-{arch}"), downloads
the url and checks it against the signature with the public key it was built
with.

Usage: scripts/updater-manifest.py DIST VERSION TAG REPO [NOTES_FILE]
  DIST        the folder with the release files and their .sig files, named
              as .github/workflows/desktop-release.yml collects them
              (my-domain_<version>_macos_aarch64.app.tar.gz, ...)
  VERSION     the app version (0.2.0)
  TAG         the release tag (v0.2.0): the download URLs point at it
  REPO        owner/name on GitHub
  NOTES_FILE  optional: the release notes shown in the app's "What's new"

Writes DIST/latest.json and prints it. A file without its .sig is left out
(built without the signing key); with no signed file at all it writes
nothing and exits 0, so an unsigned release still goes out, just without
updates. Exit 1 on a bad argument.
"""

import json
import sys
from datetime import datetime, timezone
from pathlib import Path

# File-name suffix -> the platform keys it serves.
PLATFORMS = [
    ("_macos_aarch64.app.tar.gz", ["darwin-aarch64-app", "darwin-aarch64"]),
    ("_macos_x86_64.app.tar.gz", ["darwin-x86_64-app", "darwin-x86_64"]),
    ("_linux_x86_64.AppImage", ["linux-x86_64-appimage", "linux-x86_64"]),
    ("_linux_amd64.deb", ["linux-x86_64-deb"]),
    ("_windows_x64-setup.exe", ["windows-x86_64-nsis", "windows-x86_64"]),
    ("_windows_x64.msi", ["windows-x86_64-msi"]),
]


def manifest(dist: Path, version: str, tag: str, repo: str, notes: str) -> dict:
    platforms: dict[str, dict[str, str]] = {}
    for suffix, keys in PLATFORMS:
        name = f"my-domain_{version}{suffix}"
        sig = dist / f"{name}.sig"
        if not (dist / name).is_file() or not sig.is_file():
            continue
        entry = {
            "signature": sig.read_text().strip(),
            "url": f"https://github.com/{repo}/releases/download/{tag}/{name}",
        }
        for key in keys:
            platforms[key] = entry
    return {
        "version": version,
        "notes": notes,
        "pub_date": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "platforms": platforms,
    }


def main(argv: list[str]) -> int:
    if len(argv) not in (5, 6):
        print(__doc__, file=sys.stderr)
        return 1
    dist, version, tag, repo = Path(argv[1]), argv[2], argv[3], argv[4]
    notes = Path(argv[5]).read_text().strip() if len(argv) == 6 and Path(argv[5]).is_file() else ""
    if not dist.is_dir() or not version or "/" not in repo:
        print(__doc__, file=sys.stderr)
        return 1
    m = manifest(dist, version, tag, repo, notes)
    if not m["platforms"]:
        print("no signed update files: latest.json not written (updates need the signing key)")
        return 0
    text = json.dumps(m, indent=2) + "\n"
    (dist / "latest.json").write_text(text)
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
