#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Developer/CI download of the pinned models in crates/ghi-models/registry.toml
# into ./models (git-ignored). Each file is fetched at its pinned revision and
# checked against its SHA-256; a mismatch deletes the file and fails.
# Usage: tools/scripts/fetch-models.sh [model-id ...]   (default: all)
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
registry="$root/crates/ghi-models/registry.toml"
dest="${GHI_MODELS_DIR:-$root/models}"
mkdir -p "$dest"

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

# Flatten each [[model]] table into one "id repo revision file sha256" line.
awk -F' = ' '
  /^\[\[model\]\]/ { if (id) print id, repo, rev, file, sha; id=repo=rev=file=sha="" }
  $1=="id" {id=$2} $1=="repo" {repo=$2} $1=="revision" {rev=$2} $1=="file" {file=$2} $1=="sha256" {sha=$2}
  END { if (id) print id, repo, rev, file, sha }' "$registry" | tr -d '"' |
while read -r id repo rev file sha; do
  if [[ $# -gt 0 && ! " $* " == *" $id "* ]]; then continue; fi
  path="$dest/$file"
  if [[ -f "$path" && "$(sha256 "$path")" == "$sha" ]]; then echo "ok      $id"; continue; fi
  echo "fetch   $id ($repo@${rev:0:8})"
  curl -fL --proto '=https' --proto-redir '=https' --retry 3 -o "$path.part" "https://huggingface.co/$repo/resolve/$rev/$file"
  if [[ "$(sha256 "$path.part")" != "$sha" ]]; then
    rm -f "$path.part"; echo "error: SHA-256 mismatch for $file" >&2; exit 1
  fi
  mv "$path.part" "$path"
  echo "ok      $id"
done
