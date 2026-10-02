# Network audit

Verifies the network policy of doc 05 section 3 on the release candidate:

1. **Strict offline**: with the toggle on, a 60 min meeting plus all processing opens **zero**
   outbound connections.
2. **Default mode**: only allowlisted, content-free traffic (model downloads from
   `huggingface.co` and its CDN `*.hf.co`, our model mirror, the update feed) and **no request
   body or URL carries user content**.

Three layers, cheapest first. The first is automatic and runs anywhere; the second and third are
the owner's evidence for `SECURITY.md` because they need the real signed app.

| Layer | What it proves | Needs |
|---|---|---|
| A. `tools/release/net-audit.sh` | Per-process socket sampling (app, `ghi-llm-worker`, WebKit networking) | the app or the CLI |
| B. Little Snitch (or `pf` + `tcpdump`) | Every connection attempt and every DNS lookup, including ones shorter than A's sample interval | Little Snitch 6 or sudo |
| C. Intercepting proxy | Default mode carries no content (bodies, URLs, headers) | mitmproxy, a trusted test CA |

## A. `tools/release/net-audit.sh`

Samples `lsof -nP -i -a -p <pids>` every 0.5 s for these processes:

- the workload (the CLI, or the app named by `--app`) and all its descendants,
- every process named `ghi-llm-worker`,
- every `com.apple.WebKit.Networking` process **that did not exist when the audit started**
  (the WKWebView network process is an XPC service owned by launchd, not a child of the app).

It writes `net-audit.json` and `net-audit.txt` (process, protocol, local and remote address,
state, first and last time seen) and exits non-zero in `--strict` mode if any socket was seen.
Loopback is counted too (`--allow-loopback` tolerates `127.0.0.1` and `::1` only).

### Prove the tool works (positive control)

```sh
tools/release/net-audit.sh --selftest
```

Runs a child that opens a loopback listener and connection; the audit must see both and strict
mode must fail. If it prints `selftest: FAILED`, the sampler is blind (for example, `lsof` is
blocked by a sandbox) and no result of this tool means anything.

### Automated: headless CLI, strict offline

```sh
cargo build -p ghi-cli                       # debug build: file key store, no Keychain items
tools/release/net-audit.sh --strict --cli
# with speech engines and models (cargo build -p ghi-cli --features ghi-cli/nemo):
GHI_BIN=target/debug/ghi tools/release/net-audit.sh --strict --cli --speech some-speech.wav
```

`--cli` runs, against a temp store (never your data), a replay session (`ghi session`, with
`--process` when the build has speech engines, which also runs the notes jobs and the local LLM
worker), `ghi jobs`, `ghi store list`, and `ghi models fetch qwen3-4b --strict-offline`, which
must be refused before any connection. Expected: `sockets seen: 0`, `RESULT: PASS`.

Any other workload: `tools/release/net-audit.sh --strict -- <command ...>`.

### The real app, strict mode (owner)

1. Quit Safari, Mail and anything else that uses WebKit (their networking processes are not
   attributable, see above), and quit Ghira.
2. In a terminal: `tools/release/net-audit.sh --strict --app Ghira --duration 4500 --out ~/Desktop/audit-strict`
   (start it **before** launching the app). For a clean attribution of the app's own process
   name, check `ps -axo comm | grep -i ghira` once the app runs; pass that name to `--app`.
3. Launch Ghira. Settings > Privacy > turn **Strict offline** on.
4. Record a 60 min call (or replay a long recording into the mic), stop, and let the notes and
   final pass finish. Open, search, export and import a file. Try the cloud send and a model
   download: both must be refused.
5. Press Ctrl-C in the terminal. Expected: `RESULT: PASS`, no sockets for Ghira,
   `ghi-llm-worker` or the new WebKit networking process.

Default mode: repeat without `--strict`. The report lists every socket with its remote IP. The
only ones allowed are TLS (port 443) connections to Hugging Face and its CDN, the model mirror
and the update feed, and only while a model download or an update check ran. Resolve the IPs
yourself afterwards (`dig -x`, or compare with layer B, which shows host names). Anything else
is a finding.

### Blind spots (read before trusting a PASS)

- **Interval**: a connection that opens and closes within 0.5 s may be missed. Use
  `--interval 0.1` for a short scripted run, and layer B for the real evidence.
