# SPDX-License-Identifier: Apache-2.0
"""Privacy lint (formats.md §6): a report must not carry reference text, names or file ids."""

from __future__ import annotations

import json
import re
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path

import yaml

from .errors import HarnessError
from .manifest import Manifest
from .textnorm import fold_tokens, normalize

NGRAM = 3
_DECIMAL = re.compile(r"\d+\.\d+")
# A path separator, or a Windows drive prefix ("C:" at the end or before a separator).
_PATH = re.compile(r"[\\/]|(?<![A-Za-z0-9_])[A-Za-z]:$")
_NUMERIC = re.compile(r"^[-+\d.eE%]+$")
_SLICE_CELL = re.compile(
    r"^(all|(lang|setting|speakers)=[\w+-]+(,(lang|setting|speakers)=[\w+-]+)*)$"
)

# Words the report itself is made of. Names and file ids are only looked for in text that
# is not one of these (a name "Vi" or a file id "room" must not fail a clean report).
FIXED_VOCAB = frozenset(
    """lang setting spk files hours der jer spk_count_err wer syl_wer mer partial_lag_p50
    partial_lag_p95 final_lag_p50 final_lag_p95 rtf peak_rss_mb asr_files diar_files lag_files
    notes_files vi en mixed room call other 1-2 3-5 6+ all id metric slice max floor value status
    pass best_effort fail n/a incomplete eer trials source schema_valid precision recall owner_acc
    citation_valid hallucinations pipeline gold passed ngram names_checked true false -""".split()
)
# JSON paths (list indices as *) holding fixed vocabulary; every other string is free text.
FIXED_PATHS = frozenset(
    {
        ("schema",), ("generated",), ("kit_version",), ("run", "pass"), ("run", "lang"),
        ("run", "tasks", "*"), ("slices", "*", "lang"), ("slices", "*", "setting"),
        ("slices", "*", "speakers"), ("llm", "*", "source"), ("gates", "*", "metric"),
        ("gates", "*", "status"), ("gates", "*", "slice", "lang"),
        ("gates", "*", "slice", "setting"), ("gates", "*", "slice", "speakers"),
    }
)  # fmt: skip


@dataclass
class LintIndex:
    """What must never appear in a report, built once from the manifest and its references."""

    ref_ngrams: set[tuple[str, ...]] = field(default_factory=set)
    note_ngrams: set[tuple[str, ...]] = field(default_factory=set)
    names: list[tuple[str, ...]] = field(default_factory=list)  # diacritic-folded tokens
    ids: list[re.Pattern] = field(default_factory=list)


@dataclass
class LintResult:
    findings: list[dict]
    names_checked: int

    @property
    def passed(self) -> bool:
        return not self.findings


def _ngrams(tokens: list[str]) -> Iterator[tuple[str, ...]]:
    for i in range(len(tokens) - NGRAM + 1):
        yield tuple(tokens[i : i + NGRAM])


def _strings(node) -> Iterator[str]:
    if isinstance(node, str):
        yield node
    elif isinstance(node, dict):
        for k, v in node.items():
            yield str(k)
            yield from _strings(v)
    elif isinstance(node, list):
        for v in node:
            yield from _strings(v)


def build_index(manifest: Manifest) -> LintIndex:
    idx = LintIndex()
    names: set[tuple[str, ...]] = set()
    for n in manifest.names:
        toks = tuple(fold_tokens(n))
        if toks:
            names.add(toks)
    for f in manifest.files:
        if f.ref is not None and f.ref.is_file():
            idx.ref_ngrams.update(_ngrams(normalize(f.ref.read_text(encoding="utf-8"))))
        if f.notes_ref is not None and f.notes_ref.is_file():
            doc = yaml.safe_load(f.notes_ref.read_text(encoding="utf-8")) or {}
            for s in _strings(doc):
                idx.note_ngrams.update(_ngrams(normalize(s)))
            for item in doc.get("action_items") or []:
                owner = item.get("owner") if isinstance(item, dict) else None
                if isinstance(owner, str) and fold_tokens(owner):
                    names.add(tuple(fold_tokens(owner)))
        for raw in (f.id, f.audio.stem):
            if raw and not raw.isdigit() and len(raw) >= 3:
                idx.ids.append(
                    re.compile(r"(?<![A-Za-z0-9_-])" + re.escape(raw) + r"(?![A-Za-z0-9_-])")
                )
    idx.names = sorted(names)
    return idx


