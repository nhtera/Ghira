// SPDX-License-Identifier: Apache-2.0
//! Listener audit (doc 07 §11, "Sync off -> no listener" and "Guard"; 15-L):
//! the process opens no listening socket and no outgoing connection unless a
//! hub is listening on a private LAN address, and never an internet
//! connection (`ghi_net::connections_opened()`).
//!
//! The counters are process-wide, so every test here takes one lock. The
//! socket tests need a private address of this computer: set
//! `GHI_SYNC_LAN_IP` (for example `GHI_SYNC_LAN_IP=$(ipconfig getifaddr en0)`);
//! without it they skip with a message. There is no loopback bypass.

mod common;

use std::io::{BufRead, BufReader};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use common::*;
use ghi_net::lan::{self, Listener};
use ghi_net::{connections_opened, is_lan, lan_connections_opened, listeners_open};
use ghi_sync::clock::SystemClock;
use ghi_sync::qr;
use ghi_sync::service::{Served, pair_via, run_session};

static LOCK: Mutex<()> = Mutex::new(());

/// The counters at the start of a test.
struct Baseline {
    listeners: u64,
    lan_conns: u64,
    internet: u64,
    _guard: MutexGuard<'static, ()>,
}

fn baseline() -> Baseline {
    let guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    Baseline {
        listeners: listeners_open(),
        lan_conns: lan_connections_opened(),
        internet: connections_opened(),
        _guard: guard,
    }
}

impl Baseline {
    fn assert_no_internet(&self) {
        assert_eq!(
            connections_opened(),
            self.internet,
            "sync opened an internet connection"
        );
    }
}

/// A private address of this computer, if the environment names one.
fn lan_ip() -> Option<IpAddr> {
    let Ok(text) = std::env::var("GHI_SYNC_LAN_IP") else {
        eprintln!("skipped: GHI_SYNC_LAN_IP is not set (a private address of this computer)");
        return None;
    };
    match text.trim().parse::<IpAddr>() {
        Ok(ip) if is_lan(ip) => Some(ip),
        Ok(_) => {
            eprintln!("skipped: GHI_SYNC_LAN_IP is not a private address");
            None
        }
        Err(_) => {
            eprintln!("skipped: GHI_SYNC_LAN_IP is not an address");
            None
        }
    }
}

/// `lsof -a -nP -iTCP -sTCP:LISTEN -p <pid>` as the listening `address:port`
/// names, or `None` when `lsof` is not there.
fn listening_names(pid: u32) -> Option<Vec<String>> {
    let out = Command::new("lsof")
        // `-a`: without it lsof ORs `-p` with `-i` and lists every listener.
        .args(["-a", "-nP", "-iTCP", "-sTCP:LISTEN", "-p", &pid.to_string()])
        .output()
        .ok()?;
    // lsof exits 1 when nothing matches; that is an empty list, not a failure.
    let text = String::from_utf8_lossy(&out.stdout);
    Some(
        text.lines()
            .skip(1)
            .filter_map(|l| l.split_whitespace().rev().nth(1).map(str::to_string))
            .collect(),
    )
}

// -------------------------------------------------------------- no sockets

#[test]
fn pairing_and_sessions_over_memory_open_no_socket_at_all() {
    let base = baseline();
    let (hub_n, spoke_a, spoke_b) = (node(), node(), node());
    let hub = hub_n.hub();
    pair(&hub, &hub_n, &spoke_a);
    pair(&hub, &hub_n, &spoke_b);
    let m = meeting(spoke_a.store(), "Over memory");
    sync(&hub, &spoke_a);
    sync(&hub, &spoke_b);
    assert_eq!(hub_n.store().get_meeting(&m).unwrap().title, "Over memory");
    assert_eq!(
        spoke_b.store().get_meeting(&m).unwrap().title,
        "Over memory"
    );
    assert_eq!(listeners_open(), base.listeners, "a listener was opened");
    assert_eq!(lan_connections_opened(), base.lan_conns, "a LAN connection");
    base.assert_no_internet();
}

#[test]
fn a_listener_refuses_every_address_that_is_not_private_and_opens_nothing() {
    let base = baseline();
    for ip in [
        "127.0.0.1",
        "::1",
        "169.254.1.1",
        "100.64.0.1",
        "8.8.8.8",
        "0.0.0.0",
        "::",
        "fe80::1",
        "2001:4860:4860::8888",
    ] {
        let ip: IpAddr = ip.parse().unwrap();
        let r = Listener::bind(&[ip], 0);
        assert!(
            matches!(r, Err(lan::LanError::NotLan(got)) if got == ip),
            "{ip}: bound or failed another way: {r:?}"
        );
        assert_eq!(listeners_open(), base.listeners, "{ip}: a listener is open");
    }
    // One bad address spoils the lot: nothing is bound for the good ones.
    let good: IpAddr = "10.255.255.254".parse().unwrap();
    let r = Listener::bind(&[good, "127.0.0.1".parse().unwrap()], 0);
    assert!(matches!(r, Err(lan::LanError::NotLan(_))), "{r:?}");
    assert_eq!(listeners_open(), base.listeners);
    // The empty list binds nothing either.
    assert!(Listener::bind(&[], 0).is_err());
    assert_eq!(listeners_open(), base.listeners);
}

#[test]
fn a_connection_to_a_non_private_address_is_refused_before_a_socket_opens() {
    let base = baseline();
    for addr in [
        "127.0.0.1:9",
        "[::1]:9",
        "169.254.169.254:80",
        "8.8.8.8:443",
        "100.64.0.1:7",
    ] {
        let addr: SocketAddr = addr.parse().unwrap();
        let r = lan::connect(addr, Duration::from_millis(200));
        assert!(
            matches!(r, Err(lan::LanError::NotLan(_))),
            "{addr}: {:?}",
            r.map(|_| ())
        );
    }
    assert_eq!(
        lan_connections_opened(),
        base.lan_conns,
        "a refused address was dialled"
    );
    base.assert_no_internet();
}

