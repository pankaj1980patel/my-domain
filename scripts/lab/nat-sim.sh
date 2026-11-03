#!/usr/bin/env bash
# Two devices, each behind its own simulated home router, must find a path
# to each other. Linux only; no root needed (an unprivileged user + network
# namespace holds everything, and it all disappears when the script ends).
#
#   scripts/lab/nat-sim.sh                 # every case below
#   scripts/lab/nat-sim.sh easy symmetric  # one case: router A, router B
#
# Router kinds:
#   easy       Linux masquerade: the same public port for every destination
#              (endpoint-independent mapping). Hole punching should work.
#   symmetric  masquerade with random ports: a new public port per
#              destination.
#
# Expected: easy + easy punch through. Anything with a symmetric router
# ends on the relay: Linux routers only let in replies from the exact
# address and port they sent to, and a symmetric router answers from a port
# nobody predicted. Every case must deliver the message and its answer.
#
# Layout ("internet" is the outer namespace):
#
#   md-fake-server 203.0.113.1 (+ .2 for the second STUN address)
#        | br0 (203.0.113.0/24)
#   r1 203.0.113.11 ---- r2 203.0.113.12      routers: NAT and a firewall
#                                             (unasked-for traffic dropped)
#        |                    |
#   h1 10.0.1.2          h2 10.0.2.2          mdctl "nat-a" / "nat-b"
#
# Build first: cargo build -p mdctl && cargo build -p md-testkit --bin md-fake-server
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/target/debug"

if [ "${1:-}" != "--inside" ]; then
  [ "$(uname)" = Linux ] || { echo "nat-sim.sh runs on Linux only" >&2; exit 2; }
  for b in mdctl md-fake-server; do
    [ -x "$BIN/$b" ] || { echo "missing $BIN/$b (see the build line at the top)" >&2; exit 2; }
  done
  if [ $# -eq 2 ]; then
    cases=("$1 $2")
  else
    cases=("easy easy" "easy symmetric" "symmetric symmetric")
  fi
  failed=0
  results=()
  for c in "${cases[@]}"; do
    # shellcheck disable=SC2086
    if out=$(unshare -rn --mount --fork "$0" --inside $c 2>&1); then
      status=PASS
    else
      status=FAIL
      failed=1
    fi
    path=$(printf '%s\n' "$out" | sed -n 's/^path: //p' | tail -1)
    results+=("$(printf '%-4s  A %-10s B %-10s path %s' "$status" ${c} "${path:-none}")")
    [ "$status" = PASS ] || printf '%s\n' "$out" | tail -40 >&2
  done
  echo
  printf '%s\n' "${results[@]}"
  exit "$failed"
fi

# ---- Inside the namespace ---------------------------------------------------
kind_a="$2"
kind_b="$3"
work="$(mktemp -d)"

mount -t tmpfs none /run
mkdir -p /run/netns
nsx() { local ns="$1"; shift; nsenter --net="/run/netns/$ns" "$@"; }

ip link set lo up
ip link add br0 type bridge
ip addr add 203.0.113.1/24 dev br0
ip addr add 203.0.113.2/24 dev br0
ip link set br0 up

# router <n> <kind>: router r<n> and the host h<n> behind it.
router() {
  local n="$1" kind="$2" r="r$1" h="h$1"
  ip netns add "$r"
  ip netns add "$h"
  ip link add "i$n" type veth peer name wan netns "$r"
  ip link set "i$n" master br0 up
  ip link add "l$n" netns "$r" type veth peer name eth0 netns "$h"
  nsx "$r" ip link set lo up
  nsx "$r" ip addr add "203.0.113.1$n/24" dev wan
  nsx "$r" ip link set wan up
  nsx "$r" ip addr add "10.0.$n.1/24" dev "l$n"
  nsx "$r" ip link set "l$n" up
  nsx "$r" sh -c 'echo 1 > /proc/sys/net/ipv4/ip_forward'
  local masq="masquerade"
  [ "$kind" = symmetric ] && masq="masquerade random"
  nsx "$r" nft -f - <<EOF
table ip nat {
  chain post {
    type nat hook postrouting priority srcnat;
    oifname "wan" $masq
  }
}
# Like every home router: drop what nobody asked for on the WAN side. This
# matters for punching: a packet the router accepts for itself leaves a
# conntrack entry that takes the port its own outgoing mapping needs.
table ip filter {
  chain input {
    type filter hook input priority filter;
    iifname "wan" ct state new drop
  }
  chain forward {
    type filter hook forward priority filter;
    iifname "wan" ct state new drop
  }
}
EOF
  nsx "$h" ip link set lo up
  nsx "$h" ip addr add "10.0.$n.2/24" dev eth0
  nsx "$h" ip link set eth0 up
  nsx "$h" ip route add default via "10.0.$n.1"
}
router 1 "$kind_a"
router 2 "$kind_b"

"$BIN/md-fake-server" 203.0.113.1 203.0.113.2 lab lab > "$work/server.log" 2>&1 &
for _ in $(seq 50); do grep -q ready "$work/server.log" && break; sleep 0.1; done

device() {  # device <n> <name> <script>
  nsx "h$1" "$BIN/mdctl" --name "$2" --data "$work/$2" --script "$3" \
    --server http://203.0.113.1:8080 --relay ws://203.0.113.1:8081/relay \
    --user lab --pass lab --key nat-sim-key \
    --stun 203.0.113.1:3478,203.0.113.2:3478 --signal-channel --no-lan
}

# B answers every message, A sends one, waits for the answer, then gives the
# ladder time to finish (the relay comes first, a direct path replaces it).
cat > "$work/b.mdctl" <<'EOF'
echo on
sleep 45
EOF
cat > "$work/a.mdctl" <<'EOF'
sleep 1
refresh
send @nat-b hello through the routers
wait message-received text="echo: hello through the routers" timeout=30
sleep 20
links
EOF

device 2 nat-b "$work/b.mdctl" > "$work/b.log" 2>&1 &
for _ in $(seq 100); do grep -q '"ready"' "$work/b.log" && break; sleep 0.1; done
rc=0
device 1 nat-a "$work/a.mdctl" > "$work/a.log" 2>&1 || rc=$?

echo "--- b.log (tail)"; tail -15 "$work/b.log"
echo "--- a.log (tail)"; tail -15 "$work/a.log"
# The transport A ended up on, from its last `links` answer.
path=$(grep '"cmd":"links"' "$work/a.log" | tail -1 | grep -o '"transport":"[a-z]*"' | head -1 | cut -d'"' -f4)
echo "path: ${path:-none}"
[ "$rc" -eq 0 ] || { echo "a failed (exit $rc)"; exit 1; }
case "$kind_a/$kind_b" in
  easy/easy) [ "$path" = punch ] || [ "$path" = quic ] ;;
  *) [ "$path" = relay ] ;;
esac
