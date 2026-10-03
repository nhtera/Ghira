# SPDX-License-Identifier: Apache-2.0
"""Scores hypothesis RTTM variants (capped diarizer vs re-clustered) against references.

`hyp/<variant>/<id>.rttm` next to a dataset (written by the ghi-core `vox_eval` test);
pooled DER, speaker-count error and the speaker counts per variant. Aggregates only:
per-file numbers are opt-in (`per_file=True`, `--per-file`) and stay out of reports.
"""

from __future__ import annotations

import json
from pathlib import Path

from .manifest import load_manifest
from .metrics import DER_COLLAR, diarization_components
from .rttm import read_rttm


def score_variants(dataset: Path, collar: float = DER_COLLAR, per_file: bool = False) -> dict:
    dataset = Path(dataset)
    manifest = load_manifest(dataset)
    hyp_root = dataset / "hyp"
    variants = sorted(p.name for p in hyp_root.iterdir() if p.is_dir())
    timing_path = hyp_root / "timing.json"
    timing = json.loads(timing_path.read_text()) if timing_path.is_file() else {}
    out = {"files": len(manifest.files), "collar": collar, "variants": {}}
    for v in variants:
        tot = dict(missed=0.0, false_alarm=0.0, confusion=0.0, ref_speech=0.0)
        count_err, abs_err, hyp_sp, ref_sp, n = 0.0, 0.0, 0, 0, 0
        per_file_out = {}
        for f in manifest.files:
            hyp = hyp_root / v / f"{f.id}.rttm"
            if f.rttm is None or not hyp.is_file():
                continue
            c = diarization_components(read_rttm(f.rttm), read_rttm(hyp), f.duration_s, collar)
            for k in tot:
                tot[k] += c[k]
            diff = c["hyp_speakers"] - c["ref_speakers"]
            count_err += diff
            abs_err += abs(diff)
            hyp_sp += c["hyp_speakers"]
            ref_sp += c["ref_speakers"]
            n += 1
            e = c["missed"] + c["false_alarm"] + c["confusion"]
            per_file_out[f.id] = round(100 * e / c["ref_speech"], 1) if c["ref_speech"] else None
        if not n or not tot["ref_speech"]:
            continue
        err = tot["missed"] + tot["false_alarm"] + tot["confusion"]
        out["variants"][v] = {
            "files": n,
            "der_pct": round(100 * err / tot["ref_speech"], 2),
            "confusion_pct": round(100 * tot["confusion"] / tot["ref_speech"], 2),
            "missed_pct": round(100 * tot["missed"] / tot["ref_speech"], 2),
            "false_alarm_pct": round(100 * tot["false_alarm"] / tot["ref_speech"], 2),
            "speaker_count_mae": round(abs_err / n, 2),
            "speaker_count_bias": round(count_err / n, 2),
            "hyp_speakers_total": hyp_sp,
            "ref_speakers_total": ref_sp,
        }
        if per_file and per_file_out:
            out["variants"][v]["per_file_der_pct"] = per_file_out
    timing = {k: t for k, t in timing.items() if k in {f.id for f in manifest.files}}
    if timing:
        dur = sum(t["dur_s"] for t in timing.values())
        out["runtime"] = {
            "audio_h": round(dur / 3600, 2),
            "capped_s_per_audio_h": round(sum(t["capped_s"] for t in timing.values()) / dur * 3600),
            "analyse_s_per_audio_h": round(
                sum(t["analyse_s"] for t in timing.values()) / dur * 3600
            ),
        }
    return out