// -------------------------------------------------------- real LAN sockets

#[test]
fn a_hub_on_the_lan_address_listens_only_there_pairs_and_syncs_and_leaves_nothing_open() {
    let Some(ip) = lan_ip() else {
        return;
    };
    let base = baseline();
    let listener = Listener::bind(&[ip], 0).unwrap();
    assert_eq!(listeners_open(), base.listeners + 1);
    let addrs = listener.local_addrs();
    assert!(!addrs.is_empty());
    assert!(
        addrs.iter().all(|a| a.ip() == ip && is_lan(a.ip())),
        "{addrs:?}"
    );
    // The operating system agrees: this process listens on that address and
    // port, and nowhere else.
    match listening_names(std::process::id()) {
        Some(names) => {
            let want = format!("{ip}:{}", listener.port());
            assert!(!names.is_empty(), "lsof shows no listener");
            assert!(
                names.iter().all(|n| *n == want),
                "listening names {names:?}, expected only {want}"
            );
        }
        None => eprintln!("lsof not available: OS-level listener check skipped"),
    }

    let (hub_n, spoke) = (node(), node());
    let hub = hub_n.hub();
    let code = hub.open_pairing_code(&addrs).unwrap();
    let limits = listener.limits();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let hub2 = hub.clone();
    let server = thread::spawn(move || {
        let mut served = Vec::new();
        while stop_rx.try_recv().is_err() {
            match listener.accept_timeout(Duration::from_millis(100)) {
                Ok(Some((stream, peer))) => {
                    served.push(hub2.serve(stream, Some(peer.ip()), Some(&limits)));
                }
                Ok(None) => {}
                Err(e) => panic!("accept: {e}"),
            }
        }
        served
    });

    // Pair, then sync a meeting, over real sockets to the LAN address.
    let payload = qr::parse(&code).unwrap();
    assert!(payload.addrs.iter().all(|a| is_lan(a.ip())));
    let (_, via) = pair_via(
        spoke.store().as_ref(),
        &spoke.identity,
        &payload,
        "iPhone",
        "ios",
    )
    .unwrap();
    let gid = meeting(spoke.store(), "Over the LAN");
    let (report, _) = run_session(
        spoke.dyn_store(),
        Arc::new(SystemClock),
        &spoke.identity,
        &[via],
    )
    .unwrap();
    assert!(report.rows_pushed >= 5, "{report:?}");
    assert_eq!(
        hub_n.store().get_meeting(&gid).unwrap().title,
        "Over the LAN"
    );
    assert!(
        lan_connections_opened() >= base.lan_conns + 2,
        "pairing and the session dial the LAN"
    );
    base.assert_no_internet();

    stop_tx.send(()).unwrap();
    let served = server.join().unwrap();
    assert_eq!(served.len(), 2, "{served:?}");
    assert!(
        matches!(served[0], Ok(Served::Paired(_))),
        "{:?}",
        served[0]
    );
    assert!(
        matches!(served[1], Ok(Served::Session(_))),
        "{:?}",
        served[1]
    );
    // The listener went with the thread: nothing is open any more.
    assert_eq!(
        listeners_open(),
        base.listeners,
        "the listener outlived its owner"
    );
    if let Some(names) = listening_names(std::process::id()) {
        assert!(names.is_empty(), "still listening: {names:?}");
    }
    base.assert_no_internet();
}

/// The `ghi` binary built next to the test executables, if there is one.
fn ghi_binary() -> Option<PathBuf> {
    let mut p = std::env::current_exe().ok()?;
    p.pop(); // deps
    p.pop(); // debug | release
    p.push("ghi");
    p.is_file().then_some(p)
}

#[test]
fn the_ghi_hub_process_listens_on_the_bound_lan_address_and_nowhere_else() {
    let Some(ip) = lan_ip() else {
        return;
    };
    let Some(bin) = ghi_binary() else {
        eprintln!(
            "skipped: no built `ghi` binary next to the test executables (cargo build -p ghi-cli)"
        );
        return;
    };
    let _base = baseline();
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("hub");
    let mut child = Command::new(bin)
        .args(["sync", "serve", "--no-pair", "--dir"])
        .arg(&data)
        .args(["--bind", &ip.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    // Wait for "listening on <addr:port>" on stderr; never print other lines.
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.contains("listening on") {
                let _ = tx.send(line);
                break;
            }
        }
    });
    let started = rx.recv_timeout(Duration::from_secs(60));
    let result = (|| {
        let line = started.map_err(|_| "ghi sync serve did not start listening".to_string())?;
        let want_prefix = format!("{ip}:");
        let port = line
            .split("listening on ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .map(|s| s.trim_end_matches(',').to_string())
            .filter(|s| s.starts_with(&want_prefix))
            .ok_or_else(|| "it listens on another address than the one given".to_string())?;
        let Some(names) = listening_names(pid) else {
            eprintln!("lsof not available: listener check skipped");
            return Ok(());
        };
        if names.is_empty() {
            return Err("lsof shows no listener for the serve process".to_string());
        }
        if let Some(other) = names.iter().find(|n| **n != port) {
            return Err(format!("it also listens on {other}"));
        }
        Ok(())
    })();
    let _ = child.kill();
    let _ = child.wait();
    if let Err(why) = result {
        panic!("{why}");
    }
    // Killed: nothing of it is left listening.
    if let Some(names) = listening_names(pid) {
        assert!(names.is_empty(), "{names:?}");
    }
}
