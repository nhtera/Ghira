#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Stages what the macOS app bundle ships next to the app binary, for
# `tauri build --features nemo,whisper --config src-tauri/tauri.release.conf.json`:
#
# - the notes model worker (`ghi-llm-worker`, a separate process) as a Tauri
#   sidecar: src-tauri/binaries/ghi-llm-worker-<target triple>;
# - NeMo-Speech.cpp's dylibs (ASR + diarization), resolved through symlinks
#   and named as their install names say (`@rpath/libggml.0.dylib`, …), in
#   src-tauri/frameworks/ (Tauri copies them to Contents/Frameworks; the app
#   finds them through its `@executable_path/../Frameworks` rpath).
#
# Builds NeMo-Speech.cpp first if it isn't built, and whisper.cpp (linked
# statically into the app for the optional Whisper final pass: nothing to
# stage, but `--features whisper` needs it). Release builds never reuse
# a cache in CI (RT-12); locally this reuses target/.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
tauri="$root/apps/desktop/src-tauri"
triple="$(rustc -vV | sed -n 's/^host: //p')"
nemo_lib="${NEMO_BUILD_ROOT:-$root/target/nemo}/install/lib"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "stage-bundle: macOS only (Windows is phase 13)" >&2
  exit 1
fi

if [[ ! -f "$nemo_lib/libnemo_speech_asr_c.1.dylib" ]]; then
  "$root/tools/scripts/build-nemo.sh"
fi
if [[ ! -f "${WHISPER_BUILD_ROOT:-$root/target/whisper}/install/lib/libwhisper.a" ]]; then
  "$root/tools/scripts/build-whisper.sh"
fi

echo "== ghi-llm-worker ($triple)"
cargo build --release --locked -p ghi-llm-worker --manifest-path "$root/Cargo.toml"
mkdir -p "$tauri/binaries"
cp "$root/target/release/ghi-llm-worker" "$tauri/binaries/ghi-llm-worker-$triple"

echo "== NeMo-Speech.cpp dylibs"
rm -rf "$tauri/frameworks"
mkdir -p "$tauri/frameworks"
# Every dylib the C library loads, by its install name (the files that dyld
# looks for), copied as real files (Tauri skips symlinks).
want=("libnemo_speech_asr_c.1.dylib")
while read -r dep; do
  want+=("${dep#@rpath/}")
done < <(otool -L "$nemo_lib/libnemo_speech_asr_c.1.dylib" | awk '/@rpath\//{print $1}')
for name in $(printf '%s\n' "${want[@]}" | sort -u); do
  cp -L "$nemo_lib/$name" "$tauri/frameworks/$name"
  # The library's own name must match what dyld will ask for.
  id="$(otool -D "$tauri/frameworks/$name" | tail -1)"
  if [[ "$id" != "@rpath/$name" ]]; then
    echo "stage-bundle: $name has install name $id" >&2
    exit 1
  fi
done
ls -1 "$tauri/frameworks"

# The list in tauri.release.conf.json must match what was staged.
listed="$(sed -n 's/.*"frameworks\/\(lib[^"]*\)".*/\1/p' "$tauri/tauri.release.conf.json" | sort)"
staged="$(ls -1 "$tauri/frameworks" | sort)"
if [[ "$listed" != "$staged" ]]; then
  echo "stage-bundle: tauri.release.conf.json frameworks differ from the staged dylibs:" >&2
  diff <(echo "$listed") <(echo "$staged") >&2 || true
  exit 1
fi
