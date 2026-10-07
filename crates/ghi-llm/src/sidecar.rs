// SPDX-License-Identifier: Apache-2.0
//! The ghi-llm-worker process: spawn, stdio JSON lines, timeouts, kill = unload.
//! With feature `inproc` (iOS: an app may not start a process) the same engine
//! runs on a thread over in-memory pipes ([`Sidecar::in_process`]); a kill sets
//! its stop flag, and it frees the model after the token it is on.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub use ghi_llm_worker::{Body, Hello, Op, PROTOCOL, Reply, Request, WireMessage};

use crate::{LlmError, Result};

/// How long `Drop` waits for the worker to exit after `shutdown`.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
/// How long a fresh worker has to say hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest stdout line accepted (a completion is one line).
const MAX_LINE: usize = 8 * 1024 * 1024;
/// Non-protocol stdout lines tolerated per request before giving up.
const MAX_NOISE: usize = 20;
/// Worker stderr lines kept for crash reports.
const STDERR_LINES: usize = 50;
/// Stderr is read in pieces of at most this many bytes per line.
const STDERR_LINE_MAX: usize = 4096;
/// Stderr lines shown in an error message, and their width.
const TAIL_SHOWN: usize = 12;
const TAIL_WIDTH: usize = 300;

/// Pids of the workers alive in this process, so the app can kill them on
/// exit even while one is busy generating ([`kill_all`]).
static LIVE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

fn live() -> std::sync::MutexGuard<'static, Vec<u32>> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Kills every live worker process at once (app shutdown). The sidecars stay
/// valid objects; their next request fails like after a crash.
pub fn kill_all() {
    for pid in live().drain(..) {
        kill_pid(pid);
    }
}

fn kill_pid(pid: u32) {
    #[cfg(unix)]
    // SAFETY: a plain signal to a child of ours that has not been reaped
    // (reaped ones are removed from `LIVE` first).
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = pid;
}

type Tail = Arc<Mutex<VecDeque<String>>>;

enum Line {
    Text(String),
    TooLong,
    Err(std::io::Error),
}

/// What runs the engine.
enum Proc {
    Child(Child),
    /// The engine on a thread of this process; `stop` ends it.
    #[cfg_attr(not(feature = "inproc"), allow(dead_code))]
    Thread {
        stop: Arc<std::sync::atomic::AtomicBool>,
        /// Handed to [`ENGINE`] when this sidecar goes, so the next engine
        /// waits for it.
        engine: Option<JoinHandle<()>>,
    },
}

/// One running worker. Requests are strictly one at a time.
pub struct Sidecar {
    proc: Proc,
    stdin: Option<Box<dyn Write + Send>>,
    lines: Receiver<Line>,
    tail: Tail,
    threads: Vec<JoinHandle<()>>,
    next_id: u64,
    hello: Hello,
}

impl Sidecar {
    /// Start the worker binary and check its hello.
    pub fn spawn(worker: &Path) -> Result<Sidecar> {
        let mut cmd = Command::new(worker);
        // The worker needs nothing from the environment but the debug switch.
        cmd.env_clear();
        if debug() {
            cmd.env("GHI_LLM_DEBUG", "1");
        }
        // Where the worker writes its own crash reports.
        if let Some(dir) = ghi_diag::dir() {
            cmd.env(ghi_diag::DIR_ENV, dir);
        }
        log::info!("llm worker starting");
        Sidecar::spawn_command(cmd, HELLO_TIMEOUT)
    }

    /// Start `cmd` as a worker (stdio is set here) and read its hello line.
    fn spawn_command(mut cmd: Command, hello_timeout: Duration) -> Result<Sidecar> {
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| {
            LlmError::Worker(format!("cannot start {}: {e}", cmd.get_program().display()))
        })?;
        live().push(child.id());
        let stdin = child
            .stdin
            .take()
            .map(|s| Box::new(s) as Box<dyn Write + Send>);
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");

