// SPDX-License-Identifier: Apache-2.0
//! Notification permission on macOS (`UNUserNotificationCenter`): Tauri's
//! notification plugin never asks on desktop, so the "Allow…" button of the
//! onboarding permissions asks here. Only a bundled app can (an unbundled dev
//! run has no notification center and reports `None`).

use std::time::Duration;

use block2::RcBlock;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, Bool};
use objc2::{class, msg_send};

#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

/// Badge | sound | alert.
const OPTIONS: usize = 1 | 2 | 4;

/// Shows the system prompt when undetermined and waits for the answer:
/// `Some(true)` allowed, `Some(false)` refused, `None` not asked (unbundled).
/// Blocks; call it off the main thread.
pub fn request() -> Option<bool> {
    let (tx, rx) = std::sync::mpsc::channel();
    autoreleasepool(|_| {
        // SAFETY: plain message sends to Foundation / UserNotifications
        // classes with the documented argument types.
        unsafe {
            let bundle: Option<Retained<AnyObject>> = msg_send![class!(NSBundle), mainBundle];
            let path: Option<Retained<objc2_foundation::NSString>> =
                bundle.and_then(|b| msg_send![&*b, bundlePath]);
            if !path.is_some_and(|p| p.to_string().ends_with(".app")) {
                return None;
            }
            let center: Option<Retained<AnyObject>> =
                msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
            let center = center?;
            let block = RcBlock::new(move |granted: Bool, _error: *mut AnyObject| {
                let _ = tx.send(granted.as_bool());
            });
            let _: () = msg_send![
                &*center,
                requestAuthorizationWithOptions: OPTIONS,
                completionHandler: &*block
            ];
            Some(())
        }
    })?;
    rx.recv_timeout(Duration::from_secs(300)).ok()
}
