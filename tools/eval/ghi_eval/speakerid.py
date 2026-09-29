# SPDX-License-Identifier: Apache-2.0
"""Speaker-ID trials from manifest `persons` and EER from `speaker_scores.tsv`."""

from __future__ import annotations

import csv
from pathlib import Path

from .errors import HarnessError
from .manifest import Manifest
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
