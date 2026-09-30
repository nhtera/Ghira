# Eval kit: file formats and contracts

This is the single reference for the eval kit's data layout, the `ghi` CLI
JSON contract, and the report format. The Rust CLI (`crates/ghi-cli`), the
Python harness (`tools/eval/ghi_eval`) and the guides all follow it. Change it
here first, then bump the `schema` version of whatever changed.

Times are seconds (float) from the start of the audio. Text is UTF-8, NFC.

## 1. Dataset layout

A dataset is one directory. Customer and team datasets live on an encrypted
volume and never enter git. Public sets (`scripts/fetch_public_sets.py`) use
the same layout under `tools/eval/data/` (git-ignored).

```
<dataset>/
  manifest.yaml
  audio/<id>.wav                 # one mixed track, any sample rate, mono or stereo
  audio/<id>.mic.wav             # optional, call recordings with 2 tracks
  audio/<id>.system.wav          #   (then `audio` in the manifest names the mix)
  labels/<id>.rttm               # reference diarization (optional for ASR-only sets)
  refs/<id>.txt                  # reference transcript (optional for diarization-only sets)
  notes/<id>.yaml                # reference notes for LLM metrics (optional, §5)
  runs/<run-id>/                 # harness output; contains hypothesis text, stays here
  published-hyp/hyp/<id>.rttm    # public sets only: published system output (fetcher
  published.json                 #   --with-published-hyp) and its published scores
```

### manifest.yaml (version 1)

```yaml
version: 1
name: customer-2026q4            # free text, appears in reports
# Names of people, companies and projects that may be spoken in the meetings.
# Used only by the privacy lint (§6); never copied into a report.
names: [Linh, Minh, "Công ty ABC"]
files:
  - id: m001                     # [A-Za-z0-9_-]+, unique; never appears in a report
    audio: audio/m001.wav
    tracks: {mic: audio/m001.mic.wav, system: audio/m001.system.wav}   # optional
    rttm: labels/m001.rttm       # optional
    ref: refs/m001.txt           # optional
    notes_ref: notes/m001.yaml   # optional
    lang: vi                     # vi | en | mixed
    setting: room                # room | call | other (other: public read or broadcast speech)
    playback: speakers           # headphones | speakers | na   (na for room)
    speakers: 4                  # number of distinct speakers in the recording
    duration_s: 1834.2           # optional; read from the audio when missing
    persons: {spk1: P01, spk2: P02}   # optional: RTTM speaker label -> pseudonymous
                                      # person id, for cross-meeting speaker-ID trials
```

Slices used in reports: `lang` × `setting` × speaker bucket, where the bucket
is `1-2`, `3-5` or `6+` from `speakers`.

Target mix for a customer set (recording guide): ≥10 h, ~40% `vi`, ~30% `en`,
~30% `mixed`; both settings; both playback modes for calls; 3–5 files with 6–8
speakers. `ghi-eval validate` prints hours per slice against these targets.

### labels/<id>.rttm

Standard NIST RTTM, one `SPEAKER` line per turn, overlaps allowed:

```
SPEAKER m001 1 12.340 3.210 <NA> <NA> spk1 <NA> <NA>
```

The file-id column must equal the manifest `id`.

### refs/<id>.txt

Verbatim transcript as spoken, one utterance per line, in time order. Scoring
joins the lines, so line breaks don't matter. Conventions (labelling guide):
English words in Vietnamese speech are written in English as spoken;
unintelligible speech is `[unk]`; `[unk]` tokens and anything inside
`[...]` are removed before scoring.

## 2. `ghi` CLI contract (schema version 1)

`ghi` is the headless CLI in `crates/ghi-cli`. The harness is its only
consumer. Every command prints **one JSON document on stdout**, except
`--stream`, which prints NDJSON (one event per line). Logs go to stderr.

`ghi` with no arguments prints help and exits `2`; `ghi --version` prints
`ghi <version>`.

Engines (phase 3): `transcribe`, `diarize` and `bench` run NeMo-Speech.cpp
when `ghi` is built with `--features nemo` (after `tools/scripts/build-nemo.sh`);
otherwise they exit `3` with `engine_unavailable`. Models are the pinned files
in `crates/ghi-models/registry.toml`, read from `$GHI_MODELS_DIR` (default
`./models`, relative to the working directory, so the harness needs an
absolute path; fetch them with `tools/scripts/fetch-models.sh`). Extra flags,
outside the harness contract: `--asr-model PATH`, `--diar-model PATH`, `--cpu`
(or `GHI_DEVICE=cpu`), `transcribe --offline` (one offline decode of the whole
file) and `bench --topology single|call`.

