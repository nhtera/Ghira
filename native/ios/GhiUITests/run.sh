#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs the XCUITest bundle against the installed app on the Simulator (never a
# device: the UDID must be a simulator, see apps/mobile/scripts/sim.sh).
#
#   native/ios/GhiUITests/run.sh [xcodebuild test args, e.g. -only-testing:GhiUITests/SmokeTests/testLaunchShowsTheApp]
# Env: GHI_SIM_UDID, GHI_BG_SECONDS (default 120), GHI_FAKE_MIC_PATH (a WAV the simulator can read).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
sim="$here/../../../apps/mobile/scripts/sim.sh"
udid="$("$sim" udid)"
"$sim" boot >/dev/null
cd "$here"
xcodegen generate --quiet
export TEST_RUNNER_GHI_BG_SECONDS="${GHI_BG_SECONDS:-120}"
export TEST_RUNNER_GHI_FAKE_MIC_PATH="${GHI_FAKE_MIC_PATH:-}"
xcodebuild test -project GhiUITests.xcodeproj -scheme GhiUITests \
  -destination "platform=iOS Simulator,id=$udid" -derivedDataPath build "$@"
