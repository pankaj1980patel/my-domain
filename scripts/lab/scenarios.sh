#!/usr/bin/env bash
# Run the mdctl lab scenarios from this Mac against every lab device, and
# print a pass/fail table. Exits non-zero if any scenario failed.
#
#   scripts/lab/scenarios.sh                        # every device, default set
#   MD_PEERS="I2405" scripts/lab/scenarios.sh       # just the phone
#   MD_SCENARIOS="receipts" scripts/lab/scenarios.sh
#
# The devices must be running the app and logged in to the test account
# (scripts/lab/start-all.sh). Credentials come from test-creds.env; mdctl
# reads the MD_DEV_* variables itself, so they never appear on a command line.
# Nothing loud: no ring, no flashlight (controls.mdctl only nudges the volume
# up and back down).
source "$(dirname "$0")/common.sh"

PEERS="${MD_PEERS:-pankaj-aspirea32542 I2405}"
SCENARIOS="${MD_SCENARIOS:-receipts controls file-transfer}"
DATA="${MD_LAB_DATA:-/tmp/lab-box}"   # same identity every run: one registered device
LOGS="$(mktemp -d)"

set -a && . "$MD_REPO_ROOT/scripts/lab/test-creds.env" && set +a
cd "$MD_REPO_ROOT"
say "Building mdctl"
cargo build -q -p mdctl
MDCTL="$MD_REPO_ROOT/target/debug/mdctl"

# A random file for file-transfer (no spaces in the path).
export MD_FILE="$LOGS/lab-file.bin"
head -c 1048576 /dev/urandom > "$MD_FILE"

failed=0
results=()
for peer in $PEERS; do
  for sc in $SCENARIOS; do
    script="scripts/lab/scenarios/$sc.mdctl"
    log="$LOGS/$peer-$sc.log"
    start=$(date +%s)
    if MD_PEER="$peer" "$MDCTL" --name lab-box --data "$DATA" --script "$script" > "$log" 2>&1; then
      status=PASS
    else
      status=FAIL
      failed=1
    fi
    secs=$(( $(date +%s) - start ))
    results+=("$(printf '%-4s  %-22s %-14s %4ss' "$status" "$peer" "$sc" "$secs")")
    say "$status $peer $sc (${secs}s)"
  done
done

echo
printf '%s\n' "${results[@]}"
echo
echo "Logs: $LOGS"
[ "$failed" -eq 0 ] && rm -f "$MD_FILE"
exit "$failed"
