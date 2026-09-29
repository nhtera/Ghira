# SPDX-License-Identifier: Apache-2.0
"""faster-whisper as a reference/draft ASR system (ASR only). Optional extra: `whisper`."""

from __future__ import annotations

import os
from pathlib import Path

from ..audio import wav_duration
from ..errors import HarnessError
from ..perf import PeakRSS, rtf
from .base import Adapter, Measured

DEFAULT_MODEL = "large-v3"


class WhisperRefAdapter(Adapter):
    name = "whisper-ref"

    def __init__(self, spec: str, model: str | None = None):
        super().__init__(spec)
        self.model_name = model or DEFAULT_MODEL
        self._model = None

    def version(self) -> str:
        try:
            from importlib.metadata import version

            return f"faster-whisper {version('faster-whisper')} / {self._model_basename()}"
        except Exception:
            return self._model_basename()

    def _model_basename(self) -> str:
        """The model name without any path (a local model directory can hold private names)."""
        return Path(self.model_name.replace("\\", "/")).name

    def _load(self):
        if self._model is None:
            os.environ.setdefault("HF_HUB_DISABLE_TELEMETRY", "1")
            os.environ.setdefault("DO_NOT_TRACK", "1")
            try:
                from faster_whisper import WhisperModel
            except ImportError as exc:
                raise HarnessError(
                    "faster-whisper is not installed; run `uv sync --extra whisper`"
                ) from exc
            self._model = WhisperModel(self.model_name, device="cpu", compute_type="int8")
        return self._model

    def transcribe(self, file_id, audio, *, lang, pass_):
        duration = wav_duration(audio)
        with PeakRSS() as meter:
            model = self._load()
            segments, info = model.transcribe(
                str(audio), language=None if lang == "auto" else lang, vad_filter=True
            )
            segs = [
                {
                    "id": i,
                    "start": float(s.start),
                    "end": float(s.end),
                    "text": s.text.strip(),
                    "lang": info.language if info.language in ("vi", "en") else None,
                    "speaker": None,
                    "words": None,
                }
                for i, s in enumerate(segments)  # generator: decoding happens here
            ]
        doc = {
            "schema": "ghi.transcript/1",
            "audio": Path(audio).name,
            "duration_s": duration,
            "pass": pass_,
            "lang": lang,
            "engine": {"name": f"faster-whisper:{self.model_name}", "version": "1"},
            "segments": segs,
            "perf": {
                "wall_s": meter.wall_s,
                "rtf": rtf(meter.wall_s, duration),
                "peak_rss_mb": meter.peak_rss_mb,
            },
        }
        return Measured(doc, meter.wall_s, meter.peak_rss_mb)
