#!/usr/bin/env bash
#
# Deploy my-domain-server to the Oracle Cloud Ubuntu instance.
#
#   ./deploy.sh                # build amd64 binary, ship it, restart service
#   BIN_SOURCE=gh ./deploy.sh  # download latest GitHub Actions artifact instead of building
#   BIN_SOURCE=remote ./deploy.sh   # build on the server itself (needs cargo there)
#   BIN_SOURCE=docker ./deploy.sh   # cross-compile in a native Docker container
#                                   # (fast on an arm64 Mac; the 1 GB server can't build it)
#
# Config via env vars (defaults match deployment.dev.md):
#   SSH_TARGET    ssh host (uses ~/.ssh/config)  (default oracle-dev)
#   APP_DIR       remote install dir   (default /opt/my-domain-server)
#   SERVICE       systemd unit name    (default my-domain-server)
#   BIN_SOURCE    cross | docker | gh | remote  (default: auto — cross if installed, else gh)
#
# Server env / FCM push credentials (optional — written into the remote
# systemd EnvironmentFile so the server can send FCM data messages):
#   ENV_FILE         remote EnvironmentFile path  (default /etc/my-domain-server.env)
#   FCM_SA_FILE      local path to the Firebase service-account JSON
#                    (default ./fcm-sa.json if present)
#   FCM_SA_JSON      inline service-account JSON (alternative to FCM_SA_FILE)
#   FCM_PROJECT_ID   override project id (optional; defaults to the SA's own)
#   REMOTE_FCM_JSON  where to store the key on the remote
#                    (default /etc/my-domain-server-fcm.json)
# With none of these set, the FCM step is skipped (server keeps its existing env).
#
set -euo pipefail

SSH_TARGET="${SSH_TARGET:-oracle-dev}"
APP_DIR="${APP_DIR:-/opt/my-domain-server}"
SERVICE="${SERVICE:-my-domain-server}"
BIN_NAME="my-domain-server"
TARGET="x86_64-unknown-linux-gnu"
BIN_SOURCE="${BIN_SOURCE:-auto}"

ENV_FILE="${ENV_FILE:-/etc/my-domain-server.env}"
FCM_SA_FILE="${FCM_SA_FILE:-fcm-sa.json}"
FCM_SA_JSON="${FCM_SA_JSON:-}"
FCM_PROJECT_ID="${FCM_PROJECT_ID:-}"
REMOTE_FCM_JSON="${REMOTE_FCM_JSON:-/etc/my-domain-server-fcm.json}"

cd "$(dirname "$0")"

SSH_OPTS=()
SCP_OPTS=()
REMOTE="$SSH_TARGET"

say() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }

# --- push FCM service-account creds into the remote EnvironmentFile -----------
# Uploads the Firebase service-account JSON and points
# GOOGLE_APPLICATION_CREDENTIALS at it in $ENV_FILE (creating/updating keys
# idempotently). The next service restart picks it up. No-op if no key is given.
push_fcm_creds() {
  local sa_src="" tmp_json=""
  if [[ -n "$FCM_SA_JSON" ]]; then
    tmp_json="$(mktemp)"
    printf '%s' "$FCM_SA_JSON" > "$tmp_json"
    sa_src="$tmp_json"
  elif [[ -f "$FCM_SA_FILE" ]]; then
    sa_src="$FCM_SA_FILE"
  else
    say "FCM: no service account (set FCM_SA_FILE or FCM_SA_JSON). Skipping FCM env."
    return 0
  fi

  say "FCM: uploading service account and wiring $ENV_FILE ..."
  scp "${SCP_OPTS[@]}" "$sa_src" "$REMOTE:/tmp/fcm-sa.json"
  [[ -n "$tmp_json" ]] && rm -f "$tmp_json"

  ssh "${SSH_OPTS[@]}" "$REMOTE" bash -s <<EOSSH
set -euo pipefail
# The server process reads this file at runtime, so it must be owned by the
# service user (systemd User=), not root — otherwise from_env() can't read it
# and FCM silently stays disabled.
SVC_USER="\$(systemctl show ${SERVICE} -p User --value 2>/dev/null || true)"
SVC_USER="\${SVC_USER:-root}"
sudo install -m600 /tmp/fcm-sa.json "${REMOTE_FCM_JSON}"
sudo chown "\${SVC_USER}:\${SVC_USER}" "${REMOTE_FCM_JSON}"
rm -f /tmp/fcm-sa.json
sudo touch "${ENV_FILE}"
# Idempotently set KEY=value in the EnvironmentFile (update in place, else append).
set_env() {
  local key="\$1" val="\$2"
  if sudo grep -q "^\${key}=" "${ENV_FILE}"; then
    sudo sed -i "s#^\${key}=.*#\${key}=\${val}#" "${ENV_FILE}"
  else
    echo "\${key}=\${val}" | sudo tee -a "${ENV_FILE}" >/dev/null
  fi
}
set_env GOOGLE_APPLICATION_CREDENTIALS "${REMOTE_FCM_JSON}"
[ -n "${FCM_PROJECT_ID}" ] && set_env FCM_PROJECT_ID "${FCM_PROJECT_ID}"
sudo chmod 600 "${ENV_FILE}"
echo "FCM env wired -> GOOGLE_APPLICATION_CREDENTIALS=${REMOTE_FCM_JSON}"
EOSSH
}

