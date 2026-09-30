#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Builds the pinned NeMo-Speech.cpp (ASR + diarization) for iOS and packages
# it as XCFrameworks the mobile app embeds (phase 7 spike).
#
#   tools/scripts/build-nemo-ios.sh [ios|ios-sim]...   (default: both)
#
# Output in $NEMO_IOS_ROOT (default target/nemo-ios):
#   nemo_speech_asr.xcframework, nemo_speech_asr_c.xcframework, include/
#
# - Same source, pin and ggml patch series as build-nemo.sh (macOS).
# - ggml and SentencePiece are static, so only upstream's two hard-coded
#   SHARED libraries remain. They are wrapped as frameworks
#   (@rpath/<name>.framework/<name>) instead of patching upstream CMake:
#   iOS apps may embed dynamic frameworks, not loose dylibs.
# - Metal with the shader library embedded; iOS 17.0 minimum (doc 05 §6).
# - Network code stays out of the library, as on macOS (RT-6).
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
src="$root/third_party/NeMo-Speech.cpp"
out="${NEMO_IOS_ROOT:-$root/target/nemo-ios}"
min_ios=17.0
jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
platforms=("$@")
if [[ ${#platforms[@]} -eq 0 ]]; then platforms=(ios ios-sim); fi

git -C "$src" submodule update --init --depth 1 ggml
"$src/scripts/apply-ggml-patches.sh"

spm_commit="$(sed -n 's/^COMMIT=//p' "$src/scripts/build_sentencepiece_static.sh")"
spm_src="$out/sentencepiece-src"
if [[ ! -f "$spm_src/.commit" || "$(cat "$spm_src/.commit")" != "$spm_commit" ]]; then
  rm -rf "$spm_src"
  git init -q "$spm_src"
  git -C "$spm_src" fetch -q --depth 1 https://github.com/google/sentencepiece.git "$spm_commit"
  git -C "$spm_src" checkout -q FETCH_HEAD
  echo "$spm_commit" > "$spm_src/.commit"
fi

# Wraps dylib $1 as $2/<name>.framework (platform $3) with an @rpath install name.
wrap_framework() {
  local dylib="$1" dest="$2" platform_name="$3" name
  name="$(basename "$dylib" .dylib)"
  name="${name#lib}"
  name="${name%.1}"
  local fw="$dest/$name.framework"
  rm -rf "$fw"
  mkdir -p "$fw"
  cp "$dylib" "$fw/$name"
  install_name_tool -id "@rpath/$name.framework/$name" "$fw/$name"
  cat > "$fw/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>$name</string>
  <key>CFBundleIdentifier</key><string>com.nhtera.ghira.$(echo "$name" | tr '_' '-')</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>$name</string>
  <key>CFBundlePackageType</key><string>FMWK</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleSupportedPlatforms</key><array><string>$platform_name</string></array>
  <key>CFBundleVersion</key><string>1</string>
  <key>MinimumOSVersion</key><string>$min_ios</string>
</dict>
</plist>
PLIST
}

for platform in "${platforms[@]}"; do
  case "$platform" in
    ios) sysroot=iphoneos platform_name=iPhoneOS ;;
    ios-sim) sysroot=iphonesimulator platform_name=iPhoneSimulator ;;
    *) echo "unknown platform $platform (ios, ios-sim)" >&2; exit 2 ;;
  esac
  cross=(
    -DCMAKE_SYSTEM_NAME=iOS
    -DCMAKE_OSX_SYSROOT="$sysroot"
    -DCMAKE_OSX_ARCHITECTURES=arm64
    -DCMAKE_OSX_DEPLOYMENT_TARGET="$min_ios"
    -DCMAKE_BUILD_TYPE=Release
    -DCMAKE_POSITION_INDEPENDENT_CODE=ON
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5
  )

  # SentencePiece expects ios-cmake's set_xcode_property() on iOS (for its
  # command-line tools, which aren't built here); a no-op stands in.
  stub="$out/set-xcode-property.cmake"
  printf 'function(set_xcode_property)\nendfunction()\n' > "$stub"
  spm_build="$out/$platform/sentencepiece-build"
  spm_prefix="$out/$platform/sentencepiece"
  if [[ ! -f "$spm_prefix/.commit" || "$(cat "$spm_prefix/.commit")" != "$spm_commit" ]]; then
    cmake -S "$spm_src" -B "$spm_build" "${cross[@]}" \
      -DCMAKE_PROJECT_INCLUDE_BEFORE="$stub" \
      -DSPM_ENABLE_SHARED=OFF -DSPM_BUILD_TEST=OFF -DSPM_ENABLE_TCMALLOC=OFF
    # Only the library: its command-line tools can't be linked for iOS here.
    cmake --build "$spm_build" --target sentencepiece-static --parallel "$jobs"
    install -d "$spm_prefix/lib" "$spm_prefix/include"
    install -m 0644 "$spm_build/src/libsentencepiece.a" "$spm_prefix/lib/"
    install -m 0644 "$spm_src/src/sentencepiece_processor.h" "$spm_prefix/include/"
    echo "$spm_commit" > "$spm_prefix/.commit"
  fi

  build="$out/$platform/build"
  prefix="$out/$platform/install"
  cmake -S "$src" -B "$build" "${cross[@]}" \
    -DCMAKE_INSTALL_PREFIX="$prefix" \
    -DBUILD_SHARED_LIBS=OFF \
    -DSENTENCEPIECE_LIB="$spm_prefix/lib/libsentencepiece.a" \
    -DSENTENCEPIECE_INCLUDE_DIR="$spm_prefix/include" \
    -DGGML_NATIVE=OFF \
    -DGGML_METAL=ON \
    -DGGML_METAL_EMBED_LIBRARY=ON \
    -DGGML_OPENMP=OFF \
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
  for opt in NEMO_SPEECH_BUILD_HTTP NEMO_SPEECH_BUILD_GRPC NEMO_SPEECH_WITH_FLASHLIGHT NEMO_SPEECH_BUILD_CLI; do
    if ! grep -q "^$opt:BOOL=OFF$" "$build/CMakeCache.txt"; then
      echo "error: $opt must be OFF" >&2
      exit 1
    fi
  done

  cmake --build "$build" --target nemo_speech_asr_c --parallel "$jobs"
  fw="$out/$platform/frameworks"
  rm -rf "$fw"
  mkdir -p "$fw"
  wrap_framework "$build/bin/libnemo_speech_asr.dylib" "$fw" "$platform_name"
  wrap_framework "$build/bin/libnemo_speech_asr_c.1.dylib" "$fw" "$platform_name"
  install_name_tool -change "@rpath/libnemo_speech_asr.dylib" \
    "@rpath/nemo_speech_asr.framework/nemo_speech_asr" "$fw/nemo_speech_asr_c.framework/nemo_speech_asr_c"
  # Both install names and the _c → asr link must be framework paths, and
  # nothing else of ours may be linked dynamically.
  asr="$fw/nemo_speech_asr.framework/nemo_speech_asr"
  asr_c="$fw/nemo_speech_asr_c.framework/nemo_speech_asr_c"
  if ! otool -D "$asr" | grep -qx "@rpath/nemo_speech_asr.framework/nemo_speech_asr" ||
    ! otool -D "$asr_c" | grep -qx "@rpath/nemo_speech_asr_c.framework/nemo_speech_asr_c" ||
    ! otool -L "$asr_c" | grep -q "@rpath/nemo_speech_asr.framework/nemo_speech_asr " ||
    otool -L "$asr" "$asr_c" | grep -q "libggml\|libsentencepiece\|libnemo"; then
    echo "error: unexpected install names in $fw" >&2
    otool -L "$asr" "$asr_c" >&2
    exit 1
  fi
  echo "NeMo-Speech.cpp for $platform: $fw"
done

# XCFrameworks of the slices built so far (device and/or simulator).
rm -rf "$out/include"
cp -R "$src/include" "$out/include"
test -f "$out/include/nemo_speech/asr.h" || { echo "error: no C headers in $src/include" >&2; exit 1; }
for name in nemo_speech_asr nemo_speech_asr_c; do
  args=()
  for platform in ios ios-sim; do
    if [[ -d "$out/$platform/frameworks/$name.framework" ]]; then
      args+=(-framework "$out/$platform/frameworks/$name.framework")
    fi
  done
  rm -rf "$out/$name.xcframework"
  xcodebuild -create-xcframework "${args[@]}" -output "$out/$name.xcframework" >/dev/null
done
git -C "$src" rev-parse HEAD > "$out/.pin"
echo "XCFrameworks in $out"
