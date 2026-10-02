#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Offline-install package of the pinned models (phase 12 step 8; doc 05 section 4: "offline
# install from a file").
#
#   tools/release/offline-models.sh [--models-dir DIR] [--out DIR] [--only ID[,ID...]]
#   tools/release/offline-models.sh --check PACKAGE.tar
#
# Builds ghira-models-<UTC date>.tar from the pinned files of crates/ghi-models/registry.toml.
# Every file's size and SHA-256 are verified against the registry first; a mismatch aborts, so
# the package can only hold registry-exact files. Contents (GGUF does not compress, so a plain
# tar):
#   models/<file>      the model files, under their registry names
#   manifest.json      ghi.model-package/1: id, role, file, sha256, size, license, repo, revision
#   SHA256SUMS         `shasum -a 256 -c` compatible
#   INSTALL.txt        how to install
#
# Install on the offline Mac (what the app accepts, see crates/ghi-models/src/verify.rs
# `import_file`): extract, then in the app Settings > Models > "Install from file..." for each
# file, or `ghi models import models/<file>` for each. The importer hashes the file, requires the
# hash to be a registry entry's (anything else is refused with bad_input), copies it to
# <models dir>/<file>.import and renames it into place only after the copy hashes the same.
# Nothing here is trusted: the manifest is informational.
#
# --check PACKAGE.tar: unpack to a temp dir and verify manifest, SHA256SUMS and the registry.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
registry="$root/crates/ghi-models/registry.toml"
dir="${GHI_MODELS_DIR:-$root/models}" out="$PWD" only="" check=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --models-dir) dir="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --only) only="$2"; shift 2 ;;
    --check) check="$2"; shift 2 ;;
    -h|--help) sed -n '3,30p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
size_of() { stat -f %z "$1" 2>/dev/null || stat -c %s "$1"; }
models() {
  awk -F' = ' '
    /^\[\[model\]\]/ { if (id) print id, role, file, sha, size, repo, rev, lic; id=role=file=sha=size=repo=rev=lic="" }
    $1=="id" {id=$2} $1=="role" {role=$2} $1=="file" {file=$2} $1=="sha256" {sha=$2} $1=="size" {size=$2}
    $1=="repo" {repo=$2} $1=="revision" {rev=$2} $1=="license" {lic=$2}
    END { if (id) print id, role, file, sha, size, repo, rev, lic }' "$registry" | tr -d '"'
}

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

if [[ -n "$check" ]]; then
  tar -xf "$check" -C "$tmp"
  [[ -f "$tmp/manifest.json" && -f "$tmp/SHA256SUMS" ]] || { echo "error: not a model package" >&2; exit 1; }
  (cd "$tmp" && if command -v sha256sum >/dev/null; then sha256sum -c SHA256SUMS; else shasum -a 256 -c SHA256SUMS; fi)
  status=0
  while read -r id role file sha size repo rev license; do
    [[ -f "$tmp/models/$file" ]] || continue
    if [[ "$(sha256 "$tmp/models/$file")" == "$sha" && "$(size_of "$tmp/models/$file")" == "$size" ]]; then
      echo "registry ok  $id"
    else echo "registry MISMATCH  $id" >&2; status=1; fi
  done < <(models)
  exit "$status"
fi

mkdir -p "$tmp/models" "$out"
entries=() count=0
while read -r id role file sha size repo rev license; do
  if [[ -n "$only" && ",$only," != *",$id,"* ]]; then continue; fi
  src="$dir/$file"
  [[ -f "$src" ]] || { echo "error: $src is missing (tools/scripts/fetch-models.sh $id)" >&2; exit 1; }
  [[ "$(size_of "$src")" == "$size" ]] || { echo "error: $id: size differs from the registry" >&2; exit 1; }
  [[ "$(sha256 "$src")" == "$sha" ]] || { echo "error: $id: SHA-256 differs from the registry" >&2; exit 1; }
  ln -s "$src" "$tmp/models/$file"
  entries+=("{\"id\":\"$id\",\"role\":\"$role\",\"file\":\"models/$file\",\"sha256\":\"$sha\",\"size\":$size,\"license\":\"$license\",\"repo\":\"$repo\",\"revision\":\"$rev\"}")
  echo "$sha  models/$file" >> "$tmp/SHA256SUMS"
  echo "verified $id"
  count=$((count + 1))
done < <(models)
(( count > 0 )) || { echo "error: no model selected" >&2; exit 2; }
( IFS=,; printf '{"schema":"ghi.model-package/1","models":[%s]}\n' "${entries[*]}" ) > "$tmp/manifest.json"
cat > "$tmp/INSTALL.txt" <<'TXT'
Ghira offline model package
1. Check:    shasum -a 256 -c SHA256SUMS
2. Install:  Ghira > Settings > Models > "Install from file..." for each file in models/
             (or: ghi models import models/<file>)
Ghira accepts only the pinned files listed in the app's registry; anything else is refused.
TXT
pkg="$out/ghira-models-$(date -u +%Y%m%d).tar"
# -h follows the symlinks so the files themselves are archived
tar -chf "$pkg" -C "$tmp" manifest.json SHA256SUMS INSTALL.txt models
echo "package: $pkg ($(size_of "$pkg") bytes, $count model(s))"
