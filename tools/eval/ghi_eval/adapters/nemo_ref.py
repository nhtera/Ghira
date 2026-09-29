# SPDX-License-Identifier: Apache-2.0
"""NeMo Python models as reference baselines and labelling drafts. Optional extra: `nemo`.

Config (`nemo-ref:<config.yaml>`), every key optional:

    asr_model: nvidia/parakeet-tdt-0.6b-v2        # English / auto
    asr_model_vi: nvidia/parakeet-ctc-0.6b-Vietnamese   # used when --lang vi
    diar_model: nvidia/Nemotron-3-Diarization      # alternative: nvidia/diar_streaming_sortformer_4spk-v2.1
    device: cpu                                    # or cuda

All defaults exist on the Hugging Face hub and are not gated. Nemotron-3-Diarization
(OpenMDW-1.1) loads through SortformerEncLabelModel like the Sortformer models.
Candidate for phase 3, once its NeMo Python usage is confirmed:
nvidia/nemotron-3.5-asr-streaming-0.6b (vi and en, target_lang=auto).
"""

from __future__ import annotations

import os
import tempfile
from pathlib import Path

import yaml

from ..audio import to_mono_16k
from ..errors import AdapterError, HarnessError
from ..perf import PeakRSS, rtf
from ..rttm import Turn
from .base import Adapter, Measured
from .files import diarization_doc

DEFAULTS = {
    "asr_model": "nvidia/parakeet-tdt-0.6b-v2",
    "asr_model_vi": "nvidia/parakeet-ctc-0.6b-Vietnamese",
    "diar_model": "nvidia/Nemotron-3-Diarization",
    "device": "cpu",
}


def _import_nemo():
    os.environ.setdefault("HF_HUB_DISABLE_TELEMETRY", "1")
    os.environ.setdefault("DO_NOT_TRACK", "1")
    try:
        import nemo.collections.asr as nemo_asr
    except ImportError as exc:
        raise HarnessError("NeMo is not installed; run `uv sync --extra nemo`") from exc
    return nemo_asr


def parse_segment(item) -> Turn:
    """A NeMo diarization segment: "start end speaker_0" or a (start, end, speaker) tuple."""
    if isinstance(item, str):
        start, end, spk = item.split()[:3]
    else:
        start, end, spk = item[:3]
    return Turn(float(start), float(end), str(spk))


class NemoRefAdapter(Adapter):
    name = "nemo-ref"

    def __init__(self, spec: str, config: str | None = None):
        super().__init__(spec)
        self.cfg = dict(DEFAULTS)
        if config:
            try:
                loaded = yaml.safe_load(Path(config).read_text(encoding="utf-8")) or {}
            except (OSError, yaml.YAMLError) as exc:
                raise HarnessError(f"nemo-ref config unreadable: {exc}") from exc
            if not isinstance(loaded, dict):
                raise HarnessError("nemo-ref config must be a mapping")
            self.cfg.update({k: v for k, v in loaded.items() if k in DEFAULTS})
        self._models: dict[str, object] = {}

    def version(self) -> str:
        base = lambda name: str(name).rsplit("/", 1)[-1]  # noqa: E731  (no org/path in reports)
        return f"asr={base(self.cfg['asr_model'])} diar={base(self.cfg['diar_model'])}"

    def _asr(self, lang: str):
        name = self.cfg["asr_model_vi"] if lang == "vi" else self.cfg["asr_model"]
        if name not in self._models:
            nemo_asr = _import_nemo()
            model = nemo_asr.models.ASRModel.from_pretrained(model_name=name)
            self._models[name] = model.to(self.cfg["device"]).eval()
        return name, self._models[name]

    def _diar(self):
        name = self.cfg["diar_model"]
        if name not in self._models:
            nemo_asr = _import_nemo()
            model = nemo_asr.models.SortformerEncLabelModel.from_pretrained(name)
            self._models[name] = model.to(self.cfg["device"]).eval()
        return self._models[name]

    def transcribe(self, file_id, audio, *, lang, pass_):
        with tempfile.TemporaryDirectory() as tmp, PeakRSS() as meter:
            wav = Path(tmp) / "audio16k.wav"
            duration = to_mono_16k(Path(audio), wav)
            name, model = self._asr(lang)
            out = model.transcribe([str(wav)], timestamps=True)
            hyp = out[0][0] if isinstance(out, tuple) else out[0]  # older RNNT: (best, all)
            segments = _segments_from_hypothesis(hyp, duration, lang)
        doc = {
            "schema": "ghi.transcript/1",
            "audio": Path(audio).name,
            "duration_s": duration,
            "pass": pass_,
            "lang": lang,
            "engine": {"name": f"nemo:{name}", "version": "1"},
            "segments": segments,
            "perf": {
                "wall_s": meter.wall_s,
                "rtf": rtf(meter.wall_s, duration),
                "peak_rss_mb": meter.peak_rss_mb,
            },
        }
        return Measured(doc, meter.wall_s, meter.peak_rss_mb)

    def diarize(self, file_id, audio, *, pass_):
        with tempfile.TemporaryDirectory() as tmp, PeakRSS() as meter:
            wav = Path(tmp) / "audio16k.wav"
            duration = to_mono_16k(Path(audio), wav)
            result = self._diar().diarize(audio=[str(wav)], batch_size=1)
            try:
                turns = [parse_segment(s) for s in result[0]]
            except (ValueError, IndexError, TypeError) as exc:
                raise AdapterError("nemo diarization: unexpected output format") from exc
        doc = diarization_doc(
            Path(audio).name, duration, pass_, f"nemo:{self.cfg['diar_model']}", turns
        )
        doc["perf"] = {
            "wall_s": meter.wall_s,
            "rtf": rtf(meter.wall_s, duration),
            "peak_rss_mb": meter.peak_rss_mb,
        }
        return Measured(doc, meter.wall_s, meter.peak_rss_mb)


def _segments_from_hypothesis(hyp, duration: float, lang: str) -> list[dict]:
    """Segments from NeMo timestamps when available, else one segment for the whole text."""
    text = getattr(hyp, "text", hyp if isinstance(hyp, str) else "")
    stamps = (getattr(hyp, "timestamp", None) or {}).get("segment") or []
    lang_tag = lang if lang in ("vi", "en") else None
    segs = [
        {
            "id": i,
            "start": float(s["start"]),
            "end": float(s["end"]),
            "text": str(s["segment"]).strip(),
            "lang": lang_tag,
            "speaker": None,
            "words": None,
        }
        for i, s in enumerate(stamps)
    ]
    if not segs and text.strip():
        segs = [
            {
                "id": 0,
                "start": 0.0,
                "end": duration,
                "text": text.strip(),
                "lang": lang_tag,
                "speaker": None,
                "words": None,
            }
        ]
    return segs
