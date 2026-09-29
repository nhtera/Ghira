// SPDX-License-Identifier: Apache-2.0
// Prevents an extra console window on Windows in release builds. Do not remove.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ghi_desktop_lib::run()
}
