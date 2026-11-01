#!/usr/bin/env bash
# Build an x86_64 Linux release bundle of the my-domain Tauri app, from macOS,
# inside the my-domain-linux-build Docker image. Run from the repo root.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

docker run --rm --platform linux/amd64 \
  -v "${REPO_ROOT}:/work" \
  -v my-domain-cargo-registry:/root/.cargo/registry \
  -v my-domain-linux-target:/work/apps/desktop/src-tauri/target-linux \
  -w /work/apps/desktop/src-tauri \
  my-domain-linux-build:latest \
  bash -c '
    set -euo pipefail
    export CARGO_TARGET_DIR=/work/apps/desktop/src-tauri/target-linux
    # Host target inside this amd64 container is already x86_64-unknown-linux-gnu.
    cargo tauri build
  '

echo
echo "Bundles written under: apps/desktop/src-tauri/target-linux/release/bundle/"
