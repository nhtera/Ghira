#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copies the pinned speech models from models/ into the spike app's Documents
# on a connected iPhone (phase 7; the in-app downloader comes later).
#
#   apps/mobile/scripts/push-models.sh [device-id]    (default: the only paired iPhone)
#
# Fetch them first with tools/scripts/fetch-models.sh (which checks SHA-256);
# the app checks the pinned size on load.
set -euo pipefail

root="$(cd "$(dirname "$0")/../../.." && pwd)"
bundle=com.nhtera.ghira.spike
models_dir="${GHI_MODELS_DIR:-$root/models}"
device="${1:-}"
if [[ -z "$device" ]]; then
  device="$(xcrun devicectl list devices 2>/dev/null | grep -E 'available \(paired\)|connected' | grep 'iPhone' |
    grep -oE '[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}' | head -1 || true)"
fi
if [[ -z "$device" ]]; then
  echo "no paired iPhone found; pass a device id (xcrun devicectl list devices)" >&2
  exit 1
fi

for file in nemotron-3.5-asr-streaming-0.6b.q8_0.gguf Nemotron-3-Diarization.q8_0.gguf; do
  src="$models_dir/$file"
  if [[ ! -f "$src" ]]; then
    echo "missing $src: run tools/scripts/fetch-models.sh nemotron-3.5-asr nemotron-3-diarization" >&2
    exit 1
  fi
  echo "copying $file ($(du -h "$src" | cut -f1)) to $device"
  xcrun devicectl device copy to --device "$device" \
    --domain-type appDataContainer --domain-identifier "$bundle" \
    --source "$src" --destination "Documents/models/$file" >/dev/null
done
echo "models copied; the app picks them up on the next Record"