Exit codes: `0` ok · `1` runtime failure · `2` usage error · `3` not
implemented / engine or capture unavailable. On exit codes 1 and 3 stdout is empty (except
`--stream`, which may already have printed events before a mid-stream failure;
consumers discard them) and stderr's **last line** is an error document:

```json
{"schema":"ghi.error/1","code":"not_implemented","message":"transcribe: no speech engine yet (phase 3)"}
```

`code` is one of `not_implemented`, `engine_unavailable`, `bad_input`, `internal`,
`capture_unavailable` (capture commands below: unsupported OS, or permission denied).

Every result has `perf`: `{"wall_s": float, "rtf": float|null, "peak_rss_mb": float|null}`.
`rtf` = `wall_s / duration_s`. The harness also measures wall time and peak RSS
itself (§4) and reports its own numbers; `perf` is informational.

### `ghi version --json`

```json
{"schema":"ghi.version/1","ghi":"0.1.0","core":"0.1.0","engines":[]}
```

`engines` lists `{"name": str, "version": str}` once engines exist.

### `ghi transcribe <audio> [--lang auto|vi|en] [--pass live|final] --json`

Defaults: `--lang auto`, `--pass final`.

```json
{
  "schema": "ghi.transcript/1",
  "audio": "m001.wav",
  "duration_s": 1834.2,
  "pass": "final",
  "lang": "auto",
  "engine": {"name": "nemotron-3.5-asr-streaming", "version": "1"},
  "segments": [
    {"id": 0, "start": 1.23, "end": 3.40, "text": "Mình chốt scope cho beta nhé.",
     "lang": "vi", "speaker": null, "words": null}
  ],
  "perf": {"wall_s": 95.1, "rtf": 0.052, "peak_rss_mb": 2210.0}
}
```

`segments[].id` is unique within the document (citations refer to it).
`lang` is `vi`, `en` or `null`. `speaker` is a diarization label or `null`.
`words` is `null` or a list of `{"start", "end", "text"}`.

### `ghi transcribe <audio> --stream [--realtime] [--lang ...]`

NDJSON on stdout, one event per line:

```json
{"schema":"ghi.event/1","type":"partial","seq":0,"wall_s":1.92,"audio_start":0.0,"audio_end":1.60,"text":"Mình chốt","lang":"vi","speaker":null,"words":null}
{"schema":"ghi.event/1","type":"final","seq":1,"wall_s":3.95,"audio_start":0.0,"audio_end":1.60,"text":"Mình chốt scope","lang":"vi","speaker":"S1","words":[{"start":0.0,"end":0.4,"text":"Mình","shown_s":1.92},{"start":0.5,"end":0.9,"text":"chốt","shown_s":1.92},{"start":1.0,"end":1.6,"text":"scope","shown_s":3.95}]}
{"schema":"ghi.event/1","type":"end","seq":2,"wall_s":1834.9,"audio_start":0.0,"audio_end":1834.2,"text":"","lang":null,"speaker":null,"words":null}
```

- `type`: `partial` (may change), `final` (committed caption), `end` (last line).
- `wall_s`: seconds since the CLI started feeding audio.
- `audio_start`/`audio_end`: the words' span; partials have no word times, so
  for them `audio_end` is the audio consumed so far.
- `words` (final events only, else `null`): the committed words with their
  times, and `shown_s`, the `wall_s` of the first event (partial or this
  final) that displayed the word.
- `--realtime` requires `--stream`. With it the CLI feeds audio at 1× speed,
  as the live app does, so **caption lag of a word = `shown_s − end`**, and
  **commit lag of a final = `wall_s − audio_end`**. Without `--realtime` audio
  is fed as fast as possible and the harness does not compute lag.
- Older producers may omit `words`; consumers treat a missing field as `null`.

### `ghi diarize <audio> [--pass live|final] [--max-speakers N] --json`

`--max-speakers` is a hint and is currently ignored: the model tracks up to its
own capacity (Nemotron 3 Diarization: 8); `ghi` warns when N exceeds it.

