# SPDX-License-Identifier: Apache-2.0
"""LLM note evaluation (formats.md §5): gold transcripts, judgement sheets and their metrics."""

from __future__ import annotations

import csv
import io
import json
from collections import Counter
from pathlib import Path

import yaml

from .errors import HarnessError
from .manifest import FileEntry
from .textnorm import normalize

JUDGEMENT_COLUMNS = [
    "file", "source", "sys_idx", "sys_text", "sys_owner", "sys_citations", "cited_text", "ref_idx", "ref_text", "ref_owner",
    "owner_ok", "citation_ok", "hallucinated",
]  # fmt: skip
MATCH_F1 = 0.5
# Sheets made before the citation columns existed are still readable.
REQUIRED_COLUMNS = [c for c in JUDGEMENT_COLUMNS if c not in ("sys_citations", "cited_text")]
NOTES_FILES = {"pipeline": "notes.json", "gold": "notes-gold.json"}


def gold_transcript(entry: FileEntry, duration_s: float) -> dict:
    """A `ghi.transcript/1` built from the reference text: one segment per non-empty line.

    The reference has no timestamps, so every segment has start = end = 0.0. Notes
    generation only needs the text and the segment ids (citations).
    """
    assert entry.ref is not None
    lines = [ln.strip() for ln in entry.ref.read_text(encoding="utf-8").splitlines() if ln.strip()]
    return {
        "schema": "ghi.transcript/1",
        "audio": entry.audio.name,
        "duration_s": duration_s,
        "pass": "final",
        "lang": "auto",
        "engine": {"name": "reference", "version": "1"},
        "segments": [
            {
                "id": i,
                "start": 0.0,
                "end": 0.0,
                "text": t,
                "lang": None,
                "speaker": None,
                "words": None,
            }
            for i, t in enumerate(lines)
        ],
        "perf": {"wall_s": 0.0, "rtf": None, "peak_rss_mb": None},
    }


def token_f1(a: str, b: str) -> float:
    ta, tb = normalize(a), normalize(b)
    if not ta or not tb:
        return 0.0
    common = sum((Counter(ta) & Counter(tb)).values())
    if common == 0:
        return 0.0
    p, r = common / len(ta), common / len(tb)
    return 2 * p * r / (p + r)


def match_items(sys_texts: list[str], ref_texts: list[str]) -> dict[int, int]:
    """Greedy best-first one-to-one matching with token F1 >= MATCH_F1. Returns {sys_idx: ref_idx}."""
    pairs = sorted(
        ((token_f1(s, r), i, j) for i, s in enumerate(sys_texts) for j, r in enumerate(ref_texts)),
        key=lambda p: (-p[0], p[1], p[2]),
    )
    used_s: set[int] = set()
    used_r: set[int] = set()
    out: dict[int, int] = {}
    for f1, i, j in pairs:
        if f1 < MATCH_F1:
            break
        if i in used_s or j in used_r:
            continue
        out[i] = j
        used_s.add(i)
        used_r.add(j)
    return out


def load_reference_notes(path: Path) -> list[dict]:
    doc = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    items = doc.get("action_items") or []
    return [{"text": str(i.get("text", "")), "owner": i.get("owner")} for i in items]


def _s(v) -> str:
    return "" if v is None else str(v)


def build_judgements(run_dir: Path, manifest) -> list[dict]:
    """Rows of judgements.csv for every file that has reference notes and a system notes output."""
    rows: list[dict] = []
    for entry in manifest.files:
        if entry.notes_ref is None or not entry.notes_ref.is_file():
            continue
        refs = load_reference_notes(entry.notes_ref)
        for source, fname in NOTES_FILES.items():
            path = run_dir / "hyp" / f"{entry.id}.{fname}"
            if not path.is_file():
                continue
            try:
                items = json.loads(path.read_text(encoding="utf-8")).get("action_items") or []
            except (json.JSONDecodeError, AttributeError):
                continue
            sys_items = [i for i in items if isinstance(i, dict)]
            segments = _segment_texts(run_dir, entry, source)
            matches = match_items([_s(i.get("text")) for i in sys_items], [r["text"] for r in refs])
            for i, item in enumerate(sys_items):
                j = matches.get(i)
                cites = [c for c in item.get("citations") or [] if isinstance(c, int)]
                ref = refs[j] if j is not None else None
                rows.append(
                    {
                        "file": entry.id, "source": source, "sys_idx": i,
                        "sys_text": _s(item.get("text")), "sys_owner": _s(item.get("owner")),
                        "sys_citations": ";".join(str(c) for c in cites),
                        "cited_text": " / ".join(segments.get(c, "?") for c in cites),
                        "ref_idx": "" if j is None else j,
                        "ref_text": ref["text"] if ref else "", "ref_owner": _s(ref["owner"]) if ref else "",
                        "owner_ok": "", "citation_ok": "", "hallucinated": "",
                    }
                )  # fmt: skip
            for j, ref in enumerate(refs):
                if j not in matches.values():
                    rows.append(
                        {
                            "file": entry.id, "source": source, "sys_idx": "", "sys_text": "",
                            "sys_owner": "", "sys_citations": "", "cited_text": "", "ref_idx": j, "ref_text": ref["text"],
                            "ref_owner": _s(ref["owner"]), "owner_ok": "", "citation_ok": "",
                            "hallucinated": "",
                        }
                    )  # fmt: skip
    return rows


