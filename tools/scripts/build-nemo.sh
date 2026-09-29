#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Builds the pinned NeMo-Speech.cpp (ASR + diarization only) and installs its
# shared C library into $NEMO_BUILD_ROOT/install (default target/nemo/install).
# `ghi-speech` links it with `--features nemo` (see crates/ghi-speech/build.rs).
#
# - Backend: Metal on macOS arm64, CPU elsewhere (Vulkan on Windows is phase 13).
# - ggml gets NeMo-Speech.cpp's own patch series, as upstream builds it.
# - Only the ggml nested submodule is fetched: kenlm (LGPL), llama.cpp, TTS,
#   HTTP and gRPC stay out, matching the Apache-2.0 dependency policy.
# - GGML_NATIVE=OFF: portable binaries, and ggml's native probe hangs on its
#   SVE try-run on Apple Silicon.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
src="$root/third_party/NeMo-Speech.cpp"
# CI points NEMO_BUILD_ROOT at the runner temp dir so it stays out of the cargo cache.
out="${NEMO_BUILD_ROOT:-$root/target/nemo}"
build="$out/build"
prefix="$out/install"
jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"

metal=OFF
if [[ "$(uname -s)" == "Darwin" && "$(uname -m)" == "arm64" ]]; then metal=ON; fi

git -C "$src" submodule update --init --depth 1 ggml
"$src/scripts/apply-ggml-patches.sh"

# SentencePiece (Apache-2.0) is required by ASR. Build it statically at the
# commit NeMo-Speech.cpp itself pins, so no brew/vcpkg setup is needed.
spm_commit="$(sed -n 's/^COMMIT=//p' "$src/scripts/build_sentencepiece_static.sh")"
spm_src="$out/sentencepiece-src"
spm_prefix="$out/sentencepiece"
if [[ ! -f "$spm_prefix/.commit" || "$(cat "$spm_prefix/.commit")" != "$spm_commit" ]]; then
  rm -rf "$spm_src" "$spm_prefix"
  git init -q "$spm_src"
  git -C "$spm_src" fetch -q --depth 1 https://github.com/google/sentencepiece.git "$spm_commit"
  git -C "$spm_src" checkout -q FETCH_HEAD
  cmake -S "$spm_src" -B "$spm_src/build" -DCMAKE_BUILD_TYPE=Release \
    -DSPM_ENABLE_SHARED=OFF -DSPM_BUILD_TEST=OFF -DCMAKE_INSTALL_PREFIX="$spm_prefix" \
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5 -DCMAKE_POSITION_INDEPENDENT_CODE=ON
  cmake --build "$spm_src/build" --config Release --parallel "$jobs"
  cmake --install "$spm_src/build" --config Release
  echo "$spm_commit" > "$spm_prefix/.commit"
fi

cmake -S "$src" -B "$build" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$prefix" \
  -DCMAKE_PREFIX_PATH="$spm_prefix" \
  -DGGML_NATIVE=OFF \
  -DGGML_METAL="$metal" \
  -DGGML_METAL_EMBED_LIBRARY="$metal" \
  -DNEMO_SPEECH_GGML_PATCHED=ON \
  -DNEMO_SPEECH_BUILD_ASR=ON \
  -DNEMO_SPEECH_BUILD_DIAR=ON \
  -DNEMO_SPEECH_BUILD_TTS=OFF \
  -DNEMO_SPEECH_BUILD_NMT=OFF \
  -DNEMO_SPEECH_BUILD_S2S=OFF \
  -DNEMO_SPEECH_BUILD_CLI=OFF \
  -DNEMO_SPEECH_BUILD_MIC_CAPTURE=OFF \
  -DNEMO_SPEECH_BUILD_HTTP=OFF \
  -DNEMO_SPEECH_BUILD_GRPC=OFF \
  -DNEMO_SPEECH_WITH_FLASHLIGHT=OFF

# RT-6: no network code in the linked library.
# The upstream CLI is off too: it downloads models by running curl.
for opt in NEMO_SPEECH_BUILD_HTTP NEMO_SPEECH_BUILD_GRPC NEMO_SPEECH_WITH_FLASHLIGHT NEMO_SPEECH_BUILD_CLI; do
  if ! grep -q "^$opt:BOOL=OFF$" "$build/CMakeCache.txt"; then
    echo "error: $opt must be OFF" >&2
    exit 1
  fi
done

cmake --build "$build" --config Release --parallel "$jobs"
cmake --install "$build" --config Release
echo "NeMo-Speech.cpp installed in $prefix (metal=$metal)"