```json
{
  "schema": "ghi.diarization/1",
  "audio": "m001.wav",
  "duration_s": 1834.2,
  "pass": "final",
  "engine": {"name": "nemotron-3-diarization", "version": "1"},
  "turns": [{"start": 12.34, "end": 15.55, "speaker": "S1"}],
  "perf": {"wall_s": 40.2, "rtf": 0.022, "peak_rss_mb": 1500.0}
}
```

### `ghi notes <transcript.json> [--lang auto|vi|en] --json`

Input is a `ghi.transcript/1` document. `--lang` is the language of the notes
(`auto` = the meeting's). Other options: `--template ID` (general,
one_on_one, standup, sales, interview, client, lecture) or `--template-file
T.toml`, `--user-notes FILE` (one typed note per line, optional `[mm:ss] `
prefix; adds `enhanced`), `--model ID --n-ctx N` (local model, default
`qwen3-4b`, 32768). The local model runs offline in the `ghi-llm-worker`
process; without the model or the worker `ghi` exits `3`
(`engine_unavailable`). Output (the schema may grow, but fields below keep
their meaning):

```json
{
  "schema": "ghi.notes/1",
  "engine": {"name": "qwen3-8b-q4", "version": "1"},
  "summary": [{"text": "Scope for the beta was agreed.", "citations": [0]}],
  "decisions": [{"text": "Beta ships without calendar sync.", "citations": [0, 4]}],
  "action_items": [
    {"text": "Send the beta scope doc", "owner": "Linh", "due": null, "citations": [7]}
  ],
  "perf": {"wall_s": 61.0, "rtf": null, "peak_rss_mb": 5400.0}
}
```

`citations` are `segments[].id` values of the input transcript; every item
has at least one. `summary` is the TL;DR (≤5). `owner` is the transcript's
`speaker` value of someone who speaks in the cited segments, or `null`
(never a guess). Text is plain (no Markdown, links or HTML). Also present:
`template`, `lang`, `open_questions`, `key_quotes` (`speaker`), `topics`
(`title`, `start`, `end`), `sections` (template-specific: `id`, `title`,
`items`), `strategy` (`single` or `map_reduce` with `parts`) and
`diagnostics` (requests, retries, tokens, items dropped for invalid
citations, weak anchors, unassigned owners).

### `ghi ask <transcript.json> <question> [--lang auto|vi|en] --json`

`ghi.ask/1`: `answer` is `{"kind": "answered", "text", "citations"}` or
`{"kind": "not_discussed", "searched": [folded terms]}`; plus `engine`,
`diagnostics`, `perf`. Not used by the harness.

### `ghi bench <audio> [--pass live|final] --json`

Default: `--pass live`.

```json
{
  "schema": "ghi.bench/1",
  "audio": "m001.wav",
  "duration_s": 1834.2,
  "pass": "live",
  "stages": [{"name": "asr", "wall_s": 80.0}, {"name": "diarization", "wall_s": 30.1}],
  "perf": {"wall_s": 110.1, "rtf": 0.06, "peak_rss_mb": 3000.0}
}
```

### Capture commands (phase 4; not used by the harness)

Tooling for test recordings, soak runs and crash tests. Live capture is
macOS 14.2+ only; elsewhere these exit `3` with `capture_unavailable`.
The `ghi` binary embeds an Info.plist with the Microphone and System Audio
Recording usage strings; after building, re-sign it (`codesign -s - -f
target/<profile>/ghi`) so macOS binds the plist and can show the prompts.

