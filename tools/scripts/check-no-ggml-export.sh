#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# The static whisper.cpp must stay private to the executable: NeMo ships its own
# patched ggml as a dylib, and two ggml symbol sets visible to the dynamic
# linker is how they would collide. Fails if a binary exports ggml_*, gguf_* or
# whisper_* symbols (macOS/Linux; `nm -gU`).
# Usage: check-no-ggml-export.sh <binary>...   (e.g. target/release/ghi)
set -euo pipefail

[[ $# -gt 0 ]] || { echo "usage: $0 <binary>..." >&2; exit 2; }
status=0
for bin in "$@"; do
  if [[ ! -f "$bin" ]]; then
    echo "error: $bin not found (build it first)" >&2
    status=1
  elif nm -gU "$bin" 2>/dev/null | grep -Eq ' [TDSBAtdsb] _?(ggml|gguf|whisper|ghi_wh)_'; then
    echo "error: $bin exports ggml/whisper symbols:" >&2
    nm -gU "$bin" | grep -E ' _?(ggml|gguf|whisper|ghi_wh)_' | head -5 >&2
    status=1
  fi
done
[[ $status -eq 0 ]] && echo "no ggml export: ok ($#)"
exit $status
