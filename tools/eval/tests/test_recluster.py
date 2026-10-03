# SPDX-License-Identifier: Apache-2.0
from ghi_eval.cli import main
from ghi_eval.recluster import score_variants


def test_score_variants_prefers_the_better_hypothesis(tmp_path):
    (tmp_path / "labels").mkdir()
    (tmp_path / "audio").mkdir()
    ref = [("A", 0, 10), ("B", 10, 20), ("C", 20, 30)]
    (tmp_path / "labels/f1.rttm").write_text(
        "".join(f"SPEAKER f1 1 {a} {b - a} <NA> <NA> {s} <NA> <NA>\n" for s, a, b in ref)
    )
    (tmp_path / "manifest.yaml").write_text(
        "version: 1\nname: t\nnames: []\nfiles:\n- id: f1\n  audio: audio/f1.wav\n"
        "  rttm: labels/f1.rttm\n  lang: en\n  setting: other\n  playback: na\n"
        "  speakers: 3\n  duration_s: 30.0\n"
    )
    (tmp_path / "audio/f1.wav").write_bytes(b"")

    def hyp(name, labels):
        d = tmp_path / "hyp" / name
        d.mkdir(parents=True)
        (d / "f1.rttm").write_text(
            "".join(
                f"SPEAKER f1 1 {a} {b - a} <NA> <NA> {lab} <NA> <NA>\n"
                for lab, (_, a, b) in zip(labels, ref, strict=True)
            )
        )

    hyp("capped", ["S1", "S1", "S2"])  # B and A merged
    hyp("recluster", ["S1", "S2", "S3"])
    rep = score_variants(tmp_path)
    capped, better = rep["variants"]["capped"], rep["variants"]["recluster"]
    assert better["der_pct"] < capped["der_pct"] and better["der_pct"] == 0.0
    assert capped["speaker_count_bias"] == -1 and better["speaker_count_mae"] == 0
    out = tmp_path / "r.json"
    assert main(["recluster-report", "--dataset", str(tmp_path), "--out", str(out)]) == 0
    assert out.is_file()


def test_per_file_numbers_are_opt_in(tmp_path):
    test_score_variants_prefers_the_better_hypothesis(tmp_path)
    assert "per_file_der_pct" not in score_variants(tmp_path)["variants"]["capped"]
    assert "per_file_der_pct" in score_variants(tmp_path, per_file=True)["variants"]["capped"]
