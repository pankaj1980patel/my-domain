#!/usr/bin/env bash
# Measure what the my-domain app costs the phone's battery while it sits
# with the screen off.
#
#   scripts/lab/battery.sh 3600          # measure for an hour
#   scripts/lab/battery.sh diff A B      # compare two saved snapshots
#
# The phone stays on USB (it keeps charging) but is told it's unplugged, so
# Android keeps battery stats and lets it go into Doze. Then the screen goes
# off, the app's stats are saved, and again after the wait. At the end the
# battery state is reset to normal, even if the script is interrupted.
# Prints the difference: pushes received, wake lock time, CPU time, network.
#
# Runs adb on this machine (the phone on this Mac's USB). Don't use the
# phone during the measurement.
set -euo pipefail

APP="com.mydomain.android"
ADB="adb ${MD_ANDROID_SERIAL:+-s $MD_ANDROID_SERIAL}"

# The app's section of `dumpsys batterystats` (its uid line and what follows).
section() {
  awk -v uid="$1" '
    $0 ~ "^  "uid":$" { on = 1; next }
    on && /^  [^ ]/ { exit }
    on { print }
  ' "$2"
}

# One number per line, in a fixed order, from a saved snapshot.
numbers() {
  local uid="$1" file="$2"
  section "$uid" "$file" | awk '
    function ms(s,   t, n, i, v) {   # "2m 8s 143ms" -> milliseconds
      t = 0; n = split(s, p, " ")
      for (i = 1; i <= n; i++) {
        v = p[i]
        if (v ~ /ms$/) t += v + 0
        else if (v ~ /h$/) t += (v + 0) * 3600000
        else if (v ~ /m$/) t += (v + 0) * 60000
        else if (v ~ /s$/) t += (v + 0) * 1000
      }
      return t
    }
    /Wake lock mydomain:push:/ {
      t = $0; sub(/ times\).*/, "", t); sub(/.*\(/, "", t); pushes = t + 0
    }
    /TOTAL wake:/ { t = $0; sub(/.*TOTAL wake: /, "", t); sub(/ blamed.*/, "", t); wake = ms(t) }
    /Total cpu time:/ {
      u = $0; sub(/.*u=/, "", u); sub(/ s=.*/, "", u)
      k = $0; sub(/.* s=/, "", k)
      cpu = ms(u) + ms(k)
    }
    /Mobile radio active:/ { t = $0; sub(/.*active: /, "", t); sub(/ \(.*/, "", t); radio = ms(t) }
    /Wi-Fi network:/ {
      t = $0; sub(/.*Wi-Fi network: /, "", t); sub(/ received.*/, "", t)
      wifi = (t ~ /MB$/) ? (t + 0) * 1024 : t + 0
    }
    END { print pushes + 0; print wake + 0; print cpu + 0; print radio + 0; print wifi + 0 }
  '
}

report() {
  local uid="$1" before="$2" after="$3"
  paste <(numbers "$uid" "$before") <(numbers "$uid" "$after") | awk '
    BEGIN { split("pushes received|wake lock time (ms)|CPU time (ms)|mobile radio active (ms)|Wi-Fi received (KB)", name, "|") }
    { printf "%-26s %10d\n", name[NR], $2 - $1 }
  '
}

uid_of() {
  # u0a614 for appId 10614.
  local id
  id=$($ADB shell dumpsys package "$APP" | awk -F= '/appId=/{print $2; exit}' | tr -d '\r')
  echo "u0a$((id - 10000))"
}

if [ "${1:-}" = "diff" ]; then
  report "${UID_LABEL:-$(uid_of)}" "$2" "$3"
  exit 0
fi

secs="${1:-3600}"
dir="$(mktemp -d)"
uid="$(uid_of)"
restore() { $ADB shell dumpsys battery reset >/dev/null 2>&1 || true; }
trap restore EXIT

$ADB shell dumpsys battery unplug
$ADB shell input keyevent KEYCODE_SLEEP
$ADB shell dumpsys batterystats --charged "$APP" > "$dir/before.txt"
echo "measuring for ${secs}s from $(date +%H:%M); snapshots in $dir"
sleep "$secs"
$ADB shell dumpsys batterystats --charged "$APP" > "$dir/after.txt"
$ADB shell dumpsys deviceidle | grep -E "mState=" | sed 's/^ */doze: /'
report "$uid" "$dir/before.txt" "$dir/after.txt"
