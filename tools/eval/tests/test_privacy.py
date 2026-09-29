# SPDX-License-Identifier: Apache-2.0
"""Seeded leaks must fail the lint; a clean report must pass."""

import json

import pytest

from ghi_eval.cli import main
from ghi_eval.errors import PrivacyLeak
from ghi_eval.manifest import load_manifest
from ghi_eval.privacy import build_index, lint_document, lint_file, lint_text
from ghi_eval.report import write_report

CLEAN = {
    "schema": "ghi.eval-report/1",
    "dataset": {"name": "tiny-fixture"},
    "system": {"spec": "ghi"},
}


@pytest.fixture
def index(tiny):
    return build_index(load_manifest(tiny))


def leaks(doc, index):
    return {f["kind"] for f in lint_document(doc, index).findings}


def test_clean_report_passes(index):
    result = lint_document(CLEAN, index)
    assert result.passed and result.names_checked == 3  # Linh, Minh, Công ty ABC


def test_reference_trigram_fails(index):
    # 3 tokens from refs/t01.txt, different case and punctuation
    doc = {**CLEAN, "note": "and then: Sẽ gửi TÀI liệu, later"}
    assert "reference_ngram" in leaks(doc, index)
    # spanning a line break of the reference must not matter: 2 tokens of one line stay clean
    assert leaks({**CLEAN, "note": "sẽ gửi"}, index) == set()


def test_reference_note_ngram_fails(index):
    assert "reference_note_ngram" in leaks({**CLEAN, "x": "Beta chốt scope hiện tại"}, index)


@pytest.mark.parametrize(
    "text",
    [
        "Linh",  # single token
        "LINH said",  # case
        "minh",  # diacritics dropped from a name that has none in the manifest
        "Công ty ABC",  # multi token
        "cong ty abc",  # multi token without diacritics
        "CÔNG TY abc.",  # case + punctuation
    ],
)
def test_manifest_names_fail(index, text):
    assert "name" in leaks({**CLEAN, "x": text}, index)


def test_name_needs_whole_token(index):
    assert leaks({**CLEAN, "x": "Linhtinh Minhh"}, index) == set()


def test_file_ids_fail(index):
    assert "file_id" in leaks({**CLEAN, "x": "file t01 failed"}, index)


def test_numbers_do_not_trigger(index):
    assert lint_document({**CLEAN, "der": 0.182, "hours": 10.4}, index).passed


def test_markdown_lint(index):
    assert lint_text("| vi | room | 0.182 |\n", index).passed
    assert not lint_text("prose mình chốt scope cho beta\n", index).passed


def test_lint_report_command(tiny, tmp_path, capsys):
    good, bad = tmp_path / "good.json", tmp_path / "bad.json"
    good.write_text(json.dumps(CLEAN), encoding="utf-8")
    bad.write_text(json.dumps({**CLEAN, "x": "Công ty ABC"}), encoding="utf-8")
    assert main(["lint-report", "--report", str(good), "--dataset", str(tiny)]) == 0
    assert main(["lint-report", "--report", str(bad), "--dataset", str(tiny)]) == 1
    assert "name" in capsys.readouterr().err
    assert lint_file(good, load_manifest(tiny)).passed


def test_report_with_leak_is_not_written(tiny, tiny_hyp, tmp_path, monkeypatch):
    """A leak that reaches the report body makes write_report raise and write nothing."""
    from ghi_eval import report as report_mod

    out = tmp_path / "runs"
    assert (
        main(["run", "--dataset", str(tiny), "--system", f"files:{tiny_hyp}", "--run-id", "r",
              "--tasks", "asr", "--out", str(out)]) == 0
    )  # fmt: skip
    run_dir = out / "r"
    for f in run_dir.glob("report-*"):
        f.unlink()
    real = report_mod.build_report

    def leaky(*a, **k):
        rep, warnings = real(*a, **k)
        rep["dataset"]["name"] = "Công ty ABC"  # e.g. a dataset name containing a customer
        return rep, warnings

    monkeypatch.setattr(report_mod, "build_report", leaky)
    with pytest.raises(PrivacyLeak):
        write_report(run_dir)
    assert list(run_dir.glob("report-*")) == []
    assert main(["report", "--run", str(run_dir)]) == 1
