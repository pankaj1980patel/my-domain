#!/usr/bin/env bash
# Build / test the Rust workspace on the arch (Linux) host.
#   scripts/lab/linux.sh check   # cargo check --workspace
#   scripts/lab/linux.sh test    # cargo test --workspace
#   scripts/lab/linux.sh desktop # run the Tauri desktop app on the laptop's display
source "$(dirname "$0")/common.sh"

cmd="${1:-test}"
"$(dirname "$0")/sync.sh"

case "$cmd" in
  check)   remote "cargo check --workspace --locked" ;;
  test)    remote "cargo test --workspace --locked" ;;
  desktop) remote "cd apps/desktop && npm install --silent && DISPLAY=\${DISPLAY:-:0} npm run tauri dev" ;;
  *) echo "usage: $0 {check|test|desktop}"; exit 1 ;;
esac
