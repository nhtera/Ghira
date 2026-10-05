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
// A phone call started or ended (CXCallObserver); an ended call that
// interrupted a recording asks to resume, it never resumes by itself.
void ghi_ios_call_active_changed(bool active);
// Live Activity intents.
void ghi_ios_stop_requested(void);
void ghi_ios_mark_requested(void);
// Dynamic Type changed: the root font-size multiplier (1.0 = Large, capped at 2.0).
void ghi_ios_text_scale_changed(float scale);
// The audio route changed (AVAudioSession.RouteChangeReason raw value).
void ghi_ios_route_changed(int32_t reason);
// The system sent a memory warning.
void ghi_ios_memory_warning(void);
// The share extension added files to the App Group inbox.
void ghi_ios_inbox_changed(void);

// --- Swift side (implemented in native/ios/GhiAudio/GhiPlatform.swift with
// @_cdecl; declared here as the full contract, Rust declares the same in
// apps/mobile/src-tauri/src/platform.rs) -------------------------------------

// A phone call is active (CXCallObserver).
bool ghi_swift_call_active(void);
// The app process was launched while in the background (a relaunch, a Live
// Activity intent): the job runner must start paused.
bool ghi_swift_launched_in_background(void);
// The network path is expensive (cellular, Personal Hotspot) or constrained
// (Low Data Mode): NWPathMonitor.
bool ghi_swift_on_expensive_network(void);
// Writes the machine identifier ("iPhone16,1"; the simulated model on the
// simulator) NUL-terminated into buf (capacity cap); returns its length.
size_t ghi_swift_device_model(char *buf, size_t cap);
// ProcessInfo.physicalMemory, bytes.
uint64_t ghi_swift_physical_memory(void);
// Free space for important data on the app volume, bytes; negative if unknown.
int64_t ghi_swift_available_capacity(void);
// Dynamic Type multiplier (1.0 = Large), capped at 2.0.
float ghi_swift_text_scale(void);
// Presents the share sheet for the file at path; false if it could not be shown.
// The file is deleted when the sheet closes.
bool ghi_swift_share_file(const char *path);
// Opens this app's page in the Settings app.
void ghi_swift_open_settings(void);
// The main WKWebView (Tauri's platform webview pointer) fills the screen: its scroll
// view stops adding the safe-area insets to the page's layout viewport. Main thread.
void ghi_swift_webview_never_adjust(void *webview);
// beginBackgroundTask; returns a token for ghi_swift_end_bg_task (0: none granted).
uint64_t ghi_swift_begin_bg_task(const char *name);
void ghi_swift_end_bg_task(uint64_t token);
// Covers the window (app-switcher snapshot, app lock).
void ghi_swift_set_privacy_cover(bool on);
// Nanoseconds from a clock that keeps counting while the device sleeps.
uint64_t ghi_swift_continuous_ns(void);
// Live Activity. `phase` is the raw value of `ActivityPhase`
// (native/ios/Shared/ActivityPhase.swift; mirrored in platform.rs).
void ghi_swift_activity_start(int32_t phase);
void ghi_swift_activity_update(int32_t phase, uint32_t marks);
void ghi_swift_activity_end(void);
// Microphone permission: 0 not determined, 1 granted, 2 denied.
int32_t ghi_swift_mic_permission(void);
// Shows the system prompt (first time only); poll ghi_swift_mic_permission.
void ghi_swift_request_mic_permission(void);

// Calendar (EventKit): 0 not determined, 1 full access, 2 denied (or
// restricted, or write-only). Never prompts.
int32_t ghi_swift_calendar_access(void);
// Shows the system prompt (first time only); poll ghi_swift_calendar_access.
void ghi_swift_request_calendar_access(void);
// The events starting in [from_ms, to_ms) (unix ms) as a JSON array of the raw
// events (ghi-core calendar::RawEvent), in a heap string freed with
// ghi_swift_string_free; NULL without full access.
char *ghi_swift_calendar_events(int64_t from_ms, int64_t to_ms);
void ghi_swift_string_free(char *s);

// LAN sync (phase 15, doc 07). The camera scan of the desktop's pairing code:
// the scanned text goes to Rust only (ghi_ios_qr_scanned), never to the webview.
void ghi_swift_qr_scan_start(void);
void ghi_swift_qr_scan_stop(void);
// Swift -> Rust: the text of a scanned QR code (UTF-8, NUL-terminated; Rust
// copies it, the caller keeps ownership). It carries a secret: never log it.
void ghi_ios_qr_scanned(const char *text);
// Bonjour browse for the desktop's sync service (NWBrowser; the local
// network permission prompt appears on first use).
void ghi_swift_browse_start(void);
void ghi_swift_browse_stop(void);
// Swift -> Rust: the services currently visible, as a JSON array of
// {"name": string, "host": string, "port": number} (copied by Rust).
void ghi_ios_browse_found(const char *json);

#endif