- **DNS**: lookups go through `mDNSResponder`, which owns the socket, so a hostname lookup by
  the app is **not** a socket of the app. `--dns` also logs `mDNSResponder` activity with
  `log stream` (best effort, may need admin rights and is noisy); the reliable check is layer B
  (`sudo tcpdump -i any -n 'port 53 or port 5353'` during the run: any query for a hostname the
  allowlist does not cover is a finding, though in strict mode there must be none from Ghira).
- **Other users' processes** and unattributable WebKit processes (see above).
- **Unix sockets** and XPC to system services are not network traffic and are not listed.
- A sandboxed shell may hide processes from `ps` or `lsof`; run it from Terminal.app.

## B. Little Snitch (or `pf` + `tcpdump`)

Little Snitch 6 (Network Monitor and a profile with logging) is the evidence the doc 05
acceptance names.

1. Install Little Snitch, create a profile "ghira-audit", set it to **Silent Mode: Deny** with
   **Log connections** on, and clear the rule list for Ghira, `ghi-llm-worker` and
   `com.apple.WebKit.Networking`.
2. Strict offline: run the same 60 min scenario as in A. In Network Monitor filter by Ghira,
   `ghi-llm-worker` and WebKit. Expected: **no connection attempt at all** (not even denied
   ones). Export the log (Little Snitch > Log > Export) and attach it to the release record.
3. Default mode: switch to Silent Mode: **Allow**, log on. Run first launch (model download),
   an update check (if the build has one) and a cloud send with a throw-away key. Expected
   destinations: `huggingface.co`, `*.hf.co`, the model mirror host, the update feed host
   (`github.com`, `*.githubusercontent.com`) and, only for the cloud send you triggered, the one
   provider host. Nothing else.

Without Little Snitch, the equivalent on a spare Mac:

```sh
sudo tcpdump -i any -n -w ghira.pcap 'not (host 127.0.0.1 or host ::1)'     # whole run
tcpdump -n -r ghira.pcap | awk '{print $3, $5}' | sort | uniq -c | sort -rn  # who talked to whom
tcpdump -n -r ghira.pcap 'port 53 or port 5353'                              # every DNS query
```

(`tcpdump` sees the machine's traffic, not a process's: stop other apps first.)

## C. Proxy capture: no content in default mode

`ghi-net` ignores `HTTP(S)_PROXY` and the system proxy (`.proxy(None)`) and verifies TLS with the
platform verifier, so a proxy must be transparent and its CA trusted by the system:

1. Install mitmproxy; run `mitmweb --mode local:Ghira` (macOS local capture mode, mitmproxy 11 or
   later; allow its network extension) and add its CA to the login keychain as trusted for SSL.
   Use a throw-away user account.
2. Create a meeting containing canary words (for example "kangaroo-7731 pho-bo-canary").
3. Strict off, cloud off. Trigger everything that is allowed to talk: first-launch model
   download (use a fresh models directory), "check for updates". Record, process, search,
   export, import, open Settings.
4. In mitmweb, export all flows. Expected: every flow is a `GET` (or `HEAD`) with an empty
   request body to an allowlisted host; the URL is a pinned model path or the update feed; no
   request header or URL contains the canary words, a meeting title, a name or an identifier
   (only `User-Agent: ghira/<version>`). `grep -c kangaroo flows.json` must be 0.
5. Cloud: with a throw-away key send the canary meeting with redaction on and off. The request
   body must equal the previewed payload byte for byte (compare the SHA-256 of the body with the
   preview's), contain no audio and, with redaction on, no names or numbers the preview
   masked. The host must be the provider's.

The code-level counterparts, run by `cargo test -p ghi-net`: `check_binds_host_port_path_and_body`,
`mismatch_opens_no_socket_and_keeps_attempts`, `config_ignores_the_environment_and_refuses_redirects`,
`strict_offline_is_refused_before_any_socket`. `tools/scripts/check-net-egress.sh` fails the
build if any network code appears outside `crates/ghi-net`.

## Recording the result

Put the strict and default `net-audit.txt`, the Little Snitch log and the proxy summary next to
the release record, and publish a one-paragraph result (date, build, macOS, tools, "0 connections
in strict mode; only allowlisted hosts in default mode") in `SECURITY.md`. Do not publish
captures: they contain your IP address and meeting-adjacent metadata.
