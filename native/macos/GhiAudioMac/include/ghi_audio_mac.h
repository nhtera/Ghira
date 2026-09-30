// SPDX-License-Identifier: Apache-2.0
//
// C ABI of GhiAudioMac, the macOS capture layer (Swift, exported with @_cdecl).
// Rust binds it by hand in crates/ghi-audio/src/macos.rs; keep the two in sync
// and bump GHI_MAC_ABI_VERSION on any change.
//
// Only scalars, pointers and callbacks cross the boundary: no structs, so there
// is no layout to drift. Requires macOS 14.2+ (Core Audio process taps).

#ifndef GHI_AUDIO_MAC_H
#define GHI_AUDIO_MAC_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define GHI_MAC_ABI_VERSION 1

// Tracks. Each is delivered separately, mono float32 at the device rate.
#define GHI_MAC_TRACK_MIC 0u
#define GHI_MAC_TRACK_SYSTEM 1u

// ghi_mac_start flags.
#define GHI_MAC_CAPTURE_MIC 0x1u
#define GHI_MAC_CAPTURE_SYSTEM 0x2u

// Status codes returned by the functions below. Positive values never occur;
// other negative values are OSStatus codes from Core Audio, passed through.
#define GHI_MAC_OK 0
#define GHI_MAC_ERR_INVALID -1           // bad argument or state
#define GHI_MAC_ERR_UNSUPPORTED -2       // macOS older than 14.2
#define GHI_MAC_ERR_MIC_PERMISSION -3    // microphone access denied or restricted
#define GHI_MAC_ERR_NO_DEVICE -4         // no default input/output device
#define GHI_MAC_ERR_TAP -5               // process tap or aggregate device failed
#define GHI_MAC_ERR_SYSTEM_PERMISSION -6 // System Audio Recording denied (only where Core Audio
                                         // reports it; otherwise a denied tap delivers zeros)

// Output route kinds (ghi_mac_route, GHI_MAC_EV_ROUTE_CHANGED).
#define GHI_MAC_ROUTE_UNKNOWN 0u
#define GHI_MAC_ROUTE_SPEAKERS 1u        // built-in speakers
#define GHI_MAC_ROUTE_HEADPHONES 2u      // wired headphones (built-in jack, USB headset)
#define GHI_MAC_ROUTE_BLUETOOTH 3u       // Bluetooth output (AirPods, headsets, BT speakers)
#define GHI_MAC_ROUTE_EXTERNAL 4u        // HDMI/DisplayPort/AirPlay/USB speakers

// Microphone permission (AVAuthorizationStatus).
#define GHI_MAC_PERM_UNDETERMINED 0
#define GHI_MAC_PERM_RESTRICTED 1
#define GHI_MAC_PERM_DENIED 2
#define GHI_MAC_PERM_AUTHORIZED 3

// Event kinds (ghi_mac_event_fn). `code` and `detail` per kind:
#define GHI_MAC_EV_TRACK_STARTED 1u      // code = track; detail = device name (sent after the device started; a few buffers may arrive first)
#define GHI_MAC_EV_ROUTE_CHANGED 2u      // code = route kind | 0x100 if the default input is Bluetooth (HFP)
#define GHI_MAC_EV_MIC_DEVICE_CHANGED 3u // mic restarted on the new default input; detail = device name
#define GHI_MAC_EV_SYSTEM_RESTARTED 4u   // system tap rebuilt after the output device changed
#define GHI_MAC_EV_TRACK_LOST 5u         // code = track; capture of that track stopped and could not restart
#define GHI_MAC_EV_SLEEP 6u              // the Mac is going to sleep; audio stops until WAKE
#define GHI_MAC_EV_WAKE 7u
#define GHI_MAC_EV_ERROR 8u              // code = OSStatus or GHI_MAC_ERR_*; detail = what failed

// Audio callback. Called on a Core Audio realtime thread: it must not block,
// allocate or lock. Calls for one track are serialized (never concurrent, also
// across a device rebuild); calls for different tracks may be concurrent.
// `samples` holds `frames` mono float32 samples, valid only during the call.
// `sample_rate` changes only after TRACK_STARTED, MIC_DEVICE_CHANGED or
// SYSTEM_RESTARTED. `host_time_ns` is the capture time of the first sample,
// mach absolute time in nanoseconds (same clock for both tracks; it does not
// advance during sleep, so size sleep gaps with the wall clock).
typedef void (*ghi_mac_audio_fn)(void *ctx, uint32_t track, const float *samples,
                                 uint32_t frames, double sample_rate,
                                 uint64_t host_time_ns);

// Event callback. Called from a non-realtime queue. `detail` is UTF-8, may be
// NULL, and is valid only during the call.
typedef void (*ghi_mac_event_fn)(void *ctx, uint32_t kind, int64_t code,
                                 const char *detail);

uint32_t ghi_mac_abi_version(void);

// Starts capture. `flags` is a GHI_MAC_CAPTURE_* mask.
// System track: taps all processes except this one when `n_tap_pids` is 0,
// otherwise only the given processes (per-app capture); a PID with no Core
// Audio process object is skipped with a GHI_MAC_EV_ERROR event. The HAL mixes
// the tap to mono. When both tracks are captured they share one private
// aggregate device (one clock, drift-compensated). If only the tap fails, the
// session still starts with the mic alone (GHI_MAC_EV_ERROR + TRACK_LOST for
// the system track) and keeps retrying the tap. While capturing (not paused)
// the session holds a PreventUserIdleSystemSleep power assertion ("Ghira is
// recording"): idle sleep would silence the devices. May block while a
// permission prompt is open, so never call it on an app's main thread.
// On success stores an opaque session in `*out_session`.
//
// Never call ghi_mac_start/pause/resume/stop from inside a callback.
int32_t ghi_mac_start(uint32_t flags, const int32_t *tap_pids,
                      uint32_t n_tap_pids, ghi_mac_audio_fn audio,
                      ghi_mac_event_fn event, void *ctx, void **out_session);

// Stops the devices (the mic indicator turns off) and restarts them.
int32_t ghi_mac_pause(void *session);
int32_t ghi_mac_resume(void *session);

// Stops capture and frees the session. When it returns, no audio or event
// callback is running or will run again, so `ctx` may be freed.
void ghi_mac_stop(void *session);

int32_t ghi_mac_mic_permission(void);
// Shows the system prompt if undetermined; blocks until answered. Must not be
// called on the main thread. Returns the resulting GHI_MAC_PERM_* value.
int32_t ghi_mac_request_mic_permission(void);

// Current output route and whether the default input is a Bluetooth device.
int32_t ghi_mac_route(uint32_t *output_kind, uint32_t *input_bluetooth);

// Processes known to Core Audio, as a JSON array:
// [{"pid":123,"bundle_id":"us.zoom.xos","input":true,"output":false}, ...]
// `bundle_id` may be "". Free the result with ghi_mac_free. NULL on failure.
char *ghi_mac_audio_processes(void);
void ghi_mac_free(char *s);

#ifdef __cplusplus
}
#endif

#endif // GHI_AUDIO_MAC_H
