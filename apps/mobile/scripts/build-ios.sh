#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Builds the iOS spike app (phase 7) with the speech engine.
#
#   apps/mobile/scripts/build-ios.sh [--release] [--sim [--test-hooks]]
#
# - Device (default): signed with $APPLE_DEVELOPMENT_TEAM, or the first paid
#   team Xcode knows. The team only goes into the generated Xcode project,
#   which is not committed.
# - --sim: the simulator (no signing; a placeholder team id is used).
# - --test-hooks (with --sim only): the cargo feature `test-hooks` + Swift
#   condition GHI_TEST_HOOKS (fake mic, Darwin-notification triggers).
# - CARGO_TARGET_DIR is honoured (set it to keep clear of other builds).
# - Builds NeMo-Speech.cpp for iOS first if needed (tools/scripts/build-nemo-ios.sh).
set -euo pipefail

root="$(cd "$(dirname "$0")/../../.." && pwd)"
mobile="$root/apps/mobile"
profile=(--debug)
target=aarch64
hooks=0
for arg in "$@"; do
  case "$arg" in
    --release) profile=() ;;
    --sim) target=aarch64-sim ;;
    --test-hooks) hooks=1 ;;
    *) echo "usage: $0 [--release] [--sim [--test-hooks]]" >&2; exit 2 ;;
  esac
done
if [[ $hooks == 1 && ("$target" != aarch64-sim || ${#profile[@]} == 0) ]]; then
  echo "--test-hooks is for the simulator debug build only (--sim, no --release)" >&2
  exit 2
fi
features=nemo
export GHI_SWIFT_TEST_HOOKS=
if [[ $hooks == 1 ]]; then
  features=nemo,test-hooks
  export GHI_SWIFT_TEST_HOOKS=GHI_TEST_HOOKS
fi

rustup target add aarch64-apple-ios aarch64-apple-ios-sim >/dev/null
# (Re)build NeMo-Speech.cpp when the slice we need is missing or the pin moved.
slice=ios-arm64; [[ "$target" == aarch64-sim ]] && slice=ios-arm64-simulator
nemo="$root/target/nemo-ios"
pin="$(git -C "$root/third_party/NeMo-Speech.cpp" rev-parse HEAD)"
if [[ ! -d "$nemo/nemo_speech_asr_c.xcframework/$slice" || "$(cat "$nemo/.pin" 2>/dev/null)" != "$pin" ]]; then
  "$root/tools/scripts/build-nemo-ios.sh"
fi

if [[ "$target" == aarch64 && -z "${APPLE_DEVELOPMENT_TEAM:-}" ]]; then
  APPLE_DEVELOPMENT_TEAM="$(defaults read com.apple.dt.Xcode IDEProvisioningTeamByIdentifier 2>/dev/null |
    awk '/isFreeProvisioningTeam = 0/ {paid = 1} /teamID =/ {if (paid) {gsub(/[;" ]/, "", $3); print $3; exit}}')"
  if [[ -z "$APPLE_DEVELOPMENT_TEAM" ]]; then
    echo "set APPLE_DEVELOPMENT_TEAM (a paid Apple Developer team: the Live Activity needs one)" >&2
    exit 1
  fi
fi
# The simulator needs no signing, but the Tauri CLI wants a team id.
export APPLE_DEVELOPMENT_TEAM="${APPLE_DEVELOPMENT_TEAM:-0000000000}"

(cd "$mobile/src-tauri/gen/apple" && xcodegen generate --quiet)
cd "$mobile"
CI=true pnpm tauri ios build ${profile[@]+"${profile[@]}"} --features "$features" --target "$target"
find "$mobile/src-tauri/gen/apple/build" -maxdepth 3 \( -name '*.ipa' -o -name '*.app' \) -newer "$mobile/src-tauri/gen/apple/project.yml" -print
