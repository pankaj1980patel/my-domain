#!/usr/bin/env bash
# Follow the live lab log (builds, installs, tests) from the Mac. Ctrl-C to stop.
source "$(dirname "$0")/common.sh"
ssh -t "$MD_ARCH_HOST" 'touch ~/my-domain-lab.log; tail -n 50 -f ~/my-domain-lab.log'
