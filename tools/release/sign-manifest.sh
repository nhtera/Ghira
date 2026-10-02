#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Owner-only, offline: turns a release's update-manifest.unsigned.json into
# the signed update feed (crates/ghi-update reads it).
#
#   tools/release/sign-manifest.sh <update-manifest.unsigned.json> \
#     --base-url https://github.com/<owner>/<repo>/releases/download/v<version> \
#     --sequence <N> --key <minisign secret key> \
#     [--channel alpha] [--days 30] [--pull <version>]... [--min-supported <version>]
#
# Writes ghira-<channel>.json and ghira-<channel>.json.minisig next to the
# input. `--sequence` must grow with every manifest you sign (the app refuses
# an older one); the signature's trusted comment carries it. `--pull` lists
# withdrawn versions (the app asks their users to update); nothing is ever
# rolled back: fix a bad release by publishing a higher version.
set -euo pipefail

in="${1:?unsigned manifest}"; shift
channel=alpha days=30 base="" seq="" key="" min="" pulls=()
while (($#)); do
  case "$1" in
    --base-url) base="$2"; shift 2 ;;
    --sequence) seq="$2"; shift 2 ;;
    --key) key="$2"; shift 2 ;;
    --channel) channel="$2"; shift 2 ;;
    --days) days="$2"; shift 2 ;;
    --pull) pulls+=("$2"); shift 2 ;;
    --min-supported) min="$2"; shift 2 ;;
    *) echo "sign-manifest: unknown option $1" >&2; exit 2 ;;
  esac
done
[[ -n "$base" && -n "$seq" && -n "$key" ]] || { echo "sign-manifest: --base-url, --sequence and --key are required" >&2; exit 2; }
[[ "$base" == https://* ]] || { echo "sign-manifest: --base-url must be https" >&2; exit 2; }
command -v minisign >/dev/null || { echo "sign-manifest: install minisign" >&2; exit 1; }

out="$(dirname "$in")/ghira-$channel.json"
python3 - "$in" "$out" "$base" "$seq" "$channel" "$days" "$min" "${pulls[@]}" <<'PY'
import json, sys, time
src, out, base, seq, channel, days, min_supported, *pulled = sys.argv[1:]
m = json.load(open(src))
feed = {
    "schema": 1,
    "sequence": int(seq),
    "channel": channel,
    "expires_at": int((time.time() + int(days) * 86400) * 1000),
    "latest": {
        "version": m["version"],
        "min_macos": m["min_macos"],
        "archive": {
            "url": f"{base.rstrip('/')}/{m['archive']['file']}",
            "sha256": m["archive"]["sha256"],
            "size": m["archive"]["size"],
        },
    },
    "pulled": pulled,
}
if min_supported:
    feed["min_supported"] = min_supported
open(out, "w").write(json.dumps(feed, indent=2) + "\n")
PY
minisign -S -s "$key" -m "$out" -x "$out.minisig" -t "ghira $channel seq=$seq v$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["latest"]["version"])' "$out")"
minisign -V -p "${key%.key}.pub" -m "$out" -x "$out.minisig" 2>/dev/null || echo "sign-manifest: (no ${key%.key}.pub next to the key to self-check)"
echo "Signed $out (sequence $seq). Upload it and $out.minisig to the release."