def _segment_texts(run_dir: Path, entry: FileEntry, source: str) -> dict[int, str]:
    """Text of each transcript segment by id, for judging citations."""
    if source == "gold":
        return (
            {s["id"]: s["text"] for s in gold_transcript(entry, 0.0)["segments"]}
            if entry.ref
            else {}
        )
    try:
        doc = json.loads(
            (run_dir / "hyp" / f"{entry.id}.transcript.json").read_text(encoding="utf-8")
        )
        return {s["id"]: s["text"] for s in doc["segments"]}
    except (OSError, json.JSONDecodeError, KeyError, TypeError):
        return {}


FORMULA_START = ("=", "+", "-", "@", "\t", "\r")


def _neutralize(value):
    """Stop a spreadsheet from running a text cell as a formula: prefix `'`."""
    if isinstance(value, str) and value.startswith(FORMULA_START):
        return "'" + value
    return value


def _restore(value: str) -> str:
    return value[1:] if value.startswith("'") and value[1:].startswith(FORMULA_START) else value


def write_judgements(path: Path, rows: list[dict]) -> None:
    # utf-8-sig (BOM) so Excel opens Vietnamese text correctly; the reader accepts both.
    with path.open("w", encoding="utf-8-sig", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=JUDGEMENT_COLUMNS)
        w.writeheader()
        w.writerows({k: _neutralize(v) for k, v in row.items()} for row in rows)


def read_judgements(path: Path) -> list[dict]:
    try:
        text = path.read_text(encoding="utf-8-sig")
    except UnicodeDecodeError as exc:
        raise HarnessError(f"{path.name}: save the sheet as CSV UTF-8") from exc
    try:  # Excel in some locales saves with ; or tab
        delimiter = csv.Sniffer().sniff(text[:4096].split("\n", 1)[0], delimiters=",;\t").delimiter
    except csv.Error:
        delimiter = ","
    reader = csv.DictReader(io.StringIO(text, newline=""), delimiter=delimiter)
    missing = set(REQUIRED_COLUMNS) - set(reader.fieldnames or [])
    if missing:
        raise HarnessError(f"{path.name}: missing columns {', '.join(sorted(missing))}")
    return [{k: _restore((v or "").strip()) for k, v in row.items() if k} for row in reader]


def judgement_metrics(rows: list[dict], source: str) -> tuple[dict, str | None]:
    """P/R/owner accuracy/citation validity/hallucinations for one source.

    Returns (metrics, problem). All metrics are None with a problem string while any system
    row still lacks `citation_ok` (y/n/na) or `hallucinated` (y/n): the sheet is not filled in.
    """
    empty = {"precision": None, "recall": None, "owner_acc": None, "citation_valid": None,
             "hallucinations": None}  # fmt: skip
    rows = [r for r in rows if r["source"] == source]
    sys_rows = [r for r in rows if r["sys_idx"] != ""]
    if not rows:
        return empty, None
    todo = [
        r for r in sys_rows
        if r["citation_ok"].lower() not in ("y", "n", "na") or r["hallucinated"].lower() not in ("y", "n")
    ]  # fmt: skip
    if todo:
        return empty, f"{len(todo)} of {len(sys_rows)} {source} rows not judged yet"
    matched = [r for r in sys_rows if r["ref_idx"] != ""]
    refs = {(r["file"], r["ref_idx"]) for r in rows if r["ref_idx"] != ""}
    matched_refs = {(r["file"], r["ref_idx"]) for r in matched}
    owner_rows = [r for r in matched if r["owner_ok"].lower() in ("y", "n")]
    cited = [r for r in sys_rows if r["citation_ok"].lower() in ("y", "n")]
    return {
        "precision": len(matched) / len(sys_rows) if sys_rows else None,
        "recall": len(matched_refs) / len(refs) if refs else None,
        "owner_acc": sum(r["owner_ok"].lower() == "y" for r in owner_rows) / len(owner_rows)
        if owner_rows else None,
        "citation_valid": sum(r["citation_ok"].lower() == "y" for r in cited) / len(cited)
        if cited else None,
        "hallucinations": sum(r["hallucinated"].lower() == "y" for r in sys_rows),
    }, None  # fmt: skip


def llm_summary(
    notes_scores: dict[str, dict[str, bool]], rows: list[dict] | None
) -> tuple[list[dict], list[str]]:
    """`llm` section of the report. `notes_scores[source][file_id]` = schema validity."""
    out, warnings = [], []
    for source in ("pipeline", "gold"):
        valid = notes_scores.get(source, {})
        if not valid and not any(r["source"] == source for r in rows or []):
            continue
        metrics, problem = (
            judgement_metrics(rows, source) if rows is not None else (None, "no judgements.csv")
        )
        if metrics is None:
            metrics = {"precision": None, "recall": None, "owner_acc": None, "citation_valid": None,
                       "hallucinations": None}  # fmt: skip
        if problem:
            warnings.append(problem)
        out.append(
            {
                "source": source,
                "files": len(valid),
                "schema_valid": sum(valid.values()) / len(valid) if valid else None,
                **metrics,
            }
        )
    return out, warnings
