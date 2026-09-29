# SPDX-License-Identifier: Apache-2.0
"""Common interface of a system under test."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Generic, TypeVar

from ..errors import NotSupported

T = TypeVar("T")


@dataclass
class Measured(Generic[T]):
    """An adapter result plus the harness-measured cost of producing it."""

    value: T
    wall_s: float
    peak_rss_mb: float | None = None


class Adapter:
    """A system under test. Methods a system lacks raise NotSupported.

    `file_id` is the manifest id (used by `files:` to find precomputed output);
    real systems only look at the audio path.
    """

    #: short name used in run ids and report file names ("ghi", "files", ...)
    name = "system"
    #: False for adapters that replay stored output (no wall time or RSS worth reporting)
    measures_perf = True

    def __init__(self, spec: str):
        self.spec = spec
        #: per-call limit in seconds for subprocess systems, set by the runner before each task
        self.timeout: float | None = None

    def version(self) -> str:
        return "unknown"

    def transcribe(self, file_id: str, audio: Path, *, lang: str, pass_: str) -> Measured[dict]:
        """Return a `ghi.transcript/1` document."""
        raise NotSupported("asr")

    def transcribe_stream(
        self, file_id: str, audio: Path, *, lang: str, realtime: bool
    ) -> Measured[list[dict]]:
        """Return the `ghi.event/1` documents of a streaming run."""
        raise NotSupported("stream")

    def diarize(self, file_id: str, audio: Path, *, pass_: str) -> Measured[dict]:
        """Return a `ghi.diarization/1` document."""
        raise NotSupported("diar")

    def notes(
        self, file_id: str, transcript: Path, *, lang: str, source: str = "pipeline"
    ) -> Measured[dict]:
        """Return a `ghi.notes/1` document made from the `ghi.transcript/1` file at `transcript`.

        `source` is "pipeline" (the system's own transcript) or "gold" (built from the reference).
        """
        raise NotSupported("notes")
