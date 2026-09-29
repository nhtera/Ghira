#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Smoke build of the pinned NeMo-Speech.cpp submodule (CPU, ASR + diarization only).
# Phase 1 only proves it compiles on each CI OS; the FFI is proven in phase 3 (spike S6).
# Only the ggml nested submodule is fetched: kenlm (LGPL), llama.cpp, TTS and gRPC
# dependencies stay out, matching the Apache-2.0 dependency policy.
# GGML_NATIVE=OFF: portable build, and ggml's native probe hangs on its SVE
# try-run on Apple Silicon.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
src="$root/third_party/NeMo-Speech.cpp"
# CI points NEMO_BUILD_ROOT at the runner temp dir so it stays out of the cargo cache.
out="${NEMO_BUILD_ROOT:-$root/target}"
build="$out/nemo-smoke"
jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"

git -C "$src" submodule update --init --depth 1 ggml

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
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5
  cmake --build "$spm_src/build" --config Release --parallel "$jobs"
  cmake --install "$spm_src/build" --config Release
  echo "$spm_commit" > "$spm_prefix/.commit"
fi

cmake -S "$src" -B "$build" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_PREFIX_PATH="$spm_prefix" \
  -DGGML_NATIVE=OFF \
  -DNEMO_SPEECH_GGML_PATCHED=OFF \
  -DNEMO_SPEECH_BUILD_ASR=ON \
  -DNEMO_SPEECH_BUILD_DIAR=ON \
  -DNEMO_SPEECH_BUILD_TTS=OFF \
  -DNEMO_SPEECH_BUILD_NMT=OFF \
  -DNEMO_SPEECH_BUILD_CLI=ON \
  -DNEMO_SPEECH_BUILD_MIC_CAPTURE=OFF \
  -DNEMO_SPEECH_BUILD_HTTP=OFF \
  -DNEMO_SPEECH_BUILD_GRPC=OFF \
  -DNEMO_SPEECH_WITH_FLASHLIGHT=OFF

cmake --build "$build" --config Release --parallel "$jobs"