- `ghi record --out DIR [--mode call|room] [--id ID] [--duration S]
  [--format wav|opus] [--replay MIC[,SYSTEM]] [--pid PID]... [--no-aec]
  [--silent-warn S]` records until `--duration`, Ctrl-C/SIGTERM or the end of
  a replay. `call` = mic + system audio, `room` = mic only. `--replay` plays
  WAV files through the capture pipeline at 1x instead of capturing (CI and
  crash tests). `--pid` taps only those processes. `--id` (letters, digits,
  `-`, `_`) and `--duration` (> 0) are checked as usage errors (exit `2`); an
  existing recording with the same id is never overwritten (`bad_input`).
  - `wav` writes the dataset layout of §1: `<id>.mic.wav`, `<id>.system.wav`
    and the mix `<id>.wav` (16 kHz, 16-bit). Tracks are **raw**; echo
    cancellation only feeds ASR. Not crash-safe: the WAV sizes are written
    when the recording stops. Use `opus` for crash and soak tests.
  - `opus` writes `<id>.mic.opus` / `<id>.system.opus` (Ogg Opus, ~1 s pages,
    a write barrier per page and a full sync every 2 pages: a process crash
    loses the unfinished page (≤1 s), a power loss up to ~3 s).
  - Always: `<id>.session.json` (written first: start time, mode, tracks) and
    `<id>.markers.jsonl` (pause, gap, AEC on/off...).
  - Prints `ghi.record/1`: `duration_s`, `tracks[{track, file, duration_s,
    rms_dbfs, peak, overrun_samples}]`, `mix`, `route`, `aec`, `erle_db`,
    `markers[{t_s, kind, detail?}]`, `events[{t_s, kind, detail?}]`, `stopped`
    (`duration|signal|source_ended|disk_full|writer_error|track_lost|no_audio`),
    `perf`. Losing the system track leaves a mic-only recording; `no_audio`
    means the device delivered nothing for 10 s. Sleep works like a pause:
    the timeline stops and the `wake` marker notes the wall-clock sleep.
    Events are also logged to stderr as they happen (e.g. `silent_system_track`
    after `--silent-warn` seconds, default 5: System Audio Recording is
    probably denied).
- `--format store` records a meeting into the encrypted Ghira store at `--out`
  (a data directory, see "Store commands"); `--title` names it and the
  document's `id` is the meeting gid. Track files are the store's encrypted
  bundles (`bundles/<gid>/<track>.ghb`).
- `ghi recover DIR` decodes every `*.opus` in DIR to `*.recovered.wav`,
  tolerating a torn last page; prints `ghi.recover/1`: `files[{file, wav,
  duration_s, complete, bad_pages, truncated, error?}]`; a file that cannot be
  decoded gets `error` and the others are still recovered.
- `ghi detect [--watch S]` prints `ghi.detect/1`: `processes[{pid, bundle_id,
  input, output, app}]` and the auto-detect `prompt` (`{app, title, pids}` or
  null). `--watch` polls every second and prints one document per prompt.
  Known gap: Safari plays and records call audio in `com.apple.WebKit.GPU`,
  which is not mapped to Safari yet, so Safari calls are not detected.

### Store commands (phase 5; not used by the harness)

`ghi store --dir DIR <action>` opens (or creates) an encrypted store: a
SQLCipher database, per-meeting keys and encrypted audio bundles. Debug builds
keep the key ring in a file next to the directory (`DIR.devkey`) (`GHI_KEYSTORE=keychain` uses the
Keychain); release builds use the OS key store (macOS Keychain item service
`com.nhtera.ghira.cli`, Windows DPAPI). Passwords are read from stdin, never
from arguments. Every action prints one JSON document:

- `list` → `ghi.store-list/1`: `meetings[{gid, title, started_at, duration_s,
  mode, status, tracks[{kind, pages}]}]`.
- `search QUERY [--limit N]` → `ghi.store-search/1`: `hits[{kind, meeting_gid,
  meeting_title, item_gid, t0_s, t1_s, snippet, highlights[[start, end]],
  exact, score}]`, `took_ms`. Accent-insensitive (`dong` finds `đồng`);
  highlights are char offsets into `snippet`; `exact` marks accent-exact hits.
- `audio GID [--track mic|system] --out X.wav` → `ghi.store-audio/1`: decrypts
  a track to 16 kHz WAV (`pages`, `complete`, `duration_s`).
- `add-transcript FILE [--meeting GID]` → `ghi.store-transcript/1`: stores a
  `ghi.transcript/1` as a new transcript version (a new meeting by default).
- `delete GID` → `ghi.store-deleted/1`: crypto-shred (key destroyed first,
  then files, rows and index entries).
- `export --out FILE` → `ghi.store-export/1`: everything in one archive
  encrypted with the password (Argon2id). The archive includes the master
  key, so its security is the password's.
- `import ARCHIVE` → `ghi.store-import/1`: restores into an empty `--dir` on a
  device without a key; a wrong password fails with `bad_input`.
- Empty passwords are refused (`bad_input`): the archive is only as strong as its password.

