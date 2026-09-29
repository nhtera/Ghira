# SPDX-License-Identifier: Apache-2.0
import json

import pytest

from ghi_eval.cli import main
from ghi_eval.llm import (
    JUDGEMENT_COLUMNS,
    gold_transcript,
    judgement_metrics,
    match_items,
    read_judgements,
    token_f1,
    write_judgements,
)
from ghi_eval.manifest import load_manifest


def test_token_f1_and_matching():
    assert token_f1("Send the doc", "send the DOC!") == 1.0
    assert token_f1("a b", "c d") == 0.0
    sys_items = ["Gửi tài liệu scope hôm nay", "Book a meeting room", "Gửi scope"]
    ref_items = ["Gửi tài liệu scope", "Order lunch"]
    m = match_items(sys_items, ref_items)
    assert m == {0: 0}  # greedy one-to-one; item 2 also overlaps but the ref is taken


def test_gold_transcript_one_segment_per_line(tiny):
    entry = next(f for f in load_manifest(tiny).files if f.id == "t01")
    doc = gold_transcript(entry, 30.0)
    assert [s["id"] for s in doc["segments"]] == [0, 1, 2]
    assert all(s["start"] == 0.0 == s["end"] for s in doc["segments"])
    from ghi_eval.contract import validation_errors

    assert validation_errors(doc, "transcript") == []


@pytest.fixture
def notes_run(tiny, tiny_hyp, tmp_path):
    out = tmp_path / "out"
    assert (
        main(
            [
                "run",
                "--dataset",
                str(tiny),
                "--system",
                f"files:{tiny_hyp}",
                "--run-id",
                "j",
                "--tasks",
                "asr,notes",
                "--out",
                str(out),
            ]
        )
        == 0
    )
    return out / "j"


def test_judge_writes_sheet_and_metrics_need_filling(tiny, notes_run, capsys):
    assert main(["judge", "--run", str(notes_run)]) == 0
    rows = read_judgements(notes_run / "judgements.csv")
    assert list(rows[0]) == JUDGEMENT_COLUMNS
    by_sys = {r["sys_idx"]: r for r in rows if r["sys_idx"] != ""}
    assert by_sys["0"]["ref_idx"] == "0" and by_sys["1"]["ref_idx"] == ""
    assert by_sys["0"]["sys_citations"] == "1" and by_sys["0"]["cited_text"].startswith("Ok Linh")
    assert main(["judge", "--run", str(notes_run)]) == 1  # will not overwrite
    # report before filling: judgement metrics stay null and a warning says why
    assert main(["report", "--run", str(notes_run)]) == 0
    assert "not judged yet" in capsys.readouterr().err
    report = json.loads(next(notes_run.glob("report-*.json")).read_text(encoding="utf-8"))
    assert report["llm"][0]["schema_valid"] == 1.0 and report["llm"][0]["precision"] is None
    # fill it in
    for r in rows:
        if r["sys_idx"] != "":
            r.update(
                citation_ok="y" if r["sys_idx"] == "0" else "na",
                hallucinated="n" if r["sys_idx"] == "0" else "y",
            )
            if r["ref_idx"] != "":
                r["owner_ok"] = "y"
    write_judgements(notes_run / "judgements.csv", rows)
    assert main(["report", "--run", str(notes_run)]) == 0
    entry = json.loads(next(notes_run.glob("report-*.json")).read_text(encoding="utf-8"))["llm"][0]
    assert entry == {
        "source": "pipeline", "files": 1, "schema_valid": 1.0, "precision": 0.5, "recall": 1.0,
        "owner_acc": 1.0, "citation_valid": 1.0, "hallucinations": 1,
    }  # fmt: skip


def test_judgement_metrics_per_source():
    def row(source, sys_idx, ref_idx, owner_ok="", cit="y", hal="n", file="f"):
        return {"file": file, "source": source, "sys_idx": sys_idx, "sys_text": "", "sys_owner": "",
                "ref_idx": ref_idx, "ref_text": "", "ref_owner": "", "owner_ok": owner_ok,
                "citation_ok": cit, "hallucinated": hal}  # fmt: skip

    rows = [
        row("pipeline", "0", "0", "y"), row("pipeline", "1", "", cit="n", hal="y"),
        row("pipeline", "", "1"),
        row("gold", "0", "0", "n", cit="na"),
    ]  # fmt: skip
    m, problem = judgement_metrics(rows, "pipeline")
    assert problem is None
    assert (
        m["precision"],
        m["recall"],
        m["owner_acc"],
        m["citation_valid"],
        m["hallucinations"],
    ) == (0.5, 0.5, 1.0, 0.5, 1)
    m, _ = judgement_metrics(rows, "gold")
    assert m["owner_acc"] == 0.0 and m["citation_valid"] is None
