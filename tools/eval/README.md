# ghi-eval

Python harness that scores a system (the `ghi` CLI, NeMo or Whisper reference
models, or precomputed output) on a dataset: DER/JER, WER/syllable-WER/MER,
caption lag, RTF, peak RSS, speaker-ID EER and meeting-note quality. Formats and
the CLI contract are in `docs/formats.md`.

Reports contain aggregates only. Run directories hold hypothesis text and stay
under the dataset (`<dataset>/runs/`). A privacy lint runs before every report is
written and refuses a report that contains reference text, names or file ids.
The harness itself makes no network calls, and neither do the `ghi` and `files:` systems.
The reference systems `nemo-ref` and `whisper-ref` download their models from Hugging Face
on first use (telemetry is switched off via `HF_HUB_DISABLE_TELEMETRY=1` and `DO_NOT_TRACK=1`).
After the first download set `HF_HUB_OFFLINE=1` to stay offline.

## Setup

```sh
cd tools/eval
uv sync --locked                 # Python 3.11; add --extra nemo / --extra whisper for reference systems
uv run ghi-eval --help
```

## Commands

```sh
uv run ghi-eval validate --dataset D
uv run ghi-eval run --dataset D --system ghi[:path] [--tasks asr,diar,stream,notes] \
    [--pass live|final] [--realtime] [--lang auto|vi|en] [--notes-input pipeline,gold] \
    [--run-id ID] [--out DIR] [--gates FILE]
uv run ghi-eval score  --run R
uv run ghi-eval report --run R [--gates FILE]
uv run ghi-eval judge  --run R            # judgements.csv, then a person fills it in, then `report`
uv run ghi-eval lint-report --report FILE --dataset D
uv run ghi-eval trials --dataset D        # speaker-ID trials from manifest `persons`
uv run ghi-eval convert eaf FILE.eaf --id ID --dataset D
uv run ghi-eval convert audacity LABELS.txt --id ID --dataset D
uv run ghi-eval convert rttm-audacity FILE.rttm [--out FILE]
uv run ghi-eval convert draft-eaf --run R [--dataset D] [--out DIR]
```

Systems: `ghi`, `ghi:<path>`, `files:<dir>` (precomputed, `<dir>/hyp/` laid out as a
run directory), `nemo-ref[:config.yaml]`, `whisper-ref[:model]`.

## Behaviour worth knowing

- `run` executes every requested task on every file, even when a reference is
  missing (so drafts for labelling can be made); scoring skips what has no reference.
- A `ghi` exit code 3 (`not_implemented`) skips that task for the whole run and prints
  "system does not support <task> yet". Other failures are listed in `run.yaml`, and
  `run` exits 1 after still writing the report.
- Every task has a time limit (`--timeout`, default max(600 s, 3 x audio)); on timeout the
  process tree is killed and an error is recorded. In-process systems (`nemo-ref`,
  `whisper-ref`) ignore it.
- Slices and `overall` carry `asr_files`, `diar_files`, `lag_files`, `notes_files`: files with
  both a reference and a valid hypothesis. A gate over files that have a reference but no
  valid hypothesis is `incomplete`. Word-error gates are `n/a` on `--pass live`, lag gates
  `n/a` without `--realtime`. `run.errors` counts recorded failures.
- Reports are named `report-<system>-<pass>-<YYYYMMDD>.{json,md}`.
- `stream` and lag need `--realtime`; the stream task is dropped without it.
- DER uses pyannote `collar=0.5`, the total width for +-0.25 s, with overlap scored.
  JER uses no collar (DIHARD convention). Both are pooled over files, as are all rates.
- RTF = wall time of the `asr` + `diar` tasks / audio duration, pooled. Peak RSS is the
  max over files of the process-tree peak (polled every 50 ms). `files:` reports neither.
- Lag percentiles use linear interpolation over `wall_s - audio_end` of all partial
  (or final) events pooled in a slice.
- Judgement sheets are UTF-8 with BOM (Excel-friendly). Judgement metrics stay null
  until every system row has `citation_ok` and `hallucinated` filled in. Owner accuracy
  counts matched rows whose `owner_ok` is `y` or `n`.
- Gold notes input is a `ghi.transcript/1` made from the reference txt, one segment per
  line, with `start = end = 0.0`.
- `convert eaf`: speaker = tier `PARTICIPANT` if set, else `TIER_ID`.

## Licenses

The default install is license-checked by `tests/test_licenses.py`; the optional `nemo` and
`whisper` extras are not (opt-in dev tools, never shipped).

## Tests

```sh
uv run ruff check && uv run ruff format --check && uv run pytest -q
# CI smoke (no audio needed; `duration_s` is in the manifest):
uv run ghi-eval run --dataset tests/fixtures/tiny --system files:tests/fixtures/tiny-hyp \
    --run-id ci --tasks asr,diar,notes --out "$(mktemp -d)"
```

`GHI_BIN=/path/to/ghi uv run pytest` also checks the real binary. Never commit audio
(`*.wav` is git-ignored) or a run directory.