Golden examples of each document live in `tools/eval/tests/fixtures/cli/`.
`crates/ghi-cli` serializes its types against them in its unit tests, and the
harness validates them against `ghi_eval/schemas/*.schema.json`, so a change on
either side fails a test.

## 3. Systems under test (adapters)

`ghi-eval run --system <spec>`:

| spec | What runs |
|---|---|
| `ghi` or `ghi:<path-to-binary>` | The `ghi` CLI (§2). Default binary: `ghi` on `PATH`. |
| `nemo-ref[:<config.yaml>]` | NeMo Python models (reference baselines for phase 3, drafts for labelling). Optional extra: `uv sync --extra nemo`. |
| `whisper-ref[:<model>]` | faster-whisper, ASR only (labelling drafts). Optional extra: `uv sync --extra whisper`. |
| `files:<dir>` | Precomputed hypotheses in `<dir>/hyp/` (or `<dir>` itself if it has no `hyp/`), named as in a run directory (§4). Used by CI and to score outputs made elsewhere. |

## 4. Run directory

`<dataset>/runs/<run-id>/` (default run id `<system>-<YYYYMMDD-HHMMSS>`).
It holds hypothesis text, so it stays on the encrypted volume.

```
run.yaml                       # system spec + version, tasks, pass, start/end time
hyp/<id>.transcript.json       # ghi.transcript/1
hyp/<id>.rttm                  # from ghi.diarization/1
hyp/<id>.events.ndjson         # ghi.event/1 stream (only with --realtime)
hyp/<id>.notes.json            # ghi.notes/1
hyp/<id>.notes-gold.json       # ghi.notes/1 made from the reference transcript
hyp/<id>.perf.json             # harness-measured {"wall_s","audio_s","rtf","peak_rss_mb"} per task
speaker_scores.tsv             # optional: enroll_file enroll_spk test_file test_spk score
judgements.csv                 # LLM note judgements (§5), filled in by a person
scores.json                    # per-file metrics; file ids, no text
report-<system>-<pass>-<YYYYMMDD>.json / .md   # aggregates only (§6); the only files that leave
```

## 5. LLM note references and judgements

`notes/<id>.yaml`, written by the customer for 5–10 meetings:

```yaml
action_items:
  - text: Send the beta scope doc
    owner: Linh                # or null
decisions:
  - text: Beta ships without calendar sync
```

`ghi-eval judge --run <dir>` writes `judgements.csv`, one row per system
action item plus one row per unmatched reference item, pre-filled with a
suggested match (token F1 ≥ 0.5). A person checks every row:

| column | values |
|---|---|
| `file`, `source` | file id; `pipeline` or `gold` (which transcript the notes came from) |
| `sys_idx`, `sys_text`, `sys_owner` | system item (blank on reference-only rows) |
| `sys_citations`, `cited_text` | cited segment ids (joined with `;`) and their text, to judge `citation_ok` |
| `ref_idx`, `ref_text`, `ref_owner` | matched reference item, blank if none |
| `owner_ok` | `y` / `n` / blank (only when matched) |
| `citation_ok` | `y` / `n` / `na`: do the cited segments support the item? |
| `hallucinated` | `y` / `n`: is the item unsupported by the meeting? |

Metrics: precision = matched system items / system items; recall = matched
reference items / reference items; owner accuracy = `owner_ok=y` / matched;
citation validity = `citation_ok=y` / (`y`+`n`); hallucinations = count of
`y`; schema validity = notes outputs valid against `ghi.notes/1` / outputs.

## 6. Report (schema `ghi.eval-report/1`)

`report-<system>-<pass>-<YYYYMMDD>.json` (and a `.md` rendering of the same numbers):

