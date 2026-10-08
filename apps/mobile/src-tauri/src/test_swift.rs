// SPDX-License-Identifier: Apache-2.0
//! The Swift half of the C ABI (`native/ios/GhiAudio`) for the unit-test
//! binary on the Simulator: `test-ios-sim.sh` runs `cargo test` for
//! `aarch64-apple-ios-sim` with `simctl spawn`, where no Swift is linked. The
//! stubs answer like a quiet, permitted, capable phone.

use std::ffi::c_char;

#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_init() {}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_audio_start() -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_audio_stop() {}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_activity_start(_phase: i32) {}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_activity_update(_phase: i32, _marks: u32) {}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_activity_end() {}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_thermal_state() -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_memory_footprint() -> u64 {
    200 << 20
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_battery_level() -> f32 {
    -1.0
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_keep_awake(_on: bool) {}
/// # Safety
/// Never dereferences `path`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghi_swift_exclude_from_backup(_path: *const c_char) -> bool {
    true
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_call_active() -> bool {
    false
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_launched_in_background() -> bool {
    false
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_on_expensive_network() -> bool {
    false
}
/// # Safety
/// `buf` is writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghi_swift_device_model(buf: *mut c_char, cap: usize) -> usize {
    let id = b"iPhone17,1\0";
    let n = id.len().min(cap);
    // SAFETY: `buf` holds `cap` bytes and `n <= cap`.
    unsafe { std::ptr::copy_nonoverlapping(id.as_ptr().cast::<c_char>(), buf, n) };
    n.saturating_sub(1)
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_physical_memory() -> u64 {
    8 << 30
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_text_scale() -> f32 {
    1.0
}
/// # Safety
/// Never dereferences `path`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghi_swift_share_file(_path: *const c_char) -> bool {
    false
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_open_settings() {}
/// # Safety
/// Never dereferences `webview`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghi_swift_webview_never_adjust(_webview: *mut std::ffi::c_void) {}
/// # Safety
/// Never dereferences `name`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghi_swift_begin_bg_task(_name: *const c_char) -> u64 {
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_end_bg_task(_token: u64) {}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_set_privacy_cover(_on: bool) {}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_continuous_ns() -> u64 {
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_mic_permission() -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_request_mic_permission() {}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_available_capacity() -> i64 {
    -1
}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_qr_scan_start() {}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_qr_scan_stop() {}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_browse_start() {}
#[unsafe(no_mangle)]
pub extern "C" fn ghi_swift_browse_stop() {}

/// References every stub so the linker keeps them (the platform layer's own
/// references are to external symbols it resolves at link time).
pub fn keep() -> usize {
    [
        ghi_swift_init as *const () as usize,
        ghi_swift_audio_start as *const () as usize,
        ghi_swift_audio_stop as *const () as usize,
        ghi_swift_activity_start as *const () as usize,
        ghi_swift_activity_update as *const () as usize,
        ghi_swift_activity_end as *const () as usize,
        ghi_swift_thermal_state as *const () as usize,
        ghi_swift_memory_footprint as *const () as usize,
        ghi_swift_battery_level as *const () as usize,
        ghi_swift_keep_awake as *const () as usize,
        ghi_swift_exclude_from_backup as *const () as usize,
        ghi_swift_call_active as *const () as usize,
        ghi_swift_launched_in_background as *const () as usize,
        ghi_swift_on_expensive_network as *const () as usize,
        ghi_swift_device_model as *const () as usize,
        ghi_swift_physical_memory as *const () as usize,
        ghi_swift_text_scale as *const () as usize,
        ghi_swift_share_file as *const () as usize,
        ghi_swift_open_settings as *const () as usize,
        ghi_swift_webview_never_adjust as *const () as usize,
        ghi_swift_begin_bg_task as *const () as usize,
        ghi_swift_end_bg_task as *const () as usize,
        ghi_swift_set_privacy_cover as *const () as usize,
        ghi_swift_continuous_ns as *const () as usize,
        ghi_swift_mic_permission as *const () as usize,
        ghi_swift_request_mic_permission as *const () as usize,
        ghi_swift_available_capacity as *const () as usize,
        ghi_swift_qr_scan_start as *const () as usize,
        ghi_swift_qr_scan_stop as *const () as usize,
        ghi_swift_browse_start as *const () as usize,
        ghi_swift_browse_stop as *const () as usize,
    ]
    .iter()
    .sum()
}