def _find_sub(tokens: list[str], sub: tuple[str, ...]) -> bool:
    n = len(sub)
    return any(tuple(tokens[i : i + n]) == sub for i in range(len(tokens) - n + 1))


def _scan(units: list[str], idx: LintIndex, *, ngrams: bool, names: bool) -> list[dict]:
    """Findings for text units (n-grams never span two units)."""
    found: dict[tuple[str, str], dict] = {}

    def add(kind: str, detail: str) -> None:
        found.setdefault((kind, detail), {"kind": kind, "detail": detail})

    for unit in units:
        if ngrams:
            for gram in _ngrams(normalize(unit)):
                for kind, pool in (
                    ("reference_ngram", idx.ref_ngrams),
                    ("reference_note_ngram", idx.note_ngrams),
                ):
                    if gram in pool:
                        add(kind, " ".join(gram))
        if names:
            folded = fold_tokens(unit)
            for name in idx.names:
                if _find_sub(folded, name):
                    add("name", " ".join(name))
            for pat in idx.ids:
                m = pat.search(unit)
                if m:
                    add("file_id", m.group(0))
    return list(found.values())


def _path_findings(units: list[str]) -> list[dict]:
    return [{"kind": "path", "detail": u[:80]} for u in units if _PATH.search(u)]


def _walk(node, path: tuple = ()) -> Iterator[tuple[tuple, str, bool]]:
    """(generic path, string, is_key) for every key and string value."""
    if isinstance(node, str):
        yield path, node, False
    elif isinstance(node, dict):
        for k, v in node.items():
            yield path, str(k), True
            yield from _walk(v, (*path, str(k)))
    elif isinstance(node, list):
        for v in node:
            yield from _walk(v, (*path, "*"))


def lint_document(doc, idx: LintIndex) -> LintResult:
    """Lint a parsed JSON report.

    Every key and string gets the n-gram and path checks; numbers are ignored. Names and
    file ids are checked in free-text strings only (not in fixed vocabulary).
    """
    items = list(_walk(doc))
    every = [s for _, s, _ in items]
    free = [s for p, s, is_key in items if not is_key and p not in FIXED_PATHS]
    findings = _scan(every, idx, ngrams=True, names=False)
    findings += _scan(free, idx, names=True, ngrams=False)
    findings += _path_findings([s for p, s, is_key in items if is_key or p not in FIXED_PATHS])
    return LintResult(findings, len(idx.names))


def _cells(line: str) -> list[str]:
    return [c.strip() for c in line.strip().strip("|").split("|")]


def lint_text(text: str, idx: LintIndex) -> LintResult:
    """Lint rendered Markdown: one unit per line, decimal numbers act as separators.

    Names and ids are checked only in the `system` and `dataset` lines and in table cells
    that are not fixed vocabulary, numbers or gate-slice descriptions.
    """
    lines = text.splitlines()
    free: list[str] = []
    for line in lines:
        if line.startswith(("- system:", "- dataset:")):
            free.append(line.split(":", 1)[1])
        elif line.startswith("|"):
            for cell in _cells(line):
                if cell in FIXED_VOCAB or _NUMERIC.match(cell) or _SLICE_CELL.match(cell):
                    continue
                if cell and not set(cell) <= set("-:"):
                    free.append(cell)
    findings = _scan([_DECIMAL.sub(" | ", ln) for ln in lines], idx, ngrams=True, names=False)
    findings += _scan(free, idx, names=True, ngrams=False)
    prose = [ln for ln in lines if not ln.startswith("|")]
    cells = [c for ln in lines if ln.startswith("|") for c in _cells(ln) if c not in FIXED_VOCAB]
    findings += _path_findings(prose + cells)
    return LintResult(findings, len(idx.names))


def lint_file(path: Path, manifest: Manifest) -> LintResult:
    idx = build_index(manifest)
    text = Path(path).read_text(encoding="utf-8")
    if Path(path).suffix.lower() == ".json":
        try:
            return lint_document(json.loads(text), idx)
        except json.JSONDecodeError as exc:
            raise HarnessError(f"{Path(path).name}: not valid JSON") from exc
    return lint_text(text, idx)
