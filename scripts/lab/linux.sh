#!/usr/bin/env bash
# Build / test the Rust workspace on the arch (Linux) host.
#   scripts/lab/linux.sh check   # cargo check --workspace
#   scripts/lab/linux.sh test    # cargo test --workspace
#   scripts/lab/linux.sh desktop # run the Tauri desktop app on the laptop's display
#   scripts/lab/linux.sh start   # same, detached, logged in with the test account
#                                # (log: ~/my-domain-desktop.log on the laptop)
source "$(dirname "$0")/common.sh"

cmd="${1:-test}"
"$(dirname "$0")/sync.sh"

case "$cmd" in
  check)   remote "cargo check --workspace --locked" ;;
  test)    remote "cargo test --workspace --locked" ;;
  desktop) remote "cd apps/desktop && npm install --silent && DISPLAY=\${DISPLAY:-:0} npm run tauri dev" ;;
  start)
    # Detached so this returns. setsid puts npm + cargo + the app in their own
    # process group, recorded in a pid file so a re-run stops the old one.
    # The test account is read from the synced test-creds.env ON the laptop,
    # so the password never appears in the command line or the lab log.
    # (No \$(...) here: remote() echoes the command into the log, and that echo
    # would run it.)
    remote "PIDF=\$HOME/.my-domain-desktop.pid; [ -f \$PIDF ] && xargs -I{} kill -- -{} < \$PIDF 2>/dev/null; sleep 1;
      set -a && . scripts/lab/test-creds.env && set +a &&
      cd apps/desktop && npm install --silent &&
      { DISPLAY=\${DISPLAY:-:0} setsid nohup npm run tauri dev > \$HOME/my-domain-desktop.log 2>&1 < /dev/null & echo \$! > \$PIDF; } &&
      echo started: tail -f ~/my-domain-desktop.log" ;;
  *) echo "usage: $0 {check|test|desktop|start}"; exit 1 ;;
esac
