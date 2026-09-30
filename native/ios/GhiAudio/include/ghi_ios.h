// SPDX-License-Identifier: Apache-2.0
// The Rust side of the iOS C ABI (apps/mobile/src-tauri/src/platform.rs),
// called from Swift. The Swift side (`ghi_swift_*`) is exported with @_cdecl.
#ifndef GHI_IOS_H
#define GHI_IOS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// One block of mono PCM from the audio tap at `rate` Hz; `host_ns` is the
// block's host time in nanoseconds.
void ghi_ios_push_pcm(const float *samples, size_t len, double rate, uint64_t host_ns);
// The app will resign active: no new engine steps (no new GPU work).
void ghi_ios_suspend(void);
// The app entered the background (main thread; does not block). False if an
// engine step overlapped the transition: its GPU work may have been refused,
// so the engine drops its results and redoes that audio.
bool ghi_ios_entered_background(void);
// The app is active again.
void ghi_ios_resume(void);
// ProcessInfo.thermalState raw value (0 nominal ... 3 critical).
void ghi_ios_thermal_changed(int32_t state);
// An audio interruption began (true) or ended (false).
void ghi_ios_interruption(bool began);
// Live Activity intents.
void ghi_ios_stop_requested(void);
void ghi_ios_mark_requested(void);

#endif
