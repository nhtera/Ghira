# SPDX-License-Identifier: Apache-2.0
"""Metric primitives. Everything returns raw components so callers can pool across files."""

from __future__ import annotations

import warnings
from dataclasses import dataclass

import numpy as np

from .rttm import Turn
from .textnorm import normalize

# pyannote.metrics `collar` is the TOTAL width of the forgiveness zone around each
# reference boundary. The protocol says "collar 0.25 s" meaning +-0.25 s, so the
# pyannote value is 2 * 0.25 = 0.5.
DER_COLLAR = 0.25  # +- seconds (default); pyannote wants the total width, see below
# JER follows the DIHARD convention: no collar, overlap scored.
JER_COLLAR_TOTAL = 0.0


def asr_metric_for(lang: str) -> str:
    """wer on en, syl_wer on vi (a syllable is one token), mer on mixed (code-switch)."""
    return {"en": "wer", "vi": "syl_wer", "mixed": "mer"}[lang]


@dataclass(frozen=True)
class TokenErrors:
    errors: int
    ref_tokens: int
    substitutions: int
    deletions: int
    insertions: int


def token_errors(reference: str, hypothesis: str) -> TokenErrors | None:
    """Edit-distance components between two texts after normalization.

    Returns None when the reference has no scorable tokens.
    """
    import jiwer

    ref, hyp = normalize(reference), normalize(hypothesis)
    if not ref:
        return None
    if not hyp:
        return TokenErrors(len(ref), len(ref), 0, len(ref), 0)
    out = jiwer.process_words(" ".join(ref), " ".join(hyp))
    s, d, i = out.substitutions, out.deletions, out.insertions
    return TokenErrors(s + d + i, len(ref), s, d, i)


def _annotation(turns: list[Turn], uri: str):
    from pyannote.core import Annotation, Segment

    ann = Annotation(uri=uri)
    for i, t in enumerate(turns):
        if t.end > t.start:
            ann[Segment(t.start, t.end), i] = t.speaker
    return ann


def diarization_components(
    ref: list[Turn], hyp: list[Turn], duration: float | None = None, collar: float = DER_COLLAR
) -> dict[str, float]:
    """DER and JER components for one file.

    Returns missed, false_alarm, confusion, ref_speech (DER numerators/denominator),
    jer_error, jer_speakers (JER pools by reference speaker), plus the two speaker counts.
    """
    from pyannote.core import Segment, Timeline
    from pyannote.metrics.diarization import DiarizationErrorRate, JaccardErrorRate

    r, h = _annotation(ref, "f"), _annotation(hyp, "f")
    end = max([duration or 0.0] + [t.end for t in ref] + [t.end for t in hyp])
    uem = Timeline([Segment(0.0, end)]) if end > 0 else None
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        der = DiarizationErrorRate(collar=2 * collar, skip_overlap=False)(
            r, h, uem=uem, detailed=True
        )
        n_ref = len(r.labels())
        if n_ref == 0:
            jer_err, jer_n = 0.0, 0.0
        elif len(h) == 0:
            jer_err, jer_n = float(n_ref), float(n_ref)  # every reference speaker fully missed
        else:
            jer = JaccardErrorRate(collar=JER_COLLAR_TOTAL, skip_overlap=False)(
                r, h, uem=uem, detailed=True
            )
            jer_err, jer_n = float(jer["speaker error"]), float(jer["speaker count"])
    return {
        "missed": float(der["missed detection"]),
        "false_alarm": float(der["false alarm"]),
        "confusion": float(der["confusion"]),
        "ref_speech": float(der["total"]),
        "jer_error": jer_err,
        "jer_speakers": jer_n,
        "ref_speakers": n_ref,
        "hyp_speakers": len(h.labels()),
    }


def percentile(values: list[float], q: float) -> float | None:
    """Linear-interpolated percentile (numpy default); None for no data."""
    return float(np.percentile(values, q)) if values else None


def equal_error_rate(scores: list[float], labels: list[bool]) -> float | None:
    """EER of target(True)/non-target(False) trials; higher score = more likely same speaker."""
    s = np.asarray(scores, dtype=float)
    y = np.asarray(labels, dtype=bool)
    n_t, n_n = int(y.sum()), int((~y).sum())
    if n_t == 0 or n_n == 0:
        return None
    order = np.argsort(-s, kind="stable")
    y = y[order]
    s = s[order]
    # Accept trials with score >= threshold; thresholds sit at each distinct score.
    tp = np.cumsum(y)
    fp = np.cumsum(~y)
    last_of_tie = np.r_[s[1:] != s[:-1], True]
    tp, fp = tp[last_of_tie], fp[last_of_tie]
    frr = 1.0 - tp / n_t
    far = fp / n_n
    # Add the accept-nothing point.
    frr = np.r_[1.0, frr]
    far = np.r_[0.0, far]
    i = int(np.argmin(np.abs(frr - far)))
    return float((frr[i] + far[i]) / 2.0)
