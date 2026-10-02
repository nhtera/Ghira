#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Mirror the pinned models to a Cloudflare R2 bucket (phase 12 step 8; doc 05 section 4:
# "Hugging Face at a pinned revision + SHA-256, with our R2 mirror as fallback").
#
#   tools/release/mirror-models.sh [--apply] [--models-dir DIR] [--licenses DIR]
#                                  [--only ID[,ID...]] [--tool auto|rclone|aws]
#
# DRY RUN BY DEFAULT: verifies every model file against crates/ghi-models/registry.toml
# (size + SHA-256) and prints what would be uploaded. --apply uploads. Nothing is uploaded
# unless the SHA-256 matches the registry, so a corrupt or swapped local file can never be
# published.
#
# Layout (matches crates/ghi-models/src/download.rs: mirror = "<base>/<file>"):
#   <prefix>/<file>                    the model file, e.g. Qwen3-4B-Q4_K_M.gguf
#   <prefix>/<id>/LICENSE, NOTICE      licence text and notice (redistribution requires them)
#   <prefix>/<id>/SOURCE.txt           repo, pinned revision, SHA-256, size (generated)
#   <prefix>/manifest.json             the same for all models (generated)
# Licence files come from --licenses DIR laid out as DIR/<id>/LICENSE and DIR/<id>/NOTICE
# (OpenMDW-1.1 and Apache-2.0 texts are not stored in this repo; take them from each model
# card at the pinned revision). A model without them is refused with --apply.
#
# Environment (never on the command line, never echoed)
#   R2_ACCOUNT_ID  R2_ACCESS_KEY_ID  R2_SECRET_ACCESS_KEY  R2_BUCKET   required with --apply
#   R2_PREFIX      key prefix, default "models"
#   R2_PUBLIC_BASE public https base (custom domain) used only to print the registry line
# Tools: rclone (default if present) or aws (S3 API against the R2 endpoint).
# After uploading, put `mirrors = ["<R2_PUBLIC_BASE>/<prefix>"]` into each model of
# crates/ghi-models/registry.toml (the host is then allowlisted by ghi-net automatically).
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
registry="$root/crates/ghi-models/registry.toml"
dir="${GHI_MODELS_DIR:-$root/models}" lic="" only="" tool=auto apply=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --apply) apply=1; shift ;;
    --models-dir) dir="$2"; shift 2 ;;
    --licenses) lic="$2"; shift 2 ;;
    --only) only="$2"; shift 2 ;;
    --tool) tool="$2"; shift 2 ;;
    -h|--help) sed -n '3,30p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
size_of() { stat -f %z "$1" 2>/dev/null || stat -c %s "$1"; }

# One "id file sha256 size repo revision license" line per [[model]] table.
models() {
  awk -F' = ' '
    /^\[\[model\]\]/ { if (id) print id, file, sha, size, repo, rev, lic; id=file=sha=size=repo=rev=lic="" }
    $1=="id" {id=$2} $1=="file" {file=$2} $1=="sha256" {sha=$2} $1=="size" {size=$2}
    $1=="repo" {repo=$2} $1=="revision" {rev=$2} $1=="license" {lic=$2}
    END { if (id) print id, file, sha, size, repo, rev, lic }' "$registry" | tr -d '"'
}

prefix="${R2_PREFIX:-models}"
if (( apply )); then
  for v in R2_ACCOUNT_ID R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY R2_BUCKET; do
    [[ -n "${!v:-}" ]] || { echo "error: $v is not set (required with --apply)" >&2; exit 2; }
  done
  if [[ "$tool" == auto ]]; then
    if command -v rclone >/dev/null; then tool=rclone; elif command -v aws >/dev/null; then tool=aws
    else echo "error: neither rclone nor aws is installed" >&2; exit 2; fi
  fi
  endpoint="https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com"
fi

