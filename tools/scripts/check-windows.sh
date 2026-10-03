#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Type-checks and lints the workspace for Windows from a Mac (or Linux), so
# `cfg(windows)` code doesn't rot between real Windows CI runs.
#
# It uses the GNU target with mingw-w64 (`brew install mingw-w64`), not MSVC:
# it's a compile signal for first-party code only. CRT, linking and runtime
# behaviour still need `windows-latest` CI or a Windows machine (phase 13).
#
# Usage: tools/scripts/check-windows.sh            # clippy -D warnings
#        tools/scripts/check-windows.sh --check    # cargo check only
set -euo pipefail

cd "$(dirname "$0")/../.."
TARGET=x86_64-pc-windows-gnu

command -v x86_64-w64-mingw32-gcc >/dev/null ||
  { echo "check-windows: needs mingw-w64 (brew install mingw-w64)" >&2; exit 2; }
rustup target list --installed | grep -qx "$TARGET" || rustup target add "$TARGET"

MINGW=$(brew --prefix mingw-w64 2>/dev/null || echo /usr)/toolchain-x86_64/x86_64-w64-mingw32
export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc
export CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++
export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
export BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_gnu="--target=x86_64-w64-mingw32 --sysroot=$MINGW -I$MINGW/include"
# A separate target dir: cross builds never touch the host build cache.
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-target/windows-check}

# ghi-mobile is iOS only.
if [ "${1:-}" = "--check" ]; then
  cargo check --locked --workspace --exclude ghi-mobile --target "$TARGET"
else
  cargo clippy --keep-going --locked --workspace --exclude ghi-mobile --all-targets --target "$TARGET" -- -D warnings
fi
echo "check-windows: ok"
