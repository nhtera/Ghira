#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Phase 16: a release iOS artifact must not contain the Simulator test hooks
# (apps/mobile `test-hooks` feature + Swift GHI_TEST_HOOKS): the fake microphone,
# the scripted engines and the Darwin-notification triggers.
#
# Works on macOS's /bin/bash 3.2.
# Usage: check-no-test-hooks.sh [--expect-hooks] <artifact>...
#   artifact: a static library (libghi_mobile_lib.a, libapp.a), a binary, or an
#             .app bundle (every file inside is scanned).
#   --expect-hooks: inverted, for a build made WITH the hooks; fails when a
#             marker is missing, which proves the check still detects them (CI).
set -euo pipefail

markers=("GHI_FAKE_MIC" "GHI_FAKE_ENGINES" "GHI_FAKE_CALENDAR" "GHI_FAKE_MIC_TAP" "GHI_IGNORE_THERMAL" "com.nhtera.ghira.test.")
expect=0
if [[ "${1:-}" == --expect-hooks ]]; then expect=1; shift; fi
[[ $# -gt 0 ]] || { echo "usage: $0 [--expect-hooks] <artifact>..." >&2; exit 2; }

status=0
for artifact in "$@"; do
  if [[ ! -e "$artifact" ]]; then
    echo "error: $artifact not found (build it first)" >&2
    status=1
    continue
  fi
  files=()
  binaries=0
  while IFS= read -r -d '' f; do
    files+=("$f")
    # A Mach-O binary or an ar archive (a static library): what the markers live in.
    case "$(file -b "$f")" in *Mach-O* | *"ar archive"*) binaries=$((binaries + 1)) ;; esac
  done < <(find "$artifact" -type f -print0)
  if [[ $binaries -eq 0 ]]; then
    echo "error: $artifact holds no Mach-O binary or static library: nothing to scan" >&2
    status=1
    continue
  fi
  for marker in "${markers[@]}"; do
    found=0
    for f in "${files[@]}"; do
      if LC_ALL=C grep -aqF -- "$marker" "$f"; then found=1; break; fi
    done
    if [[ $expect == 0 && $found == 1 ]]; then
      echo "error: $artifact contains the test hook marker '$marker'" >&2
      status=1
    elif [[ $expect == 1 && $found == 0 ]]; then
      echo "error: $artifact has no '$marker' but was built with the hooks: the check is blind" >&2
      status=1
    fi
  done
done
if [[ $status -eq 0 ]]; then
  [[ $expect == 1 ]] && echo "test hooks detected as expected: ok ($#)" || echo "no test hooks: ok ($#)"
fi
exit $status
