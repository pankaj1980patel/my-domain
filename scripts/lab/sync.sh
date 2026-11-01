#!/usr/bin/env bash
# Mirror this working tree (including uncommitted changes) to the arch host so
# both machines build exactly the same code. Build output and secrets stay local.
source "$(dirname "$0")/common.sh"

say "Syncing $MD_REPO_ROOT -> $MD_ARCH_HOST:~/$MD_ARCH_DIR"
rsync -az --delete \
  --exclude target/ --exclude '**/target/' --exclude node_modules/ \
  --exclude .gradle/ --exclude '**/build/' --exclude .git/ \
  --exclude .env --exclude fcm-sa.json \
  "$MD_REPO_ROOT/" "$MD_ARCH_HOST:$MD_ARCH_DIR/"
say "Synced."
