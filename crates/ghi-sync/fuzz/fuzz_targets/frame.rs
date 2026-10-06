// SPDX-License-Identifier: Apache-2.0
//! Frames and handshake of the Noise transport: arbitrary bytes where a peer's
//! bytes arrive. Never a panic, never a message above the 4 MiB cap, and the
//! reader ends (closed or error) instead of waiting.
#![no_main]

use std::io::Write;

use ghi_sync::transport::Transport;
use ghi_sync_fuzz::{established_responder, respond};
use libfuzzer_sys::fuzz_target;

const MAX_MESSAGE: usize = 4 * 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    let Some((mode, bytes)) = data.split_first() else {
        return;
    };
    if mode % 2 == 0 {
        // Raw bytes as a handshake: message 1 parsing, the 256-byte cap on
        // handshake frames, Noise's own checks.
        let (mut peer, ours) = ghi_sync::mem::mem_pipe();
        let _ = peer.write_all(bytes);
        drop(peer);
        let _ = respond(ours);
    } else {
        // Raw bytes as transport frames after a good handshake: the length
        // prefix, short and torn frames, the authentication failure path.
        let (mut transport, mut inject) = established_responder();
        let _ = inject.write_all(bytes);
        drop(inject);
        while let Ok(msg) = transport.recv() {
            assert!(msg.len() <= MAX_MESSAGE, "a message above the cap");
        }
    }
});
