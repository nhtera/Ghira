# SPDX-License-Identifier: Apache-2.0
"""Per-file scoring of a run directory into scores.json (ids and numbers only, no text)."""

from __future__ import annotations

import json
from pathlib import Path

from . import __version__
from .contract import validation_errors
from .errors import HarnessError
from .manifest import FileEntry, Manifest, load_manifest
from .metrics import DER_COLLAR, asr_metric_for, diarization_components, token_errors
from .rttm import read_rttm
from .run import load_run, write_json
from .speakerid import speaker_id_summary
from .textnorm import normalize


def _load_json(path: Path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None


def _score_asr(entry: FileEntry, hyp: Path) -> dict | None:
    path = hyp / f"{entry.id}.transcript.json"
    if entry.ref is None or not entry.ref.is_file() or not path.is_file():
        return None
    doc = _load_json(path)
    if doc is None or validation_errors(doc, "transcript"):
        return None
    hyp_text = " ".join(s["text"] for s in doc["segments"])
    te = token_errors(entry.ref.read_text(encoding="utf-8"), hyp_text)
    if te is None:
        return None
    return {"metric": asr_metric_for(entry.lang), "errors": te.errors, "ref_tokens": te.ref_tokens}


def _score_diar(entry: FileEntry, hyp: Path, duration: float | None, collar: float) -> dict | None:
    path = hyp / f"{entry.id}.rttm"
    if entry.rttm is None or not entry.rttm.is_file() or not path.is_file():
        return None
    ref = read_rttm(entry.rttm)
    if not ref:
        return None
    comp = diarization_components(ref, read_rttm(path), duration, collar)
    comp["spk_count_err"] = abs(comp["hyp_speakers"] - comp["ref_speakers"])
    return comp


def _score_lag(hyp: Path, fid: str) -> dict | None:
    path = hyp / f"{fid}.events.ndjson"
    if not path.is_file():
        return None
    lags: dict[str, list[float]] = {"partial": [], "final": []}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        try:
            ev = json.loads(line)
        except json.JSONDecodeError as exc:
            raise HarnessError(f"{path.name}: line is not JSON") from exc
        if ev.get("type") in lags:
            lags[ev["type"]].append(round(ev["wall_s"] - ev["audio_end"], 4))
    return lags


def _perf(hyp: Path, fid: str) -> dict | None:
    doc = _load_json(hyp / f"{fid}.perf.json") or {}
    parts = [doc[k] for k in ("asr", "diar") if k in doc]
    if not parts:
        return None
    audio = next((p["audio_s"] for p in parts if p.get("audio_s")), None)
    rss = [p["peak_rss_mb"] for p in parts if p.get("peak_rss_mb") is not None]
    return {
        "wall_s": sum(p["wall_s"] for p in parts),
        "audio_s": audio,
        "peak_rss_mb": max(rss) if rss else None,
    }


def _score_notes(hyp: Path, fid: str, run: dict) -> dict[str, bool]:
    """Schema validity per source. A missing output counts as invalid when the notes task
    failed for that file (an error was recorded); when it never ran, the file is left out."""
    out = {}
    for source, suffix in (("pipeline", "notes.json"), ("gold", "notes-gold.json")):
        path = hyp / f"{fid}.{suffix}"
        if path.is_file():
            doc = _load_json(path)
            out[source] = doc is not None and not validation_errors(doc, "notes")
        elif any(
            e["file"] == fid and e["task"] == "notes" and e.get("source") == source
            for e in run.get("errors") or []
        ):
            out[source] = False
    return out


def _expects(entry: FileEntry) -> dict[str, bool]:
    """Which metrics this file has a reference for (so a missing hypothesis is visible)."""
    asr = entry.ref is not None and entry.ref.is_file()
    if asr:
        asr = bool(normalize(entry.ref.read_text(encoding="utf-8")))
    diar = entry.rttm is not None and entry.rttm.is_file() and bool(read_rttm(entry.rttm))
    return {
        "asr": asr,
        "diar": diar,
        "notes": entry.notes_ref is not None and entry.notes_ref.is_file(),
    }


def score_run(run_dir: Path, dataset: Path | None = None, collar: float | None = None) -> dict:
    """Score every file; writes and returns scores.json."""
    run_dir = Path(run_dir)
    run = load_run(run_dir)
    manifest = load_manifest(Path(dataset or run["dataset"]))
    collar = collar if collar is not None else run.get("collar", DER_COLLAR)
    hyp = run_dir / "hyp"
    files: dict[str, dict] = {}
    for entry in manifest.files:
        duration = manifest.duration(entry)
        rec: dict = {
            "lang": entry.lang, "setting": entry.setting, "speakers": entry.bucket,
            "duration_s": duration,
        }  # fmt: skip
        for key, value in (
            ("asr", _score_asr(entry, hyp)),
            ("diar", _score_diar(entry, hyp, duration, collar)),
            ("lag", _score_lag(hyp, entry.id) if run.get("realtime") else None),
            ("perf", _perf(hyp, entry.id)),
        ):
            if value is not None:
                rec[key] = value
        rec["expects"] = _expects(entry)
        notes = _score_notes(hyp, entry.id, run)
        if notes:
            rec["notes"] = notes
        files[entry.id] = rec
    scores: dict = {
        "schema": "ghi.eval-scores/1",
        "kit_version": __version__,
        "der_collar": collar,
        "files": files,
        "speaker_id": _speaker_id(run_dir, manifest),
    }
    write_json(run_dir / "scores.json", scores)
    return scores


def _speaker_id(run_dir: Path, manifest: Manifest) -> dict | None:
    path = run_dir / "speaker_scores.tsv"
    if not path.is_file():
        return None
    return speaker_id_summary(manifest, path)


def load_scores(run_dir: Path) -> dict:
    path = Path(run_dir) / "scores.json"
    if not path.is_file():
        raise HarnessError(f"{path}: missing; run `ghi-eval score --run {run_dir}` first")
    return json.loads(path.read_text(encoding="utf-8"))
