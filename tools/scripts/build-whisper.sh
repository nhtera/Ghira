#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Builds the pinned whisper.cpp (final-pass ASR + its Silero VAD) as STATIC
# libraries into $WHISPER_BUILD_ROOT/install (default target/whisper/install).
# `ghi-speech` links them with `--features whisper` (see crates/ghi-speech/build.rs).
#
# - Static, with its own bundled ggml: NeMo-Speech.cpp ships a *patched* ggml as
#   @rpath/libggml.0.dylib, so a second ggml must never be a shared library
#   (same install name, different ABI). Static code binds inside the executable.
# - Metal on macOS arm64, with the shader library embedded (no .metallib file
#   to ship or locate), CPU elsewhere.
# - Symbols hidden: nothing from ggml/whisper is exported from the final binary
#   (tools/scripts/check-no-ggml-export.sh).
# - No examples, server, tests or curl: whisper.cpp's own tools download models.
# - GGML_NATIVE=OFF: portable binaries (as in build-nemo.sh).
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
src="$root/third_party/whisper.cpp"
out="${WHISPER_BUILD_ROOT:-$root/target/whisper}"
build="$out/build"
prefix="$out/install"
jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"

if [[ ! -f "$src/CMakeLists.txt" ]]; then
  git -C "$root" submodule update --init --depth 1 third_party/whisper.cpp
fi

metal=OFF
extra=()
if [[ "$(uname -s)" == "Darwin" ]]; then
  extra+=(-DCMAKE_OSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.2}")
  if [[ "$(uname -m)" == "arm64" ]]; then metal=ON; fi
fi

cmake -S "$src" -B "$build" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$prefix" \
  -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
  -DCMAKE_C_VISIBILITY_PRESET=hidden \
  -DCMAKE_CXX_VISIBILITY_PRESET=hidden \
  -DCMAKE_OBJC_VISIBILITY_PRESET=hidden \
  -DCMAKE_VISIBILITY_INLINES_HIDDEN=ON \
  -DBUILD_SHARED_LIBS=OFF \
  -DGGML_NATIVE=OFF \
  -DGGML_METAL="$metal" \
  -DGGML_METAL_EMBED_LIBRARY="$metal" \
  -DGGML_CCACHE=OFF \
  -DWHISPER_BUILD_EXAMPLES=OFF \
  -DWHISPER_BUILD_TESTS=OFF \
  -DWHISPER_BUILD_SERVER=OFF \
  -DWHISPER_CURL=OFF \
  -DWHISPER_SDL2=OFF \
  "${extra[@]}"

# RT-6: no network code in the linked library.
for opt in WHISPER_CURL WHISPER_BUILD_SERVER WHISPER_BUILD_EXAMPLES; do
  if ! grep -q "^$opt:BOOL=OFF$" "$build/CMakeCache.txt"; then
    echo "error: $opt must be OFF" >&2
    exit 1
  fi
done

cmake --build "$build" --config Release --parallel "$jobs"
cmake --install "$build" --config Release
# The licence texts the About screen and THIRD_PARTY notices read.
mkdir -p "$prefix/share/licenses/whisper.cpp"
cp "$src/LICENSE" "$prefix/share/licenses/whisper.cpp/LICENSE"
echo "whisper.cpp installed in $prefix (metal=$metal)"
