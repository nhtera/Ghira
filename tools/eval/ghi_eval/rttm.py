# SPDX-License-Identifier: Apache-2.0
"""NIST RTTM reading and writing (SPEAKER lines only)."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

from .errors import HarnessError


@dataclass(frozen=True)
class Turn:
    start: float
    end: float
    speaker: str


def clean_label(label: str) -> str:
    """RTTM fields are space separated, so a speaker label cannot contain whitespace."""
    return re.sub(r"\s+", "_", label.strip())


def read_rttm(path: Path, file_id: str | None = None) -> list[Turn]:
    """Parse SPEAKER lines. With `file_id`, a different file-id column is an error."""
    turns: list[Turn] = []
    for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        parts = line.split()
        if not parts or parts[0] != "SPEAKER":
            continue
        if len(parts) < 8:
            raise HarnessError(f"{path.name}:{n}: expected at least 8 RTTM fields")
        if file_id is not None and parts[1] != file_id:
            raise HarnessError(f"{path.name}:{n}: file-id column does not match the manifest id")
        try:
            start, dur = float(parts[3]), float(parts[4])
        except ValueError as exc:
            raise HarnessError(f"{path.name}:{n}: bad start/duration") from exc
        if dur < 0 or start < 0:
            raise HarnessError(f"{path.name}:{n}: negative start/duration")
        turns.append(Turn(start, start + dur, parts[7]))
    return turns


def format_rttm(file_id: str, turns: list[Turn]) -> str:
    lines = [
        f"SPEAKER {file_id} 1 {t.start:.3f} {t.end - t.start:.3f} <NA> <NA> {clean_label(t.speaker)} <NA> <NA>"
        for t in sorted(turns, key=lambda t: (t.start, t.end, t.speaker))
    ]
    return "\n".join(lines) + ("\n" if lines else "")


def write_rttm(path: Path, file_id: str, turns: list[Turn]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(format_rttm(file_id, turns), encoding="utf-8", newline="\n")
