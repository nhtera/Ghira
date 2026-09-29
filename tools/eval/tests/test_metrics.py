# SPDX-License-Identifier: Apache-2.0
import pytest

from ghi_eval.metrics import (
    asr_metric_for,
    diarization_components,
    equal_error_rate,
    percentile,
    token_errors,
)
from ghi_eval.report import aggregate, gate_status
from ghi_eval.rttm import Turn
from ghi_eval.textnorm import fold, normalize


def T(a, b, s):
    return Turn(a, b, s)


def test_normalize_vietnamese_english_and_brackets():
    assert normalize("Mình chốt scope cho beta nhé.") == [
        "mình",
        "chốt",
        "scope",
        "cho",
        "beta",
        "nhé",
    ]
    assert normalize("Okay,  I'll   send [unk] the DOC!") == ["okay", "ill", "send", "the", "doc"]
    assert normalize("well-known “quote” — 50%") == ["well", "known", "quote", "50"]


def test_normalize_is_nfc():
    decomposed = "Mình chốt"  # combining marks
    assert normalize(decomposed) == ["mình", "chốt"]


def test_fold_strips_diacritics():
    assert fold("Công ty ĐẠT") == "cong ty dat"


def test_wer_counts_and_pooling():
    te = token_errors("the cat sat down", "the cat sat")
    assert (te.errors, te.ref_tokens, te.deletions) == (1, 4, 1)
    te = token_errors("Mình chốt scope cho beta nhé.", "mình chốt scope cho beta")
    assert (te.errors, te.ref_tokens) == (1, 6)
    assert token_errors("[unk] ...", "anything") is None  # nothing to score
    assert token_errors("a b", "").errors == 2
    # mixed code-switch string: english words count like syllables
    te = token_errors("deadline là thứ sáu nhé team", "deadline là thứ bảy nhé team")
    assert (te.errors, te.ref_tokens, te.substitutions) == (1, 6, 1)


def test_metric_per_language():
    assert [asr_metric_for(x) for x in ("en", "vi", "mixed")] == ["wer", "syl_wer", "mer"]


def test_der_hand_computed_with_collar():
    # ref A 0-10, B 10-20; hyp x 0-12, y 12-20. Collar 0.25 s each side of ref boundaries
    # removes [0,.25] [9.75,10.25] [19.75,20] = 1.0 s -> scored 19 s. The 2 s confusion
    # (10-12) loses 0.25 s to the collar -> 1.75.  DER = 1.75 / 19.
    ref = [T(0, 10, "A"), T(10, 20, "B")]
    hyp = [T(0, 12, "x"), T(12, 20, "y")]
    c = diarization_components(ref, hyp, 20.0)
    assert c["ref_speech"] == pytest.approx(19.0)
    assert c["confusion"] == pytest.approx(1.75)
    assert c["missed"] == pytest.approx(0.0) and c["false_alarm"] == pytest.approx(0.0)


def test_jer_hand_computed():
    # A vs x: 1 - 10/12; B vs y: 1 - 8/10  -> speaker error 0.3667 over 2 speakers
    ref = [T(0, 10, "A"), T(10, 20, "B")]
    hyp = [T(0, 12, "x"), T(12, 20, "y")]
    c = diarization_components(ref, hyp, 20.0)
    assert c["jer_error"] == pytest.approx(1 / 6 + 1 / 5)
    assert c["jer_speakers"] == 2


def test_overlap_is_scored():
    ref = [T(0, 10, "A"), T(5, 15, "B")]
    hyp = [T(0, 15, "x")]  # one speaker covering the overlap: B's share of 5-10 is missed
    c = diarization_components(ref, hyp, 15.0)
    assert c["ref_speech"] > 20 - 3  # 20 s of reference speech, minus the collars
    assert c["missed"] > 4


def test_empty_hypothesis():
    c = diarization_components([T(0, 10, "A")], [], 10.0)
    assert c["missed"] == pytest.approx(c["ref_speech"])
    assert c["jer_error"] == 1 and c["hyp_speakers"] == 0


def test_pooled_not_mean_of_files():
    f1 = {"diar": diarization_components([T(0, 10, "A"), T(10, 20, "B")], [T(0, 20, "x")], 20.0)}
    f2 = {"diar": diarization_components([T(0, 60, "A")], [T(0, 60, "x")], 60.0)}
    for f in (f1, f2):
        f["diar"]["spk_count_err"] = abs(f["diar"]["hyp_speakers"] - f["diar"]["ref_speakers"])
    assert f1["diar"]["confusion"] == pytest.approx(9.5) and f1["diar"][
        "ref_speech"
    ] == pytest.approx(19)
    pooled = aggregate([f1, f2])["der"]
    assert pooled == pytest.approx(9.5 / (19 + 59.5))
    assert pooled != pytest.approx((0.5 + 0.0) / 2)


def test_asr_pooling_by_tokens():
    files = [
        {"asr": {"metric": "wer", "errors": 1, "ref_tokens": 2}},
        {"asr": {"metric": "wer", "errors": 1, "ref_tokens": 98}},
        {"asr": {"metric": "syl_wer", "errors": 5, "ref_tokens": 10}},
    ]
    agg = aggregate(files)
    assert agg["wer"] == pytest.approx(2 / 100)
    assert agg["syl_wer"] == 0.5 and agg["mer"] is None


def test_lag_percentiles():
    assert percentile([1, 2, 3, 4, 5], 50) == 3
    assert percentile([], 95) is None
    agg = aggregate([{"lag": {"partial": [0.5, 1.0], "final": [1.0, 2.0, 3.0]}}])
    assert agg["final_lag_p50"] == 2.0
    assert agg["partial_lag_p50"] == pytest.approx(0.75)


def test_equal_error_rate():
    assert equal_error_rate([0.9, 0.8, 0.1, 0.2], [True, True, False, False]) == 0.0
    assert equal_error_rate([0.1, 0.2, 0.9, 0.8], [True, True, False, False]) == 1.0
    # one target below one non-target of two each: EER 0.5
    assert equal_error_rate([0.9, 0.3, 0.5, 0.1], [True, True, False, False]) == pytest.approx(0.5)
    assert equal_error_rate([1.0, 2.0], [True, True]) is None


def test_gate_status():
    g = {"max": 0.2, "floor": 0.25}
    assert [gate_status(v, g) for v in (0.1, 0.2, 0.22, 0.25, 0.3, None)] == [
        "pass", "pass", "best_effort", "best_effort", "fail", "n/a",
    ]  # fmt: skip
    assert gate_status(0.16, {"max": 0.15}) == "fail"


def test_collar_argument_is_plus_minus_width():
    ref = [T(0, 10, "A"), T(10, 20, "B")]
    hyp = [T(0, 12, "x"), T(12, 20, "y")]
    c0 = diarization_components(ref, hyp, 20.0, collar=0.0)
    assert (c0["ref_speech"], c0["confusion"]) == (pytest.approx(20.0), pytest.approx(2.0))
