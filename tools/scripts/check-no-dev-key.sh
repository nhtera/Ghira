#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# RT-12: release binaries must not contain the debug-only file key store
# (crates/ghi-store/src/keys/dev.rs, compiled only with debug_assertions).
# Usage: check-no-dev-key.sh <binary>...   (e.g. target/release/ghi)
set -euo pipefail

marker="GHIRA-DEV-FILE-KEYSTORE-DO-NOT-SHIP"
[[ $# -gt 0 ]] || { echo "usage: $0 <release binary>..." >&2; exit 2; }
status=0
for bin in "$@"; do
  if [[ ! -f "$bin" ]]; then
    echo "error: $bin not found (build it with --release first)" >&2
    status=1
  elif LC_ALL=C grep -aq "$marker" "$bin"; then
    echo "error: $bin contains the dev key store" >&2
    status=1
  fi
done
[[ $status -eq 0 ]] && echo "no dev key: ok ($#)"
exit $status
