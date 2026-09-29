# SPDX-License-Identifier: Apache-2.0
"""Precomputed hypotheses laid out like a run directory's `hyp/` (formats.md §4)."""

from __future__ import annotations

import json
from pathlib import Path

from ..contract import validation_errors
from ..errors import AdapterError, HarnessError, InvalidOutput, NotSupported
from ..rttm import Turn, read_rttm
from .base import Adapter, Measured


class FilesAdapter(Adapter):
    """`files:<dir>`; hypotheses live in `<dir>/hyp/` (or directly in `<dir>` if it has no hyp/)."""

    name = "files"
    measures_perf = False

    def __init__(self, spec: str, directory: str):
        super().__init__(spec)
        root = Path(directory)
        if not root.is_dir():
            raise HarnessError(f"files: directory not found: {root}")
        self.hyp = root / "hyp" if (root / "hyp").is_dir() else root

    def version(self) -> str:
        return "precomputed"

    def _load(self, task: str, file_id: str, suffix: str, kind: str) -> Measured[dict]:
        path = self.hyp / f"{file_id}.{suffix}"
        if not path.is_file():
            raise NotSupported(task, "no precomputed output")
        try:
            doc = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            raise AdapterError(f"{path.name}: not JSON") from exc
        problems = validation_errors(doc, kind)
        if problems:
            raise InvalidOutput(f"{path.name}: invalid ghi.{kind}/1 ({problems[0]})", raw=doc)
        return Measured(doc, 0.0)

    def transcribe(self, file_id, audio, *, lang, pass_):
        return self._load("asr", file_id, "transcript.json", "transcript")

    def transcribe_stream(self, file_id, audio, *, lang, realtime):
        path = self.hyp / f"{file_id}.events.ndjson"
        if not path.is_file():
            raise NotSupported("stream", "no precomputed events")
        events = []
        for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if not line.strip():
                continue
            try:
                ev = json.loads(line)
            except json.JSONDecodeError as exc:
                raise AdapterError(f"{path.name}: line {n} is not JSON") from exc
            if validation_errors(ev, "event"):
                raise InvalidOutput(f"{path.name}: line {n} invalid", raw=ev)
            events.append(ev)
        return Measured(events, 0.0)

    def diarize(self, file_id, audio, *, pass_):
        rttm = self.hyp / f"{file_id}.rttm"
        if not rttm.is_file():
            raise NotSupported("diar", "no precomputed output")
        return Measured(diarization_doc(audio.name, 0.0, "final", "files", read_rttm(rttm)), 0.0)

    def notes(self, file_id, transcript, *, lang, source="pipeline"):
        suffix = "notes-gold.json" if source == "gold" else "notes.json"
        return self._load("notes", file_id, suffix, "notes")


def diarization_doc(
    audio: str, duration: float, pass_: str, engine: str, turns: list[Turn]
) -> dict:
    return {
        "schema": "ghi.diarization/1",
        "audio": audio,
        "duration_s": duration,
        "pass": pass_,
        "engine": {"name": engine, "version": "1"},
        "turns": [{"start": t.start, "end": t.end, "speaker": t.speaker} for t in turns],
        "perf": {"wall_s": 0.0, "rtf": None, "peak_rss_mb": None},
    }
