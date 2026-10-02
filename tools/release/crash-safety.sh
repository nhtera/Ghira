#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Crash-safety test (phase 12 step 4, RT-10): `kill -9` the recording process at
# random moments, recover, and prove the loss is <= 5 s every time.
#
#   tools/release/crash-safety.sh [-n N] [--seed S] [--duration SECS] [--max-loss SECS]
#                                 [--final-pass [--speech WAV]] [--ghi PATH] [--keep]
#
# How it works
#   1. A time-coded WAV (16 kHz mono) is generated: second k carries a pure tone at
#      400 + 20*(k mod 40) Hz, so every second of recovered audio says which second it is.
#   2. For each of N iterations a fresh store is created in a temp dir and
#      `ghi session --record-only --speed 1 --replay marker.wav` records it in real time
#      (record-only: the pipeline and the encrypted bundle writer of the app, without the
#      speech engines). The process gets SIGKILL at a random offset in [3, DURATION-3] s.
#   3. Recovery is what the app does at launch: `ghi jobs --dir` (store open cuts the
#      bundle back to its last good page, recover() closes the meeting), then
#      `ghi store audio` decrypts the track to a WAV.
#   4. A Goertzel detector reads the tone of every second of the recovered WAV; the intact
#      audio is the longest prefix whose tones match. loss = audio fed - intact audio.
#      "Audio fed" is the time of the last level-meter event the process printed before it
#      died (the pipeline's own clock, 0.1 s resolution), minus the first `recording` event.
#      Assert loss <= --max-loss (5 s).
#   5. --final-pass (needs a `ghi` built with `--features ghi-cli/nemo`, the speech models
#      and the local LLM): record a speech clip with `--speed 0 --record-only`, start
#      `ghi jobs`, `kill -9` it once the final pass reports progress, run `ghi jobs`
#      again and assert the final pass is retried and completes (meeting `ready`).
#
# Assumptions and limits
#   - macOS or Linux; python3 (stdlib only), no root. A process kill is covered; the 5 hard
#     power-offs of the phase need a machine and a person (docs/release/smoke-checklist.md):
#     after a power loss the Opus bundle may lose up to ~3 s (formats.md section 2).
#   - Never touches the real data directory: every store lives in a mktemp dir with the
#     debug key file `<dir>.devkey` next to it. Use a DEBUG build (default
#     target/debug/ghi); a release build would store a Keychain item per temp dir.
#   - The loss figure ignores replay start-up latency; it measures what the writer had been
#     handed when it died versus what survived.
#   - Seeded: the kill offsets come from bash's RANDOM, so --seed S repeats a run.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
n=20 seed="" dur=30 max_loss=5 final_pass=0 keep=0 speech="" ghi="${GHI_BIN:-}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    -n) n="$2"; shift 2 ;;
    --seed) seed="$2"; shift 2 ;;
    --duration) dur="$2"; shift 2 ;;
    --max-loss) max_loss="$2"; shift 2 ;;
    --final-pass) final_pass=1; shift ;;
    --speech) speech="$2"; shift 2 ;;
    --ghi) ghi="$2"; shift 2 ;;
    --keep) keep=1; shift ;;
    -h|--help) sed -n '3,40p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
(( dur >= 10 )) || { echo "--duration must be >= 10" >&2; exit 2; }
[[ -n "$seed" ]] || seed=$(( $(date +%s) % 32768 ))
RANDOM=$seed
echo "crash-safety: n=$n duration=${dur}s seed=$seed max_loss=${max_loss}s"

if [[ -z "$ghi" ]]; then
  ghi="$root/target/debug/ghi"
  [[ -x "$ghi" ]] || (cd "$root" && cargo build -p ghi-cli)
fi
[[ -x "$ghi" ]] || { echo "ghi binary not found: $ghi" >&2; exit 2; }

work="$(mktemp -d "${TMPDIR:-/tmp}/ghi-crash.XXXXXX")"
cleanup() { if (( keep )) || [[ -n "${failed:-}" ]]; then echo "work dir kept: $work"; else rm -rf "$work"; fi; }
trap cleanup EXIT

cat > "$work/marker.py" <<'PY'
import json, math, struct, sys, wave

RATE = 16000
def freq(sec): return 400 + 20 * (sec % 40)

def gen(path, secs):
    w = wave.open(path, "wb"); w.setnchannels(1); w.setsampwidth(2); w.setframerate(RATE)
    for s in range(secs):
        f = freq(s)
        w.writeframes(b"".join(struct.pack("<h", int(12000 * math.sin(2 * math.pi * f * i / RATE)))
                               for i in range(RATE)))
    w.close()

def goertzel(x, f):
    c = 2 * math.cos(2 * math.pi * f / RATE)
    s1 = s2 = 0.0
    for v in x:
        s1, s2 = v + c * s1 - s2, s1
    return s2 * s2 + s1 * s1 - c * s1 * s2

def tone(x):
    best = max(range(40), key=lambda k: goertzel(x, freq(k)))
    return best  # index into the 40-tone table