put() { # local-file key
  case "$tool" in
    rclone)
      RCLONE_S3_PROVIDER=Cloudflare RCLONE_S3_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID" \
      RCLONE_S3_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY" RCLONE_S3_ENDPOINT="$endpoint" \
        rclone copyto --s3-no-check-bucket --checksum "$1" ":s3:${R2_BUCKET}/$2" ;;
    aws)
      AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID" AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY" AWS_DEFAULT_REGION=auto \
        aws s3 cp --endpoint-url "$endpoint" "$1" "s3://${R2_BUCKET}/$2" ;;
  esac
}
remote_size() { # key
  case "$tool" in
    rclone)
      RCLONE_S3_PROVIDER=Cloudflare RCLONE_S3_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID" \
      RCLONE_S3_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY" RCLONE_S3_ENDPOINT="$endpoint" \
        rclone lsjson --s3-no-check-bucket ":s3:${R2_BUCKET}/$1" | python3 -c 'import json,sys;print(json.load(sys.stdin)[0]["Size"])' ;;
    aws)
      AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID" AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY" AWS_DEFAULT_REGION=auto \
        aws s3api head-object --endpoint-url "$endpoint" --bucket "$R2_BUCKET" --key "$1" --query ContentLength --output text ;;
  esac
}

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
status=0 count=0 manifest=()
while read -r id file sha size repo rev license; do
  if [[ -n "$only" && ",$only," != *",$id,"* ]]; then continue; fi
  count=$((count + 1))
  path="$dir/$file"
  if [[ ! -f "$path" ]]; then echo "MISSING  $id ($path)"; status=1; continue; fi
  if [[ "$(size_of "$path")" != "$size" ]]; then echo "BAD-SIZE $id"; status=1; continue; fi
  if [[ "$(sha256 "$path")" != "$sha" ]]; then echo "BAD-SHA  $id: does not match the registry, not publishing"; status=1; continue; fi
  echo "verified $id  $file  ($size bytes, sha256 ${sha:0:12}..., $license)"
  have_lic=1
  for f in LICENSE NOTICE; do [[ -n "$lic" && -f "$lic/$id/$f" ]] || have_lic=0; done
  if (( ! have_lic )); then
    echo "  warning: $id has no $lic/$id/{LICENSE,NOTICE}; --apply refuses it" >&2
    (( apply )) && { status=1; continue; }
  fi
  printf 'id: %s\nrepo: %s\nrevision: %s\nfile: %s\nsha256: %s\nsize: %s\nlicense: %s\n' \
    "$id" "$repo" "$rev" "$file" "$sha" "$size" "$license" > "$tmp/$id.SOURCE.txt"
  manifest+=("{\"id\":\"$id\",\"file\":\"$file\",\"sha256\":\"$sha\",\"size\":$size,\"repo\":\"$repo\",\"revision\":\"$rev\",\"license\":\"$license\"}")
  if (( apply )); then
    put "$path" "$prefix/$file"
    put "$lic/$id/LICENSE" "$prefix/$id/LICENSE"
    put "$lic/$id/NOTICE" "$prefix/$id/NOTICE"
    put "$tmp/$id.SOURCE.txt" "$prefix/$id/SOURCE.txt"
    got="$(remote_size "$prefix/$file")"
    if [[ "$got" == "$size" ]]; then echo "  uploaded $prefix/$file (remote size $got ok)"; else echo "  FAIL: remote size $got != $size" >&2; status=1; fi
  else
    echo "  would upload $prefix/$file, $prefix/$id/{LICENSE,NOTICE,SOURCE.txt}"
  fi
done < <(models)
(( count > 0 )) || { echo "error: no model selected" >&2; exit 2; }
if (( ${#manifest[@]} )); then
  ( IFS=,; printf '{"schema":"ghi.model-mirror/1","models":[%s]}\n' "${manifest[*]}" ) > "$tmp/manifest.json"
  if (( apply )) && (( status == 0 )); then put "$tmp/manifest.json" "$prefix/manifest.json"; else echo "  would upload $prefix/manifest.json"; fi
fi
if [[ -n "${R2_PUBLIC_BASE:-}" ]]; then echo "registry line: mirrors = [\"${R2_PUBLIC_BASE%/}/$prefix\"]"; fi
(( apply )) || echo "dry run: nothing uploaded (pass --apply with R2_* set)"
exit "$status"