        let (tx, lines) = mpsc::channel();
        let reader = thread::spawn(move || read_lines(stdout, &tx));
        let tail: Tail = Arc::default();
        let drain = {
            let tail = Arc::clone(&tail);
            thread::spawn(move || drain_stderr(stderr, &tail))
        };
        let mut s = Sidecar {
            proc: Proc::Child(child),
            stdin,
            lines,
            tail,
            threads: vec![reader, drain],
            next_id: 1,
            hello: Hello::current(),
        };
        s.read_hello(hello_timeout)?;
        Ok(s)
    }

    /// Run the engine on a thread of this process (no worker binary), and read
    /// its hello.
    #[cfg(feature = "inproc")]
    pub fn in_process() -> Result<Sidecar> {
        use std::sync::atomic::AtomicBool;
        // One model at a time in this process: a stopped engine finishes its
        // current step and frees its model before the next one loads.
        wait_for_previous_engine()?;
        // (read end, write end): we write requests, the engine writes replies.
        let (engine_in, requests) = std::io::pipe()?;
        let (replies, engine_out) = std::io::pipe()?;
        // A write after we stopped reading must fail with EPIPE, not raise
        // SIGPIPE: the app's entry point (iOS) does not ignore that signal,
        // and it would end the whole app.
        no_sigpipe(&engine_out);
        no_sigpipe(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let engine = {
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("ghi-llm".into())
                .spawn(move || {
                    ghi_llm_worker::serve::serve(
                        BufReader::new(engine_in),
                        Box::new(engine_out),
                        &stop,
                    )
                })
                .map_err(|e| LlmError::Worker(format!("cannot start the engine thread: {e}")))?
        };
        let (tx, lines) = mpsc::channel();
        let reader = thread::spawn(move || read_lines(replies, &tx));
        log::info!("llm engine starting in process");
        let mut s = Sidecar {
            proc: Proc::Thread {
                stop,
                engine: Some(engine),
            },
            stdin: Some(Box::new(requests)),
            lines,
            tail: Arc::default(),
            threads: vec![reader],
            next_id: 1,
            hello: Hello::current(),
        };
        s.read_hello(HELLO_TIMEOUT)?;
        Ok(s)
    }

    fn read_hello(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let raw = self.next_line(deadline)?;
        match serde_json::from_str::<Hello>(&raw) {
            Ok(h) if h.kind == "hello" && h.protocol == PROTOCOL => {
                self.hello = h;
                Ok(())
            }
            Ok(h) if h.kind == "hello" => {
                self.kill();
                Err(LlmError::Worker(format!(
                    "worker speaks protocol {} (worker {}), expected {PROTOCOL}",
                    h.protocol, h.worker
                )))
            }
            _ => {
                self.kill();
                Err(LlmError::Worker("worker did not start with a hello".into()))
            }
        }
    }

    /// The worker's version, from its hello.
    pub fn worker_version(&self) -> &str {
        &self.hello.worker
    }

    /// Send one request and wait for its reply. On timeout the worker is
    /// killed and reaped; after any error the sidecar must be discarded.
    pub fn request(&mut self, op: Op, timeout: Duration) -> Result<Reply> {
        self.request_live(op, timeout, timeout, &mut |_, _| {})
    }

    /// Like [`Sidecar::request`] for long requests that report progress: the
    /// worker is killed only after `idle` without a line (no progress means
    /// it hung, whatever the machine's speed), or once `hard` has passed.
    /// `on_progress` gets (prompt tokens done, output tokens).
    pub fn request_live(
        &mut self,
        op: Op,
        idle: Duration,
        hard: Duration,
        on_progress: &mut dyn FnMut(u32, u32),
    ) -> Result<Reply> {
        let id = self.next_id;
        self.next_id += 1;
        let line = serde_json::to_string(&Request { id, op })
            .map_err(|e| LlmError::Worker(format!("cannot encode request: {e}")))?;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| LlmError::Worker("worker is not running".into()))?;
        if let Err(e) = writeln!(stdin, "{line}").and_then(|_| stdin.flush()) {
            return Err(self.died(&format!("cannot write to worker: {e}")));
        }

        let started = Instant::now();
        let hard_deadline = started + hard;
        let mut deadline = (started + idle).min(hard_deadline);
        let mut noise = 0;
        loop {
            let raw = self.next_line(deadline)?;
            let Ok(reply) = serde_json::from_str::<Reply>(&raw) else {
                // Stray output that is not a reply (a library printing to fd 1).
                noise += 1;
                if noise > MAX_NOISE {
                    return Err(self.died("worker keeps writing non-protocol output"));
                }
                continue;
            };
            if reply.id != id {
                return Err(self.died(&format!(
                    "reply id {} does not match request {id}",
                    reply.id
                )));
            }
            if let Body::Progress {
                tokens_in_done,
                tokens_out,
            } = reply.body
            {
                on_progress(tokens_in_done, tokens_out);
                deadline = (Instant::now() + idle).min(hard_deadline);
                continue;
            }
            return Ok(reply);
        }
    }

    /// The next stdout line before `deadline`; every failure kills the worker.
    fn next_line(&mut self, deadline: Instant) -> Result<String> {
        let left = deadline.saturating_duration_since(Instant::now());
        match self.lines.recv_timeout(left) {
            Ok(Line::Text(raw)) => Ok(raw),
            Ok(Line::TooLong) => Err(self.died("worker line exceeds the size limit")),
            Ok(Line::Err(e)) => Err(self.died(&format!("cannot read from worker: {e}"))),
            Err(RecvTimeoutError::Timeout) => {
                self.kill();
                Err(LlmError::Timeout)
            }
            Err(RecvTimeoutError::Disconnected) => {
                let why = match &mut self.proc {
                    Proc::Child(child) => match child.wait().ok() {
                        Some(s) => format!("worker exited ({s})"),
                        None => "worker exited".to_string(),
                    },
                    Proc::Thread { .. } => "the engine stopped".to_string(),
                };
                Err(self.died(&why))
            }
        }
    }

    /// Kill the worker and build the error, with its last stderr lines.
    fn died(&mut self, why: &str) -> LlmError {
        self.kill();
        let tail = self.stderr_tail();
        self.report_exit(why);
        if tail.is_empty() {
            LlmError::Worker(why.to_string())
        } else {
            LlmError::Worker(format!("{why}; worker stderr:\n{tail}"))
        }
    }

    /// Writes `<utc>-worker.txt` into the diagnostics folder: our own
    /// description of the failure and the scrubbed stderr ring.
    fn report_exit(&self, why: &str) {
        let Some(dir) = ghi_diag::dir() else { return };
        let ring: Vec<String> = self
            .tail
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect();
        // `why` and the ring both go through the scrubber.
        ghi_diag::write_worker_exit(dir, why, &ring);
        log::warn!("llm worker died");
    }

    fn stderr_tail(&mut self) -> String {
        // Give the drain thread a moment to flush what the dead child wrote.
        let until = Instant::now() + Duration::from_millis(200);
        while Instant::now() < until && !self.threads.last().is_none_or(|t| t.is_finished()) {
            thread::sleep(Duration::from_millis(10));
        }
        let tail = self.tail.lock().unwrap_or_else(|e| e.into_inner());
        let skip = tail.len().saturating_sub(TAIL_SHOWN);
        tail.iter()
            .skip(skip)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Kill the worker and reap it; frees the model memory. An in-process
    /// engine is told to stop and frees it after its current token (not
    /// joined: a decode step must not block the caller).
    pub fn kill(&mut self) {
        self.stdin = None;
        match &mut self.proc {
            Proc::Child(child) => {
                let _ = child.kill();
                let _ = child.wait();
                let pid = child.id();
                live().retain(|&p| p != pid);
            }
            Proc::Thread { stop, .. } => stop.store(true, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// The worker process id (none for an in-process engine); the tests using it are Unix-only.
    #[cfg(all(test, unix))]
    fn pid(&self) -> Option<u32> {
        match &self.proc {
            Proc::Child(child) => Some(child.id()),
            Proc::Thread { .. } => None,
        }
    }

    /// Stops an in-process engine from another thread (it gives up after its
    /// current step and the request in flight fails); `None` for a worker
    /// process, which only its owner kills.
    pub fn stopper(&self) -> Option<Arc<dyn Fn() + Send + Sync>> {
        match &self.proc {
            Proc::Child(_) => None,
            Proc::Thread { stop, .. } => {
                let stop = Arc::clone(stop);
                Some(Arc::new(move || {
                    stop.store(true, std::sync::atomic::Ordering::Relaxed)
                }))
            }
        }
    }

    /// The worker has ended (process reaped, or engine thread returned).
    fn exited(&mut self) -> bool {
        match &mut self.proc {
            Proc::Child(child) => !matches!(child.try_wait(), Ok(None)),
            Proc::Thread { engine, .. } => engine.as_ref().is_none_or(JoinHandle::is_finished),
        }
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        if let Proc::Thread { stop, engine } = &mut self.proc {
            // Stop now (a busy engine gives up after its current step), close
            // its input, and leave the thread to the next engine's wait.
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            self.stdin = None;
            if let Some(handle) = engine.take() {
                *previous_engine() = Some(handle);
            }
            self.threads.clear();
            return;
        }
        if let Some(mut stdin) = self.stdin.take() {
            let line = serde_json::to_string(&Request {
                id: 0,
                op: Op::Shutdown,
            })
            .expect("request serializes");
            let _ = writeln!(stdin, "{line}").and_then(|_| stdin.flush());
        }
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        while Instant::now() < deadline && !self.exited() {
            thread::sleep(Duration::from_millis(20));
        }
        self.kill();
        // The pipe threads end at EOF on their own; not joined, since a
        // grandchild holding a pipe open must not block us.
        self.threads.clear();
    }
}

fn debug() -> bool {
    std::env::var_os("GHI_LLM_DEBUG").is_some_and(|v| v == "1")
}

/// stdout to lines; a line over [`MAX_LINE`] ends the stream.
fn read_lines(stdout: impl Read, tx: &mpsc::Sender<Line>) {
    let mut r = BufReader::new(stdout);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = match r
            .by_ref()
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut buf)
        {
            Ok(n) => n,
            Err(e) => {
                let _ = tx.send(Line::Err(e));
                return;
            }
        };
        if n == 0 {
            return;
        }
        if buf.last() == Some(&b'\n') {
            buf.pop();
        } else if buf.len() > MAX_LINE {
            let _ = tx.send(Line::TooLong);
            return;
        }
        let text = String::from_utf8_lossy(&buf).into_owned();
        if tx.send(Line::Text(text)).is_err() {
            // Nobody listens any more: keep reading to EOF so the writer
            // never blocks (or, in process, writes into a closed pipe).
            let _ = std::io::copy(&mut r, &mut std::io::sink());
            return;
        }
    }
}

/// stderr into a ring of the last [`STDERR_LINES`] lines; nothing else is kept.
fn drain_stderr(stderr: impl Read, tail: &Tail) {
    let dbg = debug();
    let mut r = BufReader::new(stderr);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r
            .by_ref()
            .take(STDERR_LINE_MAX as u64)
            .read_until(b'\n', &mut buf)
        {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        while matches!(buf.last(), Some(b'\n' | b'\r')) {
            buf.pop();
        }
        let line = String::from_utf8_lossy(&buf);
        if dbg {
            eprintln!("ghi-llm-worker: {line}");
        }
        let line: String = line.chars().take(TAIL_WIDTH).collect();
        let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
        if t.len() == STDERR_LINES {
            t.pop_front();
        }
        t.push_back(line);
    }
}

/// A worker for this build: the engine in process with feature `inproc`,
/// else the worker binary ([`worker_path`]).
pub fn start() -> Result<Sidecar> {
    #[cfg(feature = "inproc")]
    if in_process_default() {
        return Sidecar::in_process();
    }
    Sidecar::spawn(&worker_path()?)
}

/// Engines run in process on iOS (no child processes there). Elsewhere the
/// worker process stays the default even when a workspace build turns
/// `inproc` on for the phone; tests opt in with [`use_in_process`].
#[cfg(feature = "inproc")]
static IN_PROCESS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(cfg!(target_os = "ios"));

#[cfg(feature = "inproc")]
fn in_process_default() -> bool {
    IN_PROCESS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Makes [`start`] run engines in this process (host tests of the phone's way).
#[cfg(feature = "inproc")]
pub fn use_in_process(on: bool) {
    IN_PROCESS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// The engine thread of the last in-process sidecar, until it has ended.
fn previous_engine() -> std::sync::MutexGuard<'static, Option<JoinHandle<()>>> {
    static ENGINE: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
    ENGINE.lock().unwrap_or_else(|e| e.into_inner())
}

/// How long a new engine waits for a stopped one to free its model.
#[cfg(feature = "inproc")]
const PREVIOUS_ENGINE_GRACE: Duration = Duration::from_secs(30);

/// Waits for the previous in-process engine to end (it was told to stop);
/// an engine still busy after the grace refuses the new one (the job tries
/// again later) rather than holding two models at once.
#[cfg(feature = "inproc")]
fn wait_for_previous_engine() -> Result<()> {
    let deadline = Instant::now() + PREVIOUS_ENGINE_GRACE;
    loop {
        let mut slot = previous_engine();
        match slot.take() {
            None => return Ok(()),
            Some(h) if h.is_finished() => {
                let _ = h.join();
                return Ok(());
            }
            Some(h) => *slot = Some(h),
        }
        drop(slot);
        if Instant::now() >= deadline {
            return Err(LlmError::Worker(
                "the previous notes engine is still stopping".into(),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// `F_SETNOSIGPIPE` on Apple systems (elsewhere the Rust runtime already
/// ignores SIGPIPE in this crate's users).
#[cfg(feature = "inproc")]
fn no_sigpipe(fd: &impl std::os::fd::AsRawFd) {
    // <sys/fcntl.h> on macOS and iOS; not in the libc crate.
    #[cfg(target_vendor = "apple")]
    const F_SETNOSIGPIPE: libc::c_int = 73;
    #[cfg(target_vendor = "apple")]
    // SAFETY: a flag on a pipe end we own.
    unsafe {
        libc::fcntl(fd.as_raw_fd(), F_SETNOSIGPIPE, 1);
    }
    #[cfg(not(target_vendor = "apple"))]
    let _ = fd;
}

/// Where the worker binary is: `$GHI_LLM_WORKER`, else `ghi-llm-worker` next to
/// the running executable (or one level up, for `target/debug/deps` test binaries).
pub fn worker_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("GHI_LLM_WORKER") {
        let p = PathBuf::from(p);
        return if p.is_file() {
            Ok(p)
        } else {
            Err(LlmError::Worker(format!(
                "GHI_LLM_WORKER points at {}, which is not a file",
                p.display()
            )))
        };
    }
    let name = format!("ghi-llm-worker{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe()?;
    let dirs = exe
        .parent()
        .into_iter()
        .chain(exe.parent().and_then(Path::parent));
    for dir in dirs {
        let p = dir.join(&name);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(LlmError::Worker(format!(
        "{name} not found next to {}; build it or set GHI_LLM_WORKER",
        exe.display()
    )))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    const HELLO: &str = r#"echo '{"kind":"hello","protocol":3,"worker":"fake"}'"#;
    const T: Duration = Duration::from_secs(5);

    /// A fake worker: `/bin/sh -c "<hello>; <script>"`.
    fn fake(script: &str) -> Result<Sidecar> {
        fake_raw(&format!("{HELLO}; {script}"))
    }

    fn fake_raw(script: &str) -> Result<Sidecar> {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", script]);
        Sidecar::spawn_command(cmd, T)
    }

    fn pid_alive(pid: u32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    #[test]
    fn normal_reply() {
        let mut s = fake(r#"read l; echo '{"id":1,"kind":"health","loaded":true}'"#).unwrap();
        assert_eq!(s.worker_version(), "fake");
        let r = s.request(Op::Health, T).unwrap();
        assert_eq!(r.body, Body::Health { loaded: true });
    }

    #[test]
    fn hello_mismatch_is_rejected() {
        let err = fake_raw(r#"echo '{"kind":"hello","protocol":1,"worker":"x"}'; sleep 5"#)
            .err()
            .unwrap();
        assert!(err.to_string().contains("protocol 1"), "{err}");
        let err = fake_raw("echo hi; sleep 5").err().unwrap();
        assert!(matches!(err, LlmError::Worker(_)), "{err}");
    }

    #[test]
    fn progress_keeps_a_slow_request_alive_until_the_hard_cap() {
        // 4 progress lines 0.15 s apart (0.6 s total) with a 0.3 s idle limit.
        let p = r#"echo '{"id":1,"kind":"progress","tokens_in_done":512,"tokens_out":0}'"#;
        let script = format!(
            "read l; for i in 1 2 3 4; do {p}; sleep 0.15; done; echo '{{\"id\":1,\"kind\":\"health\",\"loaded\":true}}'"
        );
        let mut s = fake(&script).unwrap();
        let mut seen = 0;
        let r = s
            .request_live(
                Op::Health,
                Duration::from_millis(300),
                Duration::from_secs(5),
                &mut |a, _| {
                    assert_eq!(a, 512);
                    seen += 1;
                },
            )
            .unwrap();
        assert_eq!((r.body, seen), (Body::Health { loaded: true }, 4));
        // The same worker without progress is killed at the idle limit…
        let mut s = fake("read l; sleep 30").unwrap();
        let err = s
            .request_live(
                Op::Health,
                Duration::from_millis(200),
                Duration::from_secs(5),
                &mut |_, _| {},
            )
            .unwrap_err();
        assert!(matches!(err, LlmError::Timeout), "{err}");
        // …and progress forever still ends at the hard cap.
        let mut s = fake(&format!("read l; while true; do {p}; sleep 0.05; done")).unwrap();
        let t = Instant::now();
        let err = s
            .request_live(
                Op::Health,
                Duration::from_millis(300),
                Duration::from_millis(600),
                &mut |_, _| {},
            )
            .unwrap_err();
        assert!(matches!(err, LlmError::Timeout), "{err}");
        assert!(t.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn live_workers_are_registered_and_can_be_killed_while_busy() {
        let mut s = fake("read l; sleep 30").unwrap();
        let pid = s.pid().expect("a worker process");
        assert!(live().contains(&pid));
        // Busy: the request is in flight and nothing answers.
        s.stdin
            .as_mut()
            .map(|i| writeln!(i, "{{}}").and_then(|_| i.flush()));
        kill_pid(pid);
        let err = s.request(Op::Health, T).unwrap_err();
        assert!(matches!(err, LlmError::Worker(_)), "{err}");
        assert!(!pid_alive(pid));
        assert!(!live().contains(&pid), "reaped workers leave the registry");
    }

    #[test]
    fn timeout_kills_the_child() {
        let mut s = fake("read l; sleep 30").unwrap();
        let pid = s.pid().expect("a worker process");
        let err = s
            .request(Op::Health, Duration::from_millis(200))
            .unwrap_err();
        assert!(matches!(err, LlmError::Timeout), "{err}");
        assert!(!pid_alive(pid));
    }

    #[test]
    fn crash_is_a_worker_error_with_stderr() {
        let mut s = fake("read l; echo 'boom: out of memory' >&2; exit 3").unwrap();
        let err = s.request(Op::Health, T).unwrap_err();
        assert!(matches!(err, LlmError::Worker(_)), "{err}");
        assert!(err.to_string().contains("boom: out of memory"), "{err}");
    }

    #[test]
    fn garbage_is_ignored_then_reply_accepted() {
        let mut s =
            fake(r#"read l; echo not json; echo '{"id":1,"kind":"health","loaded":false}'"#)
                .unwrap();
        let r = s.request(Op::Health, T).unwrap();
        assert_eq!(r.body, Body::Health { loaded: false });
    }

    #[test]
    fn endless_garbage_is_a_worker_error() {
        let mut s = fake("read l; while true; do echo junk; done").unwrap();
        let err = s.request(Op::Health, T).unwrap_err();
        assert!(matches!(err, LlmError::Worker(_)), "{err}");
    }

    #[test]
    fn id_mismatch_is_a_worker_error() {
        let mut s = fake(r#"read l; echo '{"id":99,"kind":"health","loaded":true}'"#).unwrap();
        let err = s.request(Op::Health, T).unwrap_err();
        assert!(matches!(err, LlmError::Worker(_)), "{err}");
    }

    #[test]
    fn oversized_line_is_a_worker_error() {
        let mut s = fake("read l; head -c 9000000 /dev/zero | tr '\\0' x; echo").unwrap();
        let err = s.request(Op::Health, T).unwrap_err();
        assert!(err.to_string().contains("size limit"), "{err}");
    }

    #[test]
    fn stderr_flood_does_not_block_the_reply() {
        let mut s = fake(
            r#"read l; head -c 10000000 /dev/zero | tr '\0' e >&2; echo >&2 last-line; echo '{"id":1,"kind":"health","loaded":true}'"#,
        )
        .unwrap();
        let r = s.request(Op::Health, Duration::from_secs(20)).unwrap();
        assert_eq!(r.body, Body::Health { loaded: true });
        assert!(s.tail.lock().unwrap().len() <= STDERR_LINES);
    }

    #[test]
    fn drop_reaps_a_worker_that_ignores_shutdown() {
        let s = fake("sleep 30").unwrap();
        let pid = s.pid().expect("a worker process");
        let t = Instant::now();
        drop(s);
        assert!(t.elapsed() < Duration::from_secs(5));
        assert!(!pid_alive(pid));
    }

    #[test]
    fn missing_worker_is_reported() {
        let err = Sidecar::spawn(Path::new("/nonexistent/ghi-llm-worker"))
            .err()
            .unwrap();
        assert!(matches!(err, LlmError::Worker(_)));
    }
}