```json
{
  "schema": "ghi.eval-report/1",
  "generated": "2026-10-20T10:00:00Z",
  "kit_version": "0.1.0",
  "system": {"spec": "ghi", "version": "0.1.0"},
  "dataset": {"name": "customer-2026q4", "files": 42, "hours": 10.4},
  "run": {"pass": "final", "realtime": false, "lang": "auto", "tasks": ["asr", "diar", "notes"], "errors": 0},
  "der_collar": 0.25,
  "slices": [
    {"lang": "vi", "setting": "room", "speakers": "3-5", "files": 6, "hours": 1.8,
     "der": 0.182, "jer": 0.25, "spk_count_err": 0.5,
     "wer": null, "syl_wer": 0.141, "mer": null,
     "partial_lag_p50": 0.9, "partial_lag_p95": 1.6,
     "final_lag_p50": 1.8, "final_lag_p95": 2.7,
     "caption_lag_p50": 0.9, "caption_lag_p95": 1.7,
     "rtf": 0.05, "peak_rss_mb": 2300.0,
     "asr_files": 6, "diar_files": 6, "lag_files": 0, "notes_files": 2}
  ],
  "overall": {"...": "same metric keys, over all files"},
  "speaker_id": {"eer": 0.08, "trials": 120},
  "llm": [{"source": "pipeline", "files": 8, "schema_valid": 1.0, "precision": 0.8,
           "recall": 0.7, "owner_acc": 0.9, "citation_valid": 0.85, "hallucinations": 2}],
  "gates": [{"id": "der_vn_room", "metric": "der", "slice": {"lang": "vi", "setting": "room"},
             "max": 0.20, "floor": 0.25, "value": 0.182, "status": "pass"}],
  "privacy_lint": {"passed": true, "ngram": 3, "names_checked": 12}
}
```

- Metrics are pooled, not averaged per file: DER = total error time / total
  reference speech; WER = total errors / total reference tokens.
- `wer` is computed on `en` files, `syl_wer` on `vi` files, `mer` on `mixed`
  files. All three use the same tokens: NFC, lowercase, punctuation removed,
  split on whitespace (a Vietnamese syllable is one token). Digits are read
  out as spoken words on both sides before tokenizing (language from the manifest, `mixed`
  uses the Vietnamese rules; `ghi_eval/numwords.py`), so "53 tuổi" equals "năm mươi ba tuổi"
  and "1967" equals "nineteen sixty seven". `mer` here is the
  code-switch **mixed error rate**, not jiwer's "match error rate".
- DER: collar ±0.25 s (`der_collar`; `--collar` on `run`/`score`/`report`
  changes it, `0` = none), overlap scored. JER uses no collar. `overall` also
  carries `files` and `hours`. Missing metrics are `null`.
- `asr_files`, `diar_files`, `lag_files`, `notes_files`: files with both a
  reference and a valid hypothesis for that metric. `run.errors` counts the
  errors recorded in `run.yaml`. `system.spec` and `system.version` never hold
  paths (basenames only; `files:<dir>` is reported as `files`).
- A report never contains file ids, file names, paths, transcript text,
  reference text or names. Before writing, the privacy lint fails the report
  (JSON and `.md`) if any 3-token sequence from a reference transcript or
  reference note appears anywhere in it; if a free-text field (`system.*`,
  `dataset.name`, gate ids) contains a manifest `names` entry, a note owner or
  a file id; or if any string contains `/`, `\` or a drive prefix such as `C:`
  (fixed vocabulary like `n/a` is exempt). All matching is after the same
  normalization, with diacritics folded for names.

### Gates

`ghi_eval/gates.yaml` encodes doc 05 §9 (a copy can be passed with `--gates`):

| id | metric | slice | max | floor |
|---|---|---|---|---|
| `der_overall` | `der` | all | 0.15 | |
| `der_call` | `der` | `setting: call` | 0.15 | |
| `der_vn_room` | `der` | `lang: vi, setting: room` | 0.20 | 0.25 |
| `syl_wer_vn_call` | `syl_wer` | `lang: vi, setting: call` | 0.15 | |
| `syl_wer_vn_room` | `syl_wer` | `lang: vi, setting: room` | 0.15 | 0.25 |
| `lag_en` | `caption_lag_p95` | `lang: en` | 2.0 | |
| `lag_vn` | `caption_lag_p95` | `lang: vi` | 3.0 | |

Note: since 2026-09-30 `lag_en` and `lag_vn` measure `caption_lag_p95`. Gate values in
earlier reports used `final_lag_p95` (commit lag) and are not comparable.

Status: `pass` if value ≤ max; `best_effort` if a floor exists and value ≤
floor; `fail` otherwise; `n/a` without data; `incomplete` if some file in the
slice has a reference but no valid hypothesis for the metric (fix the errors
and re-run before trusting the gate). Word-error gates are `n/a` on
`--pass live` runs; lag gates are `n/a` unless the run used `--realtime`.
