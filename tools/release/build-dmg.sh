#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Builds the macOS app bundle and its DMG, the way release.yml does, and
# checks the result. Usage:
#
#   tools/release/build-dmg.sh --adhoc   # local test build: ad-hoc signature,
#                                        # no hardened runtime, no notarization
#   tools/release/build-dmg.sh           # Developer ID: needs the APPLE_* env
#                                        # (APPLE_SIGNING_IDENTITY, APPLE_API_KEY,
#                                        # APPLE_API_ISSUER, APPLE_API_KEY_PATH)
#
# Output in target/release/dist/: Ghira_<version>_aarch64.dmg, the .app as
# .app.zip (the update archive, made after stapling), and
# update-manifest.unsigned.json (version, sha256, size) for the owner to sign
# offline. An ad-hoc build can't be notarized and has no Team ID: it tests the
# bundle layout, rpaths, the worker sidecar, TCC prompts, notifications, file
# associations and Dock drop — not Gatekeeper (owner list, phase 12).
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
desktop="$root/apps/desktop"
adhoc=0
[[ "${1:-}" == "--adhoc" ]] && adhoc=1
version="$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "$desktop/src-tauri/tauri.conf.json")"
out="$root/target/release/dist"
mkdir -p "$out"

"$root/tools/release/stage-bundle.sh"

configs=(--config src-tauri/tauri.release.conf.json)
if (( adhoc )); then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  # Ad-hoc dylibs have no Team ID: library validation under the hardened
  # runtime would refuse them, so the local build runs without it.
  cat > "$tmp/adhoc.json" <<'JSON'
{ "bundle": { "macOS": { "signingIdentity": "-", "hardenedRuntime": false } } }
JSON
  configs+=(--config "$tmp/adhoc.json")
elif [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  echo "build-dmg: set APPLE_SIGNING_IDENTITY (and the notarization APPLE_API_*), or use --adhoc" >&2
  exit 1
fi

echo "== tauri build (nemo)"
(cd "$desktop" && pnpm tauri build --features nemo "${configs[@]}" -- --locked)

app="$root/target/release/bundle/macos/Ghira.app"
[[ -d "$app" ]] || { echo "build-dmg: no $app" >&2; exit 1; }

echo "== checks"
bin="$app/Contents/MacOS/ghi-desktop"
otool -l "$bin" | grep -A2 LC_RPATH | grep -q "@executable_path/../Frameworks" \
  || { echo "build-dmg: the app has no Frameworks rpath" >&2; exit 1; }
if otool -l "$bin" | grep -A2 LC_RPATH | grep -q "$root"; then
  echo "build-dmg: the app carries a build-machine rpath" >&2
  exit 1
fi
[[ -x "$app/Contents/MacOS/ghi-llm-worker" ]] || { echo "build-dmg: no worker in the bundle" >&2; exit 1; }
for lib in "$root"/apps/desktop/src-tauri/frameworks/*.dylib; do
  [[ -f "$app/Contents/Frameworks/$(basename "$lib")" ]] \
    || { echo "build-dmg: $(basename "$lib") missing from Frameworks" >&2; exit 1; }
done
codesign --verify --deep --strict --verbose=2 "$app"
codesign -d --entitlements - "$app" 2>/dev/null | grep -q "audio-input" \
  || { echo "build-dmg: the app lacks the audio-input entitlement" >&2; exit 1; }

dmg="$out/Ghira_${version}_aarch64.dmg"
rm -f "$dmg"
echo "== DMG"
stage="$(mktemp -d)"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "Ghira" -srcfolder "$stage" -ov -format UDZO "$dmg" >/dev/null
rm -rf "$stage"

if (( ! adhoc )); then
  echo "== sign + notarize the DMG"
  codesign --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$dmg"
  xcrun notarytool submit "$dmg" --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" \
    --issuer "$APPLE_API_ISSUER" --wait
  xcrun stapler staple "$dmg"
  spctl --assess --type open --context context:primary-signature --verbose "$dmg"
fi

echo "== update archive"
archive="$out/Ghira_${version}_aarch64.app.zip"
rm -f "$out"/*.app.tar.gz "$archive"
# ditto keeps the signature and the stapled ticket (the app unpacks it with ditto).
ditto -c -k --sequesterRsrc --keepParent "$app" "$archive"
sha="$(shasum -a 256 "$archive" | cut -d' ' -f1)"
size="$(stat -f %z "$archive")"
cat > "$out/update-manifest.unsigned.json" <<JSON
{
  "version": "$version",
  "min_macos": "14.2",
  "archive": { "file": "$(basename "$archive")", "sha256": "$sha", "size": $size },
  "dmg": "$(basename "$dmg")"
}
JSON
ls -lh "$out"
