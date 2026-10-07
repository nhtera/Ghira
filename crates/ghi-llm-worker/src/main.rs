// SPDX-License-Identifier: Apache-2.0
//! Entry point of the LLM worker. Spawned by `ghi-llm` with `std::process`;
//! it speaks the JSON-lines protocol of the library on stdin and a private
//! copy of the original stdout, and runs llama.cpp in-process
//! ([`ghi_llm_worker::serve`]). Killing the process is how the model is unloaded.

use std::io::{BufReader, Write};
use std::sync::atomic::AtomicBool;

fn main() {
    if let Some(dir) = ghi_diag::dir_from_env() {
        ghi_diag::install_panic_hook("worker", dir, || "worker".into());
    }
    let out = take_protocol_stdout();
    let stdin = std::io::stdin();
    ghi_llm_worker::serve::serve(BufReader::new(stdin.lock()), out, &AtomicBool::new(false));
}

/// Keep the real stdout for protocol replies and point fd 1 at stderr, so a C
/// `printf` from llama.cpp or ggml can never corrupt the protocol.
#[cfg(unix)]
fn take_protocol_stdout() -> Box<dyn Write + Send> {
    use std::fs::File;
    use std::os::fd::FromRawFd;
    // SAFETY: plain fd juggling at process start, before any other thread runs;
    // the duplicated fd is owned by the returned File.
    unsafe {
        let fd = libc::dup(1);
        assert!(fd >= 0, "dup(stdout) failed");
        assert!(libc::dup2(2, 1) >= 0, "dup2(stderr, stdout) failed");
        Box::new(File::from_raw_fd(fd))
    }
}

/// Elsewhere there is no fd redirection: the protocol goes to plain stdout and
/// only the voided llama logs protect it from library output.
#[cfg(not(unix))]
fn take_protocol_stdout() -> Box<dyn Write + Send> {
    Box::new(std::io::stdout())
}
