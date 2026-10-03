#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs the XCUITest bundle against the installed app on the Simulator (never a
# device: the UDID must be a simulator, see apps/mobile/scripts/sim.sh).
#
#   native/ios/GhiUITests/run.sh [xcodebuild test args, e.g. -only-testing:GhiUITests/SmokeTests/testLaunchShowsTheApp]
# Env: GHI_SIM_UDID, GHI_BG_SECONDS (home + lock dwell of SmokeTests/testRecordingSurvivesHomeAndLock,
# default 120), GHI_FAKE_MIC_PATH (a WAV the simulator can read), GHI_REAL_ENGINES=1 (the models in
# the app instead of the scripted engines). FullFlowTests needs flows.sh (fresh install) and is
# left out unless -only-testing names it.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
sim="$here/../../../apps/mobile/scripts/sim.sh"
udid="$("$sim" udid)"
"$sim" boot >/dev/null
# Face ID enrolled (the app-lock tests post the match / no-match notifications).
xcrun simctl spawn "$udid" notifyutil -s com.apple.BiometricKit.enrollmentChanged 1 >/dev/null 2>&1 || true
xcrun simctl spawn "$udid" notifyutil -p com.apple.BiometricKit.enrollmentChanged >/dev/null 2>&1 || true
cd "$here"
xcodegen generate --quiet
export TEST_RUNNER_GHI_BG_SECONDS="${GHI_BG_SECONDS:-120}"
export TEST_RUNNER_GHI_FAKE_MIC_PATH="${GHI_FAKE_MIC_PATH:-}"
export TEST_RUNNER_GHI_REAL_ENGINES="${GHI_REAL_ENGINES:-}" # only a non-empty value turns it on
# The App Group container (share-extension inbox tests drop files there).
export TEST_RUNNER_GHI_GROUP_DIR="$("$sim" container group 2>/dev/null || true)"
skip=(-skip-testing:GhiUITests/FullFlowTests)
for a in "$@"; do [[ "$a" == -only-testing:* ]] && skip=(); done
xcodebuild test -project GhiUITests.xcodeproj -scheme GhiUITests \
  -destination "platform=iOS Simulator,id=$udid" -derivedDataPath build ${skip[@]+"${skip[@]}"} "$@"
