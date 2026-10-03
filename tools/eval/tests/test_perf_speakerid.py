# SPDX-License-Identifier: Apache-2.0
import sys

from ghi_eval.cli import main
from ghi_eval.manifest import load_manifest
from ghi_eval.perf import PeakRSS, run_measured
from ghi_eval.speakerid import make_trials, read_scores


def test_run_measured_tracks_child_memory():
    code = "import time; b = bytearray(80 * 1024 * 1024); b[::4096] = b'x' * len(b[::4096]); time.sleep(0.4)"
    done = run_measured([sys.executable, "-c", code])
    assert done.returncode == 0 and done.wall_s >= 0.4
    assert done.peak_rss_mb is not None and done.peak_rss_mb > 60


def test_run_measured_captures_output_and_exit_code():
    done = run_measured(
        [
            sys.executable,
            "-c",
            "import sys; print('out'); print('err', file=sys.stderr); sys.exit(3)",
        ]
    )
    assert (done.returncode, done.stdout.strip(), done.stderr.strip()) == (3, "out", "err")


def test_peak_rss_context():
    with PeakRSS() as meter:
        block = bytearray(50 * 1024 * 1024)
        block[::4096] = b"x" * len(block[::4096])
    assert meter.peak_rss_mb and meter.wall_s >= 0


def test_trials_and_eer(tiny, tmp_path, tiny_hyp):
    trials = make_trials(load_manifest(tiny))
    # t01 (P01,P02) x t02 (P01,P03): 4 cross pairs, one target
    assert len(trials) == 4 and [t[4] for t in trials].count("target") == 1
    assert main(["trials", "--dataset", str(tiny)]) == 0
    tsv = (tiny / "trials.tsv").read_text(encoding="utf-8").splitlines()
    assert tsv[0].split("\t")[-1] == "label" and len(tsv) == 5

    out = tmp_path / "out"
    scores = {("t01", "spk1", "t02", "spk1"): 0.9, ("t01", "spk1", "t02", "spk2"): 0.2,
              ("t01", "spk2", "t02", "spk1"): 0.1, ("t01", "spk2", "t02", "spk2"): 0.3}  # fmt: skip
    run = [
        "run",
        "--dataset",
        str(tiny),
        "--system",
        f"files:{tiny_hyp}",
        "--run-id",
        "s",
        "--tasks",
        "asr",
        "--out",
        str(out),
    ]
    assert main(run) == 0
    lines = ["enroll_file\tenroll_spk\ttest_file\ttest_spk\tscore"]
    lines += ["\t".join([*k, str(v)]) for k, v in scores.items()]
    (out / "s" / "speaker_scores.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8")
    assert len(read_scores(out / "s" / "speaker_scores.tsv")) == 4
    assert main(["report", "--run", str(out / "s")]) == 0
    import json

    rep = json.loads(next((out / "s").glob("report-*.json")).read_text(encoding="utf-8"))
    assert rep["speaker_id"] == {"eer": 0.0, "trials": 4}


def test_speakerid_scoring_and_thresholds():
    import numpy as np

    from ghi_eval.speakerid import profile_trials, score_set, threshold_at_far

    rng = np.random.default_rng(0)
    base = {p: rng.normal(size=16) for p in "ABC"}

    def vec(p):
        return {"vec": list(base[p] + 0.2 * rng.normal(size=16))}

    voices = {f"m{i}": {p: vec(p) for p in "ABC"} for i in range(3)}
    persons = {m: {p: p for p in "ABC"} for m in voices}
    scores, labels = profile_trials(voices, persons)
    assert len(scores) == 3 * 3 * 3 and sum(labels) == 9
    res = score_set(scores, labels)
    assert res["eer"] == 0.0 and res["at_far_1pct"]["frr"] == 0.0
    assert res["at_0.70"]["far"] == 0.0

    # 100 non-targets at 0.0..0.99; FAR 1% allows exactly one above the threshold.
    s = [i / 100 for i in range(100)] + [2.0]
    y = [False] * 100 + [True]
    op = threshold_at_far(s, y, 0.01)
    assert op["far"] == 0.01 and op["frr"] == 0.0
