# SPDX-License-Identifier: Apache-2.0
"""Run a system on every file of a dataset and write the run directory (formats.md §4)."""

from __future__ import annotations

import json
import sys
import tempfile
from collections.abc import Callable
from datetime import UTC, datetime
from pathlib import Path

import yaml

from . import __version__
from .adapters import Adapter, make_adapter
from .errors import AdapterError, HarnessError, InvalidOutput, NotSupported
from .llm import gold_transcript
from .manifest import ID_RE, FileEntry, load_manifest
from .metrics import DER_COLLAR
from .perf import rtf
from .rttm import Turn, write_rttm

TASKS = ("asr", "diar", "stream", "notes")
NOTES_SOURCES = ("pipeline", "gold")


def write_json(path: Path, doc: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(doc, indent=2, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n"
    )


def utc_now() -> datetime:
    return datetime.now(UTC).replace(microsecond=0)


def parse_list(value: str, allowed: tuple[str, ...], what: str) -> list[str]:
    items = [v.strip() for v in value.split(",") if v.strip()]
    bad = [v for v in items if v not in allowed]
    if bad or not items:
        raise HarnessError(
            f"bad {what}: {', '.join(bad) or value!r} (choose from {', '.join(allowed)})"
        )
    return items


def load_run(run_dir: Path) -> dict:
    path = Path(run_dir) / "run.yaml"
    if not path.is_file():
        raise HarnessError(f"{path}: not a run directory (run.yaml missing)")
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def _perf_entry(wall_s: float, audio_s: float | None, peak_rss_mb: float | None) -> dict:
    ratio = rtf(wall_s, audio_s)
    return {
        "wall_s": round(wall_s, 3),
        "audio_s": round(audio_s, 3) if audio_s else None,
        "rtf": round(ratio, 4) if ratio is not None else None,
        "peak_rss_mb": peak_rss_mb,
    }


def execute(
    dataset: Path,
    spec: str,
    *,
    tasks: list[str],
    pass_: str = "final",
    realtime: bool = False,
    lang: str = "auto",
    notes_input: list[str] | None = None,
    run_id: str | None = None,
    out: Path | None = None,
    collar: float = DER_COLLAR,
    timeout: float | None = None,
    log: Callable[[str], None] | None = None,
) -> Path:
    """Run `spec` over the dataset. Returns the run directory."""
    log = log or (lambda m: print(m, file=sys.stderr))
    notes_input = notes_input or ["pipeline"]
    dataset = Path(dataset).resolve()
    manifest = load_manifest(dataset)
    adapter = make_adapter(spec)
    started = utc_now()
    run_id = run_id or f"{adapter.name}-{started:%Y%m%d-%H%M%S}"
    if not ID_RE.match(run_id):
        raise HarnessError("--run-id must match [A-Za-z0-9_-]+")
    run_dir = (Path(out) if out else dataset / "runs") / run_id
    hyp = run_dir / "hyp"
    hyp.mkdir(parents=True, exist_ok=True)

    if "stream" in tasks and not realtime:
        log("note: the stream task needs --realtime to measure lag; skipping it")
        tasks = [t for t in tasks if t != "stream"]

    if "notes" in tasks and "pipeline" in notes_input and "asr" not in tasks:
        log("note: pipeline notes are made from this run's transcripts; add asr to --tasks")
    unsupported: dict[str, str] = {}
    errors: list[dict] = []
    total = len(manifest.files)
    for n, entry in enumerate(manifest.files, 1):
        log(f"[{n}/{total}] {entry.id}")
        perf: dict[str, dict] = {}
        _run_file(
            adapter,
            entry,
            manifest.duration(entry),
            timeout,
            tasks,
            pass_,
            realtime,
            lang,
            notes_input,
            hyp,
            perf,
            unsupported,
            errors,
            log,
        )
        if adapter.measures_perf:
            write_json(hyp / f"{entry.id}.perf.json", perf)

    run_doc = {
        "schema": "ghi.eval-run/1",
        "kit_version": __version__,
        "system": {"spec": spec, "name": adapter.name, "version": _safe_version(adapter)},
        "dataset": str(dataset),
        "dataset_name": manifest.name,
        "tasks": tasks,
        "pass": pass_,
        "lang": lang,
        "realtime": realtime,
        "collar": collar,
        "timeout": timeout,
        "notes_input": notes_input,
        "started": started.isoformat().replace("+00:00", "Z"),
        "ended": utc_now().isoformat().replace("+00:00", "Z"),
        "unsupported": unsupported,
        "errors": errors,
    }
    (run_dir / "run.yaml").write_text(
        yaml.safe_dump(run_doc, sort_keys=False, allow_unicode=True), encoding="utf-8", newline="\n"
    )
    for task, msg in unsupported.items():
        log(f"system does not support {task} yet: {msg}")
    for e in errors:
        log(f"error [{e['file']}/{e['task']}]: {e['error']}")
    return run_dir


def _safe_version(adapter: Adapter) -> str:
    try:
        return adapter.version()
    except HarnessError:
        return "unknown"


def _run_file(
    adapter,
    entry: FileEntry,
    duration: float | None,
    timeout: float | None,
    tasks,
    pass_,
    realtime,
    lang,
    notes_input,
    hyp: Path,
    perf: dict,
    unsupported: dict,
    errors: list,
    log,
) -> None:
    fid = entry.id
    if not entry.audio.is_file() and adapter.name != "files":
        errors.append({"file": fid, "task": "all", "error": "audio file missing"})
        return

    def attempt(task: str, fn):
        """Call an adapter method; record unsupported/failed instead of raising."""
        if task in unsupported:
            return None
        try:
            return fn()
        except NotSupported as exc:
            if adapter.measures_perf:
                unsupported[task] = exc.reason or "not implemented"
            # files: a file without precomputed output is simply not scored
        except InvalidOutput as exc:
            errors.append({"file": fid, "task": task, "error": str(exc)})
            return exc
        except AdapterError as exc:
            errors.append({"file": fid, "task": task, "error": str(exc)})
        return None

    audio_s = duration
    # per-task limit; a hung engine must not stall the whole run
    adapter.timeout = timeout if timeout is not None else max(600.0, 3.0 * (duration or 0.0))
    transcript_path = hyp / f"{fid}.transcript.json"
    if "asr" in tasks:
        res = attempt("asr", lambda: adapter.transcribe(fid, entry.audio, lang=lang, pass_=pass_))
        if res is not None and not isinstance(res, InvalidOutput):
            write_json(transcript_path, res.value)
            audio_s = audio_s or res.value.get("duration_s")
            perf["asr"] = _perf_entry(res.wall_s, audio_s, res.peak_rss_mb)
    if "diar" in tasks:
        res = attempt("diar", lambda: adapter.diarize(fid, entry.audio, pass_=pass_))
        if res is not None and not isinstance(res, InvalidOutput):
            turns = [Turn(t["start"], t["end"], t["speaker"]) for t in res.value["turns"]]
            write_rttm(hyp / f"{fid}.rttm", fid, turns)
            audio_s = audio_s or res.value.get("duration_s")
            perf["diar"] = _perf_entry(res.wall_s, audio_s, res.peak_rss_mb)
    if "stream" in tasks:
        res = attempt(
            "stream",
            lambda: adapter.transcribe_stream(fid, entry.audio, lang=lang, realtime=realtime),
        )
        if res is not None and not isinstance(res, InvalidOutput):
            lines = "".join(json.dumps(ev, ensure_ascii=False) + "\n" for ev in res.value)
            (hyp / f"{fid}.events.ndjson").write_text(lines, encoding="utf-8", newline="\n")
            perf["stream"] = _perf_entry(res.wall_s, audio_s, res.peak_rss_mb)
    if "notes" in tasks:
        for source in notes_input:
            _notes(adapter, entry, source, lang, hyp, transcript_path, perf, attempt, errors)


def _notes(adapter, entry, source, lang, hyp, transcript_path, perf, attempt, errors) -> None:
    fid = entry.id
    with tempfile.TemporaryDirectory() as tmp:
        if source == "gold":
            if entry.ref is None or not entry.ref.is_file():
                return  # nothing to build a gold transcript from
            duration = entry.duration_s or 0.0
            src = Path(tmp) / f"{fid}.gold-transcript.json"
            write_json(src, gold_transcript(entry, duration))
            out_path = hyp / f"{fid}.notes-gold.json"
        else:
            src = transcript_path
            out_path = hyp / f"{fid}.notes.json"
            if not src.is_file() and adapter.name != "files":
                return  # asr failed or was not run; that is reported for the asr task
        n_errors = len(errors)
        res = attempt("notes", lambda: adapter.notes(fid, src, lang=lang, source=source))
        for e in errors[n_errors:]:
            e["source"] = source
        if isinstance(res, InvalidOutput):
            if res.raw is not None:  # keep it so scoring counts it as schema-invalid
                write_json(out_path, res.raw)
        elif res is not None:
            write_json(out_path, res.value)
            perf[f"notes-{source}" if source == "gold" else "notes"] = _perf_entry(
                res.wall_s, None, res.peak_rss_mb
            )
