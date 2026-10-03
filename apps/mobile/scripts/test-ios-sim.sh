#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs `cargo test` for iOS-compatible crates ON the Simulator (phase 16-B):
# the test binaries are built for aarch64-apple-ios-sim and executed with
# `xcrun simctl spawn`. Simulator only; the UDID must be a booted simulator.
#
#   apps/mobile/scripts/test-ios-sim.sh [-p crate]... [-- test args]   (default: -p ghi-store)
#
# Spawned binaries carry no entitlements, so Keychain-backed tests fail there:
# skip them with  -- --skip keychain  (the default skips tests named *keychain*).
# Honours CARGO_TARGET_DIR and GHI_SIM_UDID (default: iPhone 17 Pro, iOS 26.3).
set -euo pipefail

root="$(cd "$(dirname "$0")/../../.." && pwd)"
sim="$root/apps/mobile/scripts/sim.sh"

pkgs=()
while [[ $# -gt 0 && "$1" != -- ]]; do
  case "$1" in
    -p) pkgs+=(-p "${2:?-p needs a crate}"); shift 2 ;;
    *) echo "usage: $0 [-p crate]... [-- test args]" >&2; exit 2 ;;
  esac
done
[[ "${1:-}" == -- ]] && shift
[[ ${#pkgs[@]} -gt 0 ]] || pkgs=(-p ghi-store)
[[ $# -gt 0 ]] || set -- --skip keychain

# sim.sh resolves and validates the UDID (refuses non-simulators); `container`
# is not needed, so ask `boot` for the state instead.
udid="${GHI_SIM_UDID:-}"
if [[ -z "$udid" ]]; then
  udid="$(xcrun simctl list devices available -j | python3 -c '
import json, sys
for rt, devs in json.load(sys.stdin)["devices"].items():
    if rt.endswith("iOS-26-3"):
        for d in devs:
            if d["name"] == "iPhone 17 Pro":
                print(d["udid"]); raise SystemExit
')"
fi
[[ -n "$udid" ]] || { echo "no iPhone 17 Pro / iOS 26.3 simulator; set GHI_SIM_UDID" >&2; exit 1; }
GHI_SIM_UDID="$udid" "$sim" boot >/dev/null

rustup target add aarch64-apple-ios-sim >/dev/null
cd "$root"
export CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUNNER="xcrun simctl spawn $udid"
cargo test --target aarch64-apple-ios-sim "${pkgs[@]}" -- "$@"