def decode(path):
    w = wave.open(path, "rb")
    assert w.getframerate() == RATE and w.getnchannels() == 1, "expected 16 kHz mono"
    data = struct.unpack("<%dh" % w.getnframes(), w.readframes(w.getnframes()))
    n = len(data)
    intact = 0.0
    sec = 0
    while sec * RATE < n:
        lo, hi = sec * RATE, min((sec + 1) * RATE, n)
        part = data[lo:hi]
        if len(part) >= RATE:               # a whole second: judge its middle quarter second
            mid = part[RATE // 2 - 2000: RATE // 2 + 2000]
        elif len(part) >= 1600:             # torn last second: judge what is there
            mid = part[:4000]
        else:
            break
        if tone([float(v) for v in mid]) != sec % 40:
            break
        intact = sec + len(part) / RATE
        sec += 1
    return {"recovered_s": n / RATE, "intact_s": round(intact, 3)}

if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "gen": gen(sys.argv[2], int(sys.argv[3]))
    elif cmd == "decode": print(json.dumps(decode(sys.argv[2])))
    elif cmd == "fed":  # first `recording` event -> last event, seconds
        first = last = None
        for line in open(sys.argv[2], encoding="utf-8", errors="replace"):
            try: ev = json.loads(line)
            except ValueError: continue            # a torn last line
            if first is None and ev["event"].get("state") == "recording": first = ev["atMs"]
            last = ev["atMs"]
        print(0.0 if first is None else round((last - first) / 1000, 3))
    elif cmd == "field":  # field FILE KEY: first meeting's key from `ghi store list`
        print(json.load(open(sys.argv[2]))["meetings"][0][sys.argv[3]])
PY

py() { python3 "$work/marker.py" "$@"; }
py gen "$work/marker.wav" "$dur"

rows=() fails=0
printf '\n%-4s %-9s %-9s %-10s %-9s %-7s\n' iter kill_at_s fed_s intact_s loss_s result
for ((i = 1; i <= n; i++)); do
  d="$work/it$i"; mkdir -p "$d"
  off="$(( 3 + RANDOM % (dur - 6) )).$(printf '%03d' $((RANDOM % 1000)))"
  "$ghi" session --record-only --speed 1 --dir "$d/store" --replay "$work/marker.wav" \
    > "$d/ev.out" 2> "$d/ev.err" &
  pid=$!
  sleep "$off"
  kill -9 "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  fed="$(py fed "$d/ev.out")"
  # What the app does at launch: open the store (cuts the torn page), recover, close the meeting.
  "$ghi" jobs --dir "$d/store" > "$d/jobs.out" 2> "$d/jobs.err" || true
  "$ghi" store --dir "$d/store" list > "$d/list.json"
  gid="$(py field "$d/list.json" gid)"
  "$ghi" store --dir "$d/store" audio "$gid" --out "$d/rec.wav" > "$d/audio.json"
  res="$(py decode "$d/rec.wav")"
  intact="$(python3 -c 'import json,sys;print(json.loads(sys.argv[1])["intact_s"])' "$res")"
  loss="$(python3 -c 'import sys;print(round(float(sys.argv[1])-float(sys.argv[2]),3))' "$fed" "$intact")"
  verdict=PASS
  if python3 -c 'import sys;sys.exit(0 if float(sys.argv[1])<=float(sys.argv[2]) else 1)' "$loss" "$max_loss"; then :; else verdict=FAIL; fails=$((fails+1)); fi
  # Audio that is not the recording (tone mismatch very early) shows up as a huge loss, caught above.
  printf '%-4s %-9s %-9s %-10s %-9s %-7s\n' "$i" "$off" "$fed" "$intact" "$loss" "$verdict"
  rows+=("$loss")
  rm -rf "$d/store" "$d/store.devkey"
done
if (( n > 0 )); then
  python3 - "$max_loss" "${rows[@]}" <<'PY'
import sys
m = float(sys.argv[1]); v = [float(x) for x in sys.argv[2:]]
print(f"\nrecord kill -9: n={len(v)} max_loss={max(v):.3f}s mean={sum(v)/len(v):.3f}s limit={m}s")
PY
fi

if (( final_pass )); then
  echo
  echo "final-pass kill test"
  if ! "$ghi" version --json | grep -q '"name"'; then
    echo "  SKIP: $ghi was built without speech engines (cargo feature ghi-cli/nemo)" >&2
    failed=1; fails=$((fails+1))
  else
    clip="$speech"
    [[ -n "$clip" ]] || clip="$(find "$root/tools/eval/data" -path "*/audio/*.wav" 2>/dev/null | head -1 || true)"
    [[ -f "$clip" ]] || { echo "  no speech clip: pass --speech WAV (16 kHz mono)" >&2; exit 2; }
    d="$work/fp"; mkdir -p "$d"
    "$ghi" session --record-only --speed 0 --dir "$d/store" --replay "$clip" > "$d/rec.out" 2> "$d/rec.err"
    "$ghi" jobs --dir "$d/store" > "$d/jobs1.out" 2> "$d/jobs1.err" &
    pid=$!
    for _ in $(seq 1 600); do   # up to 60 s for the final pass to report progress
      grep -q '"kind":"final_pass".*"type":"jobProgress"\|"type":"jobProgress".*"kind":"final_pass"' "$d/jobs1.out" 2>/dev/null && break
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.1
    done
    sleep "0.$(( RANDOM % 9 + 1 ))"
    killed=0; kill -9 "$pid" 2>/dev/null && killed=1
    wait "$pid" 2>/dev/null || true
    "$ghi" jobs --dir "$d/store" > "$d/jobs2.out" 2> "$d/jobs2.err" || true
    status="$("$ghi" store --dir "$d/store" list | python3 -c 'import json,sys;print(json.load(sys.stdin)["meetings"][0]["status"])')"
    fp="$(tail -1 "$d/jobs2.out" | python3 -c 'import json,sys;j=json.load(sys.stdin)["jobs"];print(next((x["result"] for x in j if x["kind"]=="final_pass"),"missing"))')"
    echo "  killed during final pass: $killed; retried final_pass result after restart: $fp; meeting status: $status"
    if [[ "$killed" == 1 && "$fp" == "done" && "$status" == "ready" ]]; then echo "  PASS"; else echo "  FAIL"; fails=$((fails+1)); failed=1; fi
  fi
fi

if (( fails )); then failed=1; echo "RESULT: FAIL ($fails)"; exit 1; fi
echo "RESULT: PASS"
