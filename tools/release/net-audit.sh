#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Network audit (phase 12 step 2, doc 05 section 3): sample every socket the app, its
# `ghi-llm-worker` children and the WebKit networking process open, every 0.5 s, and report
# each one. In --strict mode any socket is a failure (exit 1): "Strict offline" must open none.
#
#   tools/release/net-audit.sh [--strict] [--allow-loopback] [--interval S] [--out DIR]
#                              [--ghi PATH] [--speech WAV] [--dns]  --cli
#   tools/release/net-audit.sh [--strict] ... --app NAME [--duration S]      # drive the real app
#   tools/release/net-audit.sh [--strict] ... -- COMMAND [ARGS...]          # audit any workload
#   tools/release/net-audit.sh --selftest                                    # prove it sees sockets
#
# What is sampled (`lsof -nP -i -a -p <pids>`, so TCP and UDP, v4 and v6, listening or connected)
#   - the workload process and all its descendants (the CLI, `ghi-llm-worker`, helpers),
#   - processes named NAME (--app; default match is exact process name, e.g. Ghira),
#   - `com.apple.WebKit.Networking`: the WKWebView network process is an XPC service launched
#     by launchd, not a child of the app, so it is attributed by NEW pids: only those not
#     running when the audit started count. Start the audit BEFORE launching the app and quit
#     other WebKit apps (Safari, Mail...) first, or their processes are not attributable.
#
# Modes
#   --cli     scripted headless meeting against a temp store (never your data): a tone replay
#             through `ghi session` (+ `--process` when this ghi has speech engines and the
#             models are installed), `ghi jobs`, `ghi store list`, and a `ghi models fetch
#             --strict-offline` that must be refused before any connection. Needs a DEBUG ghi
#             (default target/debug/ghi: file key store, no Keychain items).
#   --app N   the owner drives the real app (see docs/release/network-audit.md) while this
#             samples; stops after --duration seconds or on Ctrl-C.
#
# Blind spots (also in the doc): a connection that opens and closes inside one interval is not
# seen (confirm with Little Snitch); DNS goes through mDNSResponder, so a lookup is NOT a
# socket of the app (--dns additionally logs mDNSResponder activity with `log stream`, best
# effort); sockets of other users' processes need root.
#
# Output: DIR/net-audit.json and DIR/net-audit.txt (default ./net-audit-<time>/). Only
# addresses, ports, process names and counts: no content.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
export GHI_AUDIT_ROOT="$root"
exec python3 - "$@" <<'PY'
import argparse, ipaddress, json, os, re, signal, subprocess, sys, tempfile, threading, time
from datetime import datetime, timezone
from pathlib import Path

root = Path(os.environ["GHI_AUDIT_ROOT"])
ap = argparse.ArgumentParser(prog="net-audit.sh", add_help=True)
ap.add_argument("--strict", action="store_true", help="any socket is a failure")
ap.add_argument("--allow-loopback", action="store_true", help="strict: tolerate 127.0.0.1/::1 only")
ap.add_argument("--interval", type=float, default=0.5)
ap.add_argument("--out", type=Path)
ap.add_argument("--ghi", default=os.environ.get("GHI_BIN", ""))
ap.add_argument("--speech", type=Path, help="speech WAV for --cli (default: a tone)")
ap.add_argument("--dns", action="store_true", help="also log mDNSResponder activity (best effort)")
ap.add_argument("--cli", action="store_true")
ap.add_argument("--app", help="process name of the app to watch (e.g. Ghira)")
ap.add_argument("--duration", type=float, help="--app: stop after this many seconds")
ap.add_argument("--selftest", action="store_true")
ap.add_argument("workload", nargs="*", help="after `--`: a command to run and audit")
args = ap.parse_args()
if args.selftest:
    args.strict = True
    args.workload = [sys.executable, "-c", (
        "import socket,time\n"
        "s=socket.socket();s.bind(('127.0.0.1',0));s.listen(1)\n"
        "c=socket.create_connection(s.getsockname());time.sleep(2.5)")]
if not (args.cli or args.app or args.workload):
    ap.error("choose --cli, --app NAME, -- COMMAND or --selftest")

out = args.out or Path.cwd() / ("net-audit-" + datetime.now().strftime("%Y%m%d-%H%M%S"))
out.mkdir(parents=True, exist_ok=True)

