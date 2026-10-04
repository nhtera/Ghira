#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# The full-flow XCUITest (Tests/Flows) from a fresh install, on the Simulator,
# with the fake microphone and scripted engines (no models needed).
#
#   apps/mobile/scripts/build-ios.sh --sim --test-hooks   # once
#   native/ios/GhiUITests/flows.sh
set -euo pipefail
[[ -z "${GHI_DEVICE_UDID:-}" ]] || { echo "flows.sh: simulator only (it resets the app)" >&2; exit 1; }
here="$(cd "$(dirname "$0")" && pwd)"
sim="$here/../../../apps/mobile/scripts/sim.sh"
wav="${TMPDIR:-/tmp}/ghira-flow-mic.wav"
python3 - "$wav" <<'PY'
import random, struct, sys, wave
random.seed(1)
w = wave.open(sys.argv[1], "wb")
w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000)
w.writeframes(b"".join(struct.pack("<h", random.randint(-800, 800)) for _ in range(16000 * 180)))
w.close()
PY
"$sim" boot >/dev/null
"$sim" reset >/dev/null
"$sim" install >/dev/null
"$sim" grant >/dev/null
# The inbox step drops files into the App Group container: it must exist.
group="$("$sim" container group)"
[[ -n "$group" && -d "$group" ]] || { echo "flows.sh: no App Group container for the app" >&2; exit 1; }
GHI_FAKE_MIC_PATH="$wav" "$here/run.sh" -only-testing:GhiUITests/FullFlowTests "$@"
