#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs a 16 kHz mono WAV through the speech engine on a connected iPhone and
# prints the result (phase 7 go/no-go: NeMo-Speech.cpp on iOS, throughput).
#
#   apps/mobile/scripts/selftest-ios.sh <file.wav> [device-id]
#
# The phone must be unlocked (the engine only uses the GPU in the foreground).
# Relaunches the app: a recording in progress is stopped.
# Models first: apps/mobile/scripts/push-models.sh.
set -euo pipefail

wav="${1:?usage: $0 <file.wav> [device-id]}"
device="${2:-}"
bundle=com.nhtera.ghira.spike
if [[ -z "$device" ]]; then
  device="$(xcrun devicectl list devices 2>/dev/null | grep -E 'available \(paired\)|connected' | grep 'iPhone' |
    grep -oE '[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}' | head -1 || true)"
fi
[[ -n "$device" ]] || { echo "no paired iPhone found" >&2; exit 1; }
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
# devicectl <3-word subcommand> --device <id> <options...> (the device must
# come before a launch's bundle id: later arguments go to the app).
dc() { xcrun devicectl "$1" "$2" "$3" --device "$device" "${@:4}" >/dev/null; }
files() {
  xcrun devicectl device info files --device "$device" --domain-type appDataContainer \
    --domain-identifier "$bundle" 2>/dev/null | awk '{print $1}' | grep '^Documents/selftest-.*\.json$' || true
}

name="$(basename "$wav")"
dc device copy to --domain-type appDataContainer --domain-identifier "$bundle" \
  --source "$wav" --destination "Documents/$name"
before="$(files)"
dc device process launch --terminate-existing \
  --environment-variables "{\"GHI_SELFTEST\": \"$name\"}" "$bundle"
echo "running $name on the phone..."
for _ in $(seq 1 180); do
  sleep 2
  new="$(comm -13 <(echo "$before" | sort) <(files | sort) | head -1)"
  if [[ -n "$new" ]]; then
    dc device copy from --domain-type appDataContainer --domain-identifier "$bundle" \
      --source "$new" --destination "$out/result.json"
    cat "$out/result.json"
    echo
    exit 0
  fi
done
echo "no result after 6 minutes" >&2
exit 1