WEBKIT = "com.apple.WebKit.Networking"
ALWAYS = {"ghi-llm-worker"}

def procs():
    """pid -> (ppid, comm basename), from ps."""
    txt = subprocess.run(["ps", "-axo", "pid=,ppid=,comm="], capture_output=True, text=True).stdout
    table = {}
    for line in txt.splitlines():
        m = re.match(r"\s*(\d+)\s+(\d+)\s+(.*)$", line)
        if m:
            table[int(m[1])] = (int(m[2]), os.path.basename(m[3].strip()))
    return table

baseline_webkit = {p for p, (_, c) in procs().items() if c == WEBKIT}

def descendants(table, roots):
    kids = {}
    for p, (pp, _) in table.items():
        kids.setdefault(pp, []).append(p)
    seen, todo = set(), list(roots)
    while todo:
        p = todo.pop()
        if p in seen: continue
        seen.add(p); todo.extend(kids.get(p, []))
    return seen

def classify(addr):
    """Where a socket points: none (listening/unconnected), loopback, lan, internet."""
    if not addr: return "none"
    host = addr.rsplit(":", 1)[0].strip("[]")
    if host in ("*", ""): return "none"
    try: ip = ipaddress.ip_address(host.split("%")[0])
    except ValueError: return "internet"
    if ip.is_loopback: return "loopback"
    if ip.is_private or ip.is_link_local: return "lan"
    return "internet"

sockets = {}          # key -> record
seen_procs = {}       # pid -> comm
samples = 0
stop = threading.Event()
roots = []            # workload pids (descendants are audited)
lock = threading.Lock()

def sample():
    global samples
    table = procs()
    pids = descendants(table, roots) if roots else set()
    for p, (_, c) in table.items():
        if c in ALWAYS or (args.app and c == args.app):
            pids |= descendants(table, [p])
        if c == WEBKIT and p not in baseline_webkit:
            pids.add(p)
    pids.discard(os.getpid())
    pids = {p for p in pids if p in table}
    samples += 1
    if not pids: return
    for p in pids: seen_procs[p] = table[p][1]
    r = subprocess.run(["lsof", "-nP", "-i", "-a", "-p", ",".join(map(str, sorted(pids))),
                        "-F", "pcPnT"], capture_output=True, text=True)
    cur = {}
    now = time.time()
    for line in r.stdout.splitlines():
        k, v = line[0], line[1:]
        if k == "p": cur = {"pid": int(v)}
        elif k == "c": cur["proc"] = v
        elif k == "P": cur["proto"] = v
        elif k == "n":
            local, _, remote = v.partition("->")
            rec = {**cur, "local": local, "remote": remote, "state": cur.get("state", "")}
            key = (rec["proc"], rec["proto"], local, remote)
            with lock:
                e = sockets.setdefault(key, {"proc": rec["proc"], "pid": rec["pid"], "proto": rec["proto"],
                    "local": local, "remote": remote,
                    "kind": classify(remote) if remote else ("loopback" if classify(local) == "loopback" else "none"),
                    "state": "", "first_s": now - t0, "last_s": now - t0, "samples": 0})
                e["last_s"] = now - t0; e["samples"] += 1
        elif k == "T" and v.startswith("ST="):
            for e in list(sockets.values())[-1:]:
                e["state"] = v[3:]

def sampler():
    while not stop.is_set():
        try: sample()
        except Exception as exc:  # never lose the audit to one failed tick
            print(f"warning: sample failed: {exc}", file=sys.stderr)
        stop.wait(args.interval)

dns_proc = None
if args.dns:
    dns_log = open(out / "mdnsresponder.log", "w")
    try:
        dns_proc = subprocess.Popen(["log", "stream", "--style", "compact", "--predicate",
                                     'process == "mDNSResponder"'], stdout=dns_log, stderr=subprocess.DEVNULL)
    except OSError:
        print("warning: `log stream` unavailable; no DNS log", file=sys.stderr)

t0 = time.time()
th = threading.Thread(target=sampler, daemon=True); th.start()
workload_rc = None
notes = []

def run_workload(cmd, **kw):
    p = subprocess.Popen(cmd, **kw)
    roots.append(p.pid)
    rc = p.wait()
    return rc

