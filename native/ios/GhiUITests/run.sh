#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs the XCUITest bundle against the installed app on the Simulator (never a
# device: the UDID must be a simulator, see apps/mobile/scripts/sim.sh).
#
#   native/ios/GhiUITests/run.sh [xcodebuild test args, e.g. -only-testing:GhiUITests/SmokeTests/testLaunchShowsTheApp]
# Env: GHI_SIM_UDID, GHI_BG_SECONDS (home + lock dwell of SmokeTests/testRecordingSurvivesHomeAndLock,
# default 120), GHI_REC_SECONDS (recording length of the transcript probes, default 100), GHI_FAKE_MIC_PATH (a WAV the simulator can read), GHI_REAL_ENGINES=1 (the models in
# the app instead of the scripted engines). FullFlowTests needs flows.sh (fresh install) and is
# left out unless -only-testing names it (so are the two transcript probes of SmokeTests, which take minutes).
#
# A real iPhone instead of the Simulator: GHI_DEVICE_UDID=<hardware udid> (xcodebuild's id, not
# devicectl's identifier; `xcrun devicectl list devices --json-output f` -> hardwareProperties.udid).
# The runner is signed with $APPLE_DEVELOPMENT_TEAM (or the first paid Xcode team) and
# -allowProvisioningUpdates. The app (test-hooks device build) must already be installed; nothing is
# reset or uninstalled. Simulator-only tests skip themselves. GHI_DEVICE_CTL_ID=<devicectl id> copies
# GHI_FAKE_MIC_PATH (a host WAV) into the app's Documents/fake-mic.wav before the run.
# GHI_XCODE_ACTION=build-for-testing prepares the runner without touching the phone.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
sim="$here/../../../apps/mobile/scripts/sim.sh"
action="${GHI_XCODE_ACTION:-test}"
if [[ -n "${GHI_DEVICE_UDID:-}" ]]; then
  destination="platform=iOS,id=$GHI_DEVICE_UDID"
  team="${APPLE_DEVELOPMENT_TEAM:-$(defaults read com.apple.dt.Xcode IDEProvisioningTeamByIdentifier 2>/dev/null |
    awk '/isFreeProvisioningTeam = 0/ {paid = 1} /teamID =/ {if (paid) {gsub(/[;" ]/, "", $3); print $3; exit}}')}"
  [[ -n "$team" ]] || { echo "run.sh: set APPLE_DEVELOPMENT_TEAM" >&2; exit 1; }
  sign=(-allowProvisioningUpdates DEVELOPMENT_TEAM="$team" CODE_SIGNING_ALLOWED=YES CODE_SIGN_STYLE=Automatic)
  export TEST_RUNNER_GHI_ON_DEVICE=1
  mic="${GHI_FAKE_MIC_PATH:-}"
  if [[ "$action" != build-for-testing && -n "$mic" && -n "${GHI_DEVICE_CTL_ID:-}" ]]; then
    xcrun devicectl device copy to --device "$GHI_DEVICE_CTL_ID" --domain-type appDataContainer \
      --domain-identifier com.nhtera.ghira --source "$mic" --destination Documents/fake-mic.wav >/dev/null
    mic="Documents/fake-mic.wav" # the test hooks resolve a relative path against the app's home
  fi
  export TEST_RUNNER_GHI_FAKE_MIC_PATH="$mic"
else
  udid="$("$sim" udid)"
  destination="platform=iOS Simulator,id=$udid"
  sign=()
  "$sim" boot >/dev/null
  # Face ID enrolled (the app-lock tests post the match / no-match notifications).
  xcrun simctl spawn "$udid" notifyutil -s com.apple.BiometricKit.enrollmentChanged 1 >/dev/null 2>&1 || true
  xcrun simctl spawn "$udid" notifyutil -p com.apple.BiometricKit.enrollmentChanged >/dev/null 2>&1 || true
  export TEST_RUNNER_GHI_FAKE_MIC_PATH="${GHI_FAKE_MIC_PATH:-}"
  # The App Group container (share-extension inbox tests drop files there).
  export TEST_RUNNER_GHI_GROUP_DIR="$("$sim" container group 2>/dev/null || true)"
fi
cd "$here"
xcodegen generate --quiet
export TEST_RUNNER_GHI_BG_SECONDS="${GHI_BG_SECONDS:-120}"
export TEST_RUNNER_GHI_REC_SECONDS="${GHI_REC_SECONDS:-}" # SmokeTests transcript probes (default 100)
export TEST_RUNNER_GHI_REAL_ENGINES="${GHI_REAL_ENGINES:-}" # only a non-empty value turns it on
skip=(-skip-testing:GhiUITests/FullFlowTests -skip-testing:GhiUITests/SmokeTests/testForegroundTranscriptKeepsMoving -skip-testing:GhiUITests/SmokeTests/testTranscriptGoesOnAfterHome)
for a in "$@"; do [[ "$a" == -only-testing:* ]] && skip=(); done
xcodebuild "$action" -project GhiUITests.xcodeproj -scheme GhiUITests \
  -destination "$destination" -derivedDataPath build ${sign[@]+"${sign[@]}"} ${skip[@]+"${skip[@]}"} "$@"
