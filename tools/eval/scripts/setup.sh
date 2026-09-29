#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Install the Ghira eval kit (macOS / Linux). Run from anywhere:
#   ./scripts/setup.sh              basic kit
#   ./scripts/setup.sh --with-nemo  also the NeMo models used for labelling drafts (large)
#
# uv (the Python tool that installs everything) is required. If it is missing we show the
# official installer command and ask y/N before running it. We never run it silently.
set -euo pipefail

with_nemo=0
for arg in "$@"; do
  case "$arg" in
    --with-nemo) with_nemo=1 ;;
    -h|--help) sed -n '3,7p' "$0"; exit 0 ;;
    *) echo "Unknown option: $arg" >&2; exit 2 ;;
  esac
done

cd "$(dirname "$0")/.."

if ! command -v uv >/dev/null 2>&1; then
  installer='curl -LsSf https://astral.sh/uv/install.sh | sh'
  echo "uv is not installed. The official installer is:"
  echo "  $installer"
  echo "(It is published by Astral, https://docs.astral.sh/uv/getting-started/installation/)"
  read -r -p "Run it now? [y/N] " answer
  case "$answer" in
    y|Y|yes|YES) sh -c "$installer" ;;
    *) echo "Not installed. Run the command above yourself, then run this script again."; exit 1 ;;
  esac
  export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
  command -v uv >/dev/null 2>&1 || { echo "uv still not found. Open a new terminal and retry." >&2; exit 1; }
fi

if [ "$with_nemo" -eq 1 ]; then
  uv sync --locked --extra nemo
else
  uv sync --locked
fi

uv run ghi-eval --help >/dev/null
echo "OK: the eval kit is installed. Next: follow docs/runbook.md, step 4 (dry run)."
