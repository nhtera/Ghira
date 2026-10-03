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
# Honours CARGO_TARGET_DIR and GHI_SIM_UDID (default: iPhone 17 Pro on iOS 26.3, see sim.sh).
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

# sim.sh resolves and validates the simulator ($GHI_SIM_UDID when set, whatever its
# runtime; refuses non-simulators).
udid="$("$sim" udid)"
export GHI_SIM_UDID="$udid"
"$sim" boot >/dev/null

rustup target add aarch64-apple-ios-sim >/dev/null
cd "$root"
export CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUNNER="xcrun simctl spawn $udid"
cargo test --target aarch64-apple-ios-sim "${pkgs[@]}" -- "$@"
