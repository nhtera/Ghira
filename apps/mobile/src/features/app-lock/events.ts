// SPDX-License-Identifier: Apache-2.0
// Window events of the app lock. Anything that keeps UI above the routes (a
// sheet host) closes itself on LOCKED_EVENT; screens inside the routes are
// unmounted while locked and mount fresh after UNLOCKED_EVENT.
export const LOCKED_EVENT = "ghi-locked";
export const UNLOCKED_EVENT = "ghi-unlocked";