# --- pick how to get the amd64 binary -----------------------------------------
if [[ "$BIN_SOURCE" == "auto" ]]; then
  if command -v cross >/dev/null 2>&1; then
    BIN_SOURCE="cross"
  elif command -v gh >/dev/null 2>&1; then
    BIN_SOURCE="gh"
  else
    BIN_SOURCE="remote"
  fi
fi
say "Binary source: $BIN_SOURCE"

LOCAL_BIN=""
case "$BIN_SOURCE" in
  cross)
    say "Cross-compiling for $TARGET ..."
    # The server is a member of the repo-root Cargo workspace: build just this
    # package; output lands in the shared <repo>/target.
    cross build --release --locked -p "$BIN_NAME" --target "$TARGET"
    LOCAL_BIN="../../target/$TARGET/release/$BIN_NAME"
    ;;
  docker)
    # Native-arch container + an x86_64 cross gcc (for ring): no emulation.
    # Debian bookworm's glibc (2.36) is older than the server's, so the binary runs there.
    say "Cross-compiling for $TARGET in Docker ..."
    docker run --rm -v "$(cd ../.. && pwd)":/src -v md-cargo-registry:/usr/local/cargo/registry -w /src \
      -e CARGO_TARGET_DIR=/src/target/docker \
      -e CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
      -e CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc -e AR_x86_64_unknown_linux_gnu=x86_64-linux-gnu-ar \
      rust:1-bookworm bash -c "apt-get update -qq >/dev/null && apt-get install -y -qq gcc-x86-64-linux-gnu libc6-dev-amd64-cross >/dev/null &&
        rustup target add $TARGET >/dev/null 2>&1 && cargo build --release --locked -p $BIN_NAME --target $TARGET"
    LOCAL_BIN="../../target/docker/$TARGET/release/$BIN_NAME"
    ;;
  gh)
    say "Downloading latest successful build artifact via gh ..."
    TMP="$(mktemp -d)"
    RUN_ID="$(gh run list --workflow server-release.yml --status success --limit 1 --json databaseId -q '.[0].databaseId')"
    [[ -n "$RUN_ID" ]] || { echo "No successful server-release.yml run found. Push to main or run the workflow first."; exit 1; }
    gh run download "$RUN_ID" --name "${BIN_NAME}-amd64" --dir "$TMP"
    LOCAL_BIN="$TMP/$BIN_NAME"
    chmod +x "$LOCAL_BIN"
    ;;
  remote)
    say "Building on the remote host ..."
    ssh "${SSH_OPTS[@]}" "$REMOTE" "command -v cargo >/dev/null || { echo 'cargo not installed on remote'; exit 1; }"
    # Sync the whole workspace (the root Cargo.toml/Cargo.lock are needed for
    # --locked) minus build output and secrets, then build just the server.
    rsync -az --delete -e "ssh ${SSH_OPTS[*]}" \
      --exclude target --exclude .git --exclude .env --exclude node_modules \
      --exclude "fcm-sa.json" --exclude "google-services*.json" --exclude "test-creds.env" \
      ../../ "$REMOTE:/tmp/${BIN_NAME}-src/"
    push_fcm_creds
    ssh "${SSH_OPTS[@]}" "$REMOTE" \
      "cd /tmp/${BIN_NAME}-src && cargo build --release --locked -p ${BIN_NAME} && \
       sudo install -m755 target/release/${BIN_NAME} ${APP_DIR}/${BIN_NAME} && \
       sudo systemctl restart ${SERVICE} && systemctl is-active ${SERVICE}"
    say "Deployed (remote build). Done."
    exit 0
    ;;
  *)
    echo "Unknown BIN_SOURCE: $BIN_SOURCE"; exit 1 ;;
esac

[[ -f "$LOCAL_BIN" ]] || { echo "Binary not found at $LOCAL_BIN"; exit 1; }
say "Using binary: $LOCAL_BIN ($(du -h "$LOCAL_BIN" | cut -f1))"

# --- ship it ------------------------------------------------------------------
say "Uploading to $REMOTE ..."
scp "${SCP_OPTS[@]}" "$LOCAL_BIN" "$REMOTE:/tmp/${BIN_NAME}.new"

push_fcm_creds

say "Installing and restarting service ..."
ssh "${SSH_OPTS[@]}" "$REMOTE" bash -s <<EOF
set -euo pipefail
sudo mkdir -p "${APP_DIR}"
# Keep the running binary for a quick rollback: sudo mv ${APP_DIR}/${BIN_NAME}.prev ${APP_DIR}/${BIN_NAME}
[ -f "${APP_DIR}/${BIN_NAME}" ] && sudo cp "${APP_DIR}/${BIN_NAME}" "${APP_DIR}/${BIN_NAME}.prev"
sudo install -m755 "/tmp/${BIN_NAME}.new" "${APP_DIR}/${BIN_NAME}"
rm -f "/tmp/${BIN_NAME}.new"
sudo systemctl restart "${SERVICE}"
sleep 1
systemctl is-active "${SERVICE}"
EOF

say "Health check ..."
if ssh "${SSH_OPTS[@]}" "$REMOTE" "curl -fsS http://localhost:\${PORT:-6969}/health" 2>/dev/null; then
  echo
  say "Deploy complete ✅"
else
  echo
  say "Service restarted, but health check did not return OK. Check: ssh $REMOTE 'journalctl -u $SERVICE -n 50'"
fi
