#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copies the pinned phone models (ASR, diarization, voice) from models/ into the
# app's models folder (Library/Application Support/Ghira/models, `Core::models`)
# on a connected iPhone, instead of the in-app download (Settings -> Models).
#
#   apps/mobile/scripts/push-models.sh [device-id]    (default: the only paired iPhone)
#
# Fetch them first with tools/scripts/fetch-models.sh (which checks SHA-256);
# the app checks the pinned size on load.
set -euo pipefail

root="$(cd "$(dirname "$0")/../../.." && pwd)"
bundle=com.nhtera.ghira
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

for file in nemotron-3.5-asr-streaming-0.6b.q8_0.gguf Nemotron-3-Diarization.q8_0.gguf \
  3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx; do
  src="$models_dir/$file"
  if [[ ! -f "$src" ]]; then
    echo "missing $src: run tools/scripts/fetch-models.sh nemotron-3.5-asr nemotron-3-diarization campplus-zh-en" >&2
    exit 1
  fi
  echo "copying $file ($(du -h "$src" | cut -f1)) to $device"
  xcrun devicectl device copy to --device "$device" \
    --domain-type appDataContainer --domain-identifier "$bundle" \
    --source "$src" --destination "Library/Application Support/Ghira/models/$file" >/dev/null
done
echo "models copied; relaunch the app (Settings -> Models shows them installed)"
