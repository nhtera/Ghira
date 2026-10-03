# SPDX-License-Identifier: Apache-2.0
"""Speaker-ID trials from manifest `persons` and EER from `speaker_scores.tsv`."""

from __future__ import annotations

import csv
from pathlib import Path

from .errors import HarnessError
from .manifest import Manifest, load_manifest
from .metrics import equal_error_rate

TRIAL_COLUMNS = ["enroll_file", "enroll_spk", "test_file", "test_spk", "label"]
SCORE_COLUMNS = ["enroll_file", "enroll_spk", "test_file", "test_spk", "score"]


def make_trials(manifest: Manifest) -> list[tuple[str, str, str, str, str]]:
    """Cross-meeting pairs: label `target` when both speakers are the same person, else `nontarget`.

    Each unordered pair of files is used once (earlier file enrolls, later file tests).
    """
    files = [f for f in manifest.files if f.persons]
    trials = []
    for i, a in enumerate(files):
        for b in files[i + 1 :]:
            for sa, pa in sorted(a.persons.items()):
                for sb, pb in sorted(b.persons.items()):
                    trials.append((a.id, sa, b.id, sb, "target" if pa == pb else "nontarget"))
    return trials


def write_trials(path: Path, trials: list[tuple[str, str, str, str, str]]) -> None:
    with path.open("w", encoding="utf-8", newline="") as fh:
        w = csv.writer(fh, delimiter="\t", lineterminator="\n")
        w.writerow(TRIAL_COLUMNS)
        w.writerows(trials)


def read_scores(path: Path) -> list[tuple[str, str, str, str, float]]:
    rows = []
    with path.open("r", encoding="utf-8-sig", newline="") as fh:
        for n, parts in enumerate(csv.reader(fh, delimiter="\t"), 1):
            if not parts or (len(parts) == 1 and not parts[0].strip()):
                continue
            if len(parts) != 5:
                raise HarnessError(f"{path.name}:{n}: expected 5 tab-separated columns")
            try:
                rows.append((parts[0], parts[1], parts[2], parts[3], float(parts[4])))
            except ValueError:
                if n == 1:  # header
                    continue
                raise HarnessError(f"{path.name}:{n}: score is not a number") from None
    return rows


def speaker_id_summary(manifest: Manifest, scores_path: Path) -> dict | None:
    """`{"eer", "trials"}` from the scored cross-meeting trials, or None without usable trials."""
    persons = {f.id: f.persons for f in manifest.files}
    scores, labels = [], []
    for ef, es, tf, ts, score in read_scores(scores_path):
        pe, pt = persons.get(ef, {}).get(es), persons.get(tf, {}).get(ts)
        if ef == tf or pe is None or pt is None:
            continue
        scores.append(score)
        labels.append(pe == pt)
    if not scores:
        return None
    return {"eer": equal_error_rate(scores, labels), "trials": len(scores)}


# --- Scoring voice vectors (threshold tuning for voice profiles) -----------------------------


def persons_from_rttm(dataset: Path) -> int:
    """Fill each manifest file's `persons` from its RTTM speaker labels (label -> same id).

    Only for sets whose labels are already stable pseudonymous ids across meetings (AMI's
    participant codes, e.g. FEE013). Returns the number of files updated.
    """
    import yaml

    from .rttm import read_rttm

    path = Path(dataset) / "manifest.yaml"
    raw = yaml.safe_load(path.read_text(encoding="utf-8"))
    n = 0
    for item in raw.get("files", []):
        if not item.get("rttm"):
            continue
        labels = sorted({t.speaker for t in read_rttm(Path(dataset) / item["rttm"])})
        item["persons"] = {label: label for label in labels}
        n += 1
    path.write_text(yaml.safe_dump(raw, sort_keys=False, allow_unicode=True), encoding="utf-8")
    return n


def _unit(v):
    import numpy as np

    v = np.asarray(v, dtype=float)
    return v / np.linalg.norm(v)


def pair_scores(voices: dict, trials) -> list[tuple[str, str, str, str, float]]:
    """Cosine of the unit vectors for each trial whose two sides both have a voice."""
    rows = []
    for ef, es, tf, ts, _ in trials:
        a, b = voices.get(ef, {}).get(es), voices.get(tf, {}).get(ts)
        if a is None or b is None:
            continue
        rows.append((ef, es, tf, ts, float(_unit(a["vec"]) @ _unit(b["vec"]))))
    return rows