try:
    if args.cli:
        ghi = args.ghi or str(root / "target/debug/ghi")
        if not os.access(ghi, os.X_OK):
            subprocess.run(["cargo", "build", "-p", "ghi-cli"], cwd=root, check=True)
        work = Path(tempfile.mkdtemp(prefix="ghi-netaudit."))
        wav = args.speech
        if not wav:
            import math, struct, wave
            wav = work / "tone.wav"
            w = wave.open(str(wav), "wb"); w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000)
            for s in range(20):
                f = 400 + 20 * s
                w.writeframes(b"".join(struct.pack("<h", int(9000 * math.sin(2 * math.pi * f * i / 16000))) for i in range(16000)))
            w.close()
        store = str(work / "store")
        engines = '"name"' in subprocess.run([ghi, "version", "--json"], capture_output=True, text=True).stdout
        sess = [ghi, "session", "--dir", store, "--replay", str(wav), "--speed", "4"]
        sess += ["--process"] if engines else ["--record-only"]
        notes.append("speech engines: " + ("yes (session --process)" if engines else "no (record-only)"))
        steps = [sess, [ghi, "jobs", "--dir", store], [ghi, "store", "--dir", store, "list"],
                 [ghi, "models", "fetch", "qwen3-4b", "--dir", str(work / "models"), "--strict-offline"]]
        for st in steps:
            rc = run_workload(st, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            notes.append(f"step {' '.join(st[1:3])}: exit {rc}")
        workload_rc = 0
    elif args.workload:
        workload_rc = run_workload(args.workload)
    else:
        deadline = None if args.duration is None else time.time() + args.duration
        signal.signal(signal.SIGINT, lambda *_: stop.set())
        print(f"auditing process '{args.app}' + ghi-llm-worker + new {WEBKIT}; Ctrl-C to finish", file=sys.stderr)
        while not stop.is_set() and (deadline is None or time.time() < deadline):
            time.sleep(0.2)
finally:
    time.sleep(args.interval * 1.5)
    stop.set(); th.join(timeout=5)
    if dns_proc: dns_proc.terminate()

dur = time.time() - t0
socks = sorted(sockets.values(), key=lambda e: (e["first_s"], e["proc"]))
by_kind = {}
for e in socks: by_kind[e["kind"]] = by_kind.get(e["kind"], 0) + 1
offending = [e for e in socks if not (args.allow_loopback and e["kind"] == "loopback")] if args.strict else []
report = {
    "schema": "ghi.net-audit/1",
    "generated": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    "mode": "strict" if args.strict else "default",
    "interval_s": args.interval, "duration_s": round(dur, 1), "samples": samples,
    "processes_seen": sorted(set(seen_procs.values())),
    "sockets_total": len(socks), "sockets_by_kind": by_kind, "sockets": socks,
    "notes": notes,
    "blind_spots": ["connections shorter than the interval", "DNS (mDNSResponder is the socket owner)",
                    "processes of other users (need root)"],
    "result": "fail" if offending else "pass",
}
(out / "net-audit.json").write_text(json.dumps(report, indent=2) + "\n")
lines = [f"net-audit: mode={report['mode']} duration={report['duration_s']}s samples={samples} "
         f"processes={', '.join(report['processes_seen']) or '-'}"]
lines += [f"  {n}" for n in notes]
lines.append(f"{'proc':<22} {'proto':<5} {'kind':<9} {'state':<12} {'first_s':>7} {'last_s':>7}  local -> remote")
for e in socks:
    lines.append(f"{e['proc'][:22]:<22} {e['proto']:<5} {e['kind']:<9} {e['state'][:12]:<12} "
                 f"{e['first_s']:7.1f} {e['last_s']:7.1f}  {e['local']}{' -> ' + e['remote'] if e['remote'] else ''}")
lines.append(f"sockets seen: {len(socks)} {by_kind or ''}")
lines.append(f"RESULT: {report['result'].upper()}" + (f" ({len(offending)} socket(s) in strict mode)" if offending else ""))
txt = "\n".join(lines) + "\n"
(out / "net-audit.txt").write_text(txt)
print(txt, end="")
print(f"report: {out}/net-audit.json")
if args.selftest:
    ok = bool(offending) and any(e["kind"] == "loopback" for e in socks)
    print("selftest:", "OK (the audit sees sockets and strict mode fails)" if ok else "FAILED (sampler is blind)")
    sys.exit(0 if ok else 1)
sys.exit(1 if offending else 0)
PY