def profile_trials(voices: dict, persons: dict[str, dict[str, str]]):
    """Leave-one-meeting-out: a person's profile is the normalised mean of their voices in
    the *other* meetings, tested against every speaker of the held-out meeting (what a saved
    voice profile meets in a new meeting). Returns (scores, labels)."""
    scores, labels = [], []
    by_person: dict[str, dict[str, object]] = {}
    for meeting, spk in voices.items():
        for label, v in spk.items():
            p = persons.get(meeting, {}).get(label)
            if p is not None:
                by_person.setdefault(p, {})[meeting] = _unit(v["vec"])
    for person, ms in by_person.items():
        if len(ms) < 2:
            continue
        for held, _ in ms.items():
            rest = [v for m, v in ms.items() if m != held]
            profile = _unit(sum(rest))
            for label, v in voices[held].items():
                q = persons[held].get(label)
                if q is None:
                    continue
                scores.append(float(profile @ _unit(v["vec"])))
                labels.append(q == person)
    return scores, labels


def operating_point(scores: list[float], labels: list[bool], threshold: float) -> dict:
    """FAR (non-targets accepted) and FRR (targets rejected) at `score >= threshold`."""
    import numpy as np

    s, y = np.asarray(scores), np.asarray(labels, dtype=bool)
    return {
        "threshold": round(float(threshold), 4),
        "far": float((s[~y] >= threshold).mean()) if (~y).any() else None,
        "frr": float((s[y] < threshold).mean()) if y.any() else None,
    }


def threshold_at_far(scores: list[float], labels: list[bool], far: float) -> dict:
    """Lowest threshold whose false-accept rate is <= `far`, with the FRR it costs."""
    import numpy as np

    s, y = np.asarray(scores), np.asarray(labels, dtype=bool)
    non = np.sort(s[~y])[::-1]
    k = int(np.floor(far * len(non)))  # non-targets allowed above the threshold
    thr = non[k] + 1e-9 if k < len(non) else non[-1]
    return operating_point(scores, labels, thr)


def distribution(scores: list[float], labels: list[bool]) -> dict:
    import numpy as np

    s, y = np.asarray(scores), np.asarray(labels, dtype=bool)

    def summ(x):
        if not len(x):
            return None
        p = np.percentile(x, [1, 5, 25, 50, 75, 95, 99])
        return {
            "n": int(len(x)),
            "mean": round(float(x.mean()), 3),
            "p1_p5_p25_p50_p75_p95_p99": [round(float(v), 3) for v in p],
        }

    return {"target": summ(s[y]), "nontarget": summ(s[~y])}


def score_set(scores: list[float], labels: list[bool]) -> dict:
    """EER, the thresholds for FAR 1% / 0.1%, the current defaults, and score distributions."""
    return {
        "trials": len(scores),
        "targets": int(sum(labels)),
        "eer": equal_error_rate(scores, labels),
        "at_far_1pct": threshold_at_far(scores, labels, 0.01),
        "at_far_0.1pct": threshold_at_far(scores, labels, 0.001),
        "at_0.70": operating_point(scores, labels, 0.70),
        "at_0.50": operating_point(scores, labels, 0.50),
        "distribution": distribution(scores, labels),
        "max_nontarget": round(
            max((x for x, y in zip(scores, labels, strict=True) if not y), default=0), 3
        ),
    }


def speakerid_report(dataset: Path, voices_path: Path, scores_out: Path | None = None) -> dict:
    """Aggregate numbers only: pairwise cross-meeting trials and leave-one-meeting-out profiles."""
    import json

    manifest = load_manifest(dataset)
    voices = json.loads(Path(voices_path).read_text(encoding="utf-8"))
    persons = {f.id: f.persons for f in manifest.files}
    rows = pair_scores(voices, make_trials(manifest))
    if scores_out is not None:
        with Path(scores_out).open("w", encoding="utf-8", newline="") as fh:
            w = csv.writer(fh, delimiter="\t", lineterminator="\n")
            w.writerow(SCORE_COLUMNS)
            w.writerows((*r[:4], f"{r[4]:.6f}") for r in rows)
    scores, labels = [], []
    for ef, es, tf, ts, sc in rows:
        scores.append(sc)
        labels.append(persons[ef][es] == persons[tf][ts])
    ps, pl = profile_trials(voices, persons)
    # Same room and microphone (AMI series = meeting id minus its letter): the hard non-targets.
    same = [i for i, r in enumerate(rows) if r[0][:-1] == r[2][:-1]]
    return {
        "pairwise_same_series": score_set([scores[i] for i in same], [labels[i] for i in same]),
        "meetings": len(voices),
        "speakers": sum(len(v) for v in voices.values()),
        "pairwise": score_set(scores, labels),
        "profile_loo": score_set(ps, pl),
    }
