# SPDX-License-Identifier: Apache-2.0
import json
import shutil
from pathlib import Path

import pytest
import yaml
from conftest import FIXTURES, write_silence

from ghi_eval.cli import main
from ghi_eval.contract import validation_errors

DURATIONS = {"t01": 30.0, "t02": 20.0, "t03": 24.0}


@pytest.fixture
def dataset(tiny, tiny_hyp):
    """The tiny dataset with real (silent) WAVs and no duration_s in the manifest."""
    for fid, dur in DURATIONS.items():
        write_silence(tiny / "audio" / f"{fid}.wav", dur)
    doc = yaml.safe_load((tiny / "manifest.yaml").read_text(encoding="utf-8"))
    for f in doc["files"]:
        f.pop("duration_s")
    (tiny / "manifest.yaml").write_text(yaml.safe_dump(doc, allow_unicode=True), encoding="utf-8")
    return tiny


def events(final_lag, words=True):
    rows = []
    for i, t in enumerate(("partial", "final", "end")):
        row = {"schema": "ghi.event/1", "type": t, "seq": i, "wall_s": 3.0 + final_lag * (t != "partial"),
               "audio_start": 0.0, "audio_end": 3.0, "text": "", "lang": None, "speaker": None}  # fmt: skip
        if words and t == "final":  # two words: shown 0.5 s and final_lag after they ended
            row["words"] = [{"start": 0.0, "end": 1.0, "text": "a", "shown_s": 1.5},
                            {"start": 1.0, "end": 3.0, "text": "b", "shown_s": 3.0 + final_lag}]  # fmt: skip
        elif words:
            row["words"] = None
        rows.append(row)
    return "".join(json.dumps(r) + "\n" for r in rows)


def test_validate_reads_audio_durations(dataset, capsys):
    assert main(["validate", "--dataset", str(dataset)]) == 0
    out = capsys.readouterr().out
    assert "audio missing" not in out and "validate: ok" in out


def test_run_score_report(dataset, tiny_hyp, capsys):
    hyp = tiny_hyp / "hyp"
    (hyp / "t01.events.ndjson").write_text(events(1.0), encoding="utf-8")
    (hyp / "t02.events.ndjson").write_text(events(2.5), encoding="utf-8")
    code = main(["run", "--dataset", str(dataset), "--system", f"files:{tiny_hyp}", "--run-id", "e2e",
                 "--realtime", "--tasks", "asr,diar,notes,stream"])  # fmt: skip
    assert code == 0
    run_dir = dataset / "runs" / "e2e"
    report_path = next(run_dir.glob("report-files-*.json"))
    report = json.loads(report_path.read_text(encoding="utf-8"))
    assert validation_errors(report, "eval-report") == []
    assert next(run_dir.glob("report-files-*.md")).read_text(encoding="utf-8").startswith("# Ghira")

    assert report["dataset"]["files"] == 3
    assert report["dataset"]["hours"] == pytest.approx(74 / 3600, abs=0.005)
    assert report["overall"]["syl_wer"] == pytest.approx(1 / 22, abs=1e-4)  # t01 only
    assert report["overall"]["wer"] == pytest.approx(1 / 12, abs=1e-4)
    assert report["overall"]["mer"] == 0.0
    assert report["overall"]["final_lag_p95"] == pytest.approx(
        2.425
    )  # final lags [1.0, 2.5] pooled: 1.0 + 0.95 * 1.5
    slices = {(s["lang"], s["setting"], s["speakers"]): s for s in report["slices"]}
    assert set(slices) == {("vi", "room", "1-2"), ("en", "call", "1-2"), ("mixed", "room", "3-5")}
    assert slices[("mixed", "room", "3-5")]["spk_count_err"] == 3.0
    gates = {g["id"]: g for g in report["gates"]}
    assert gates["der_call"]["status"] == "pass"
    assert gates["der_overall"]["status"] == "fail"  # t03 has one hypothesis speaker for four
    assert gates["syl_wer_vn_room"]["status"] == "pass"
    assert gates["syl_wer_vn_call"]["status"] == "n/a"
    # caption lag: t01 words [0.5, 1.0], t02 words [0.5, 2.5]; en gate sees t02 only
    assert report["overall"]["caption_lag_p50"] == pytest.approx(0.75)
    assert report["overall"]["lag_files"] == 2
    assert gates["lag_en"]["metric"] == "caption_lag_p95"
    assert gates["lag_en"]["status"] == "fail" and gates["lag_en"]["value"] == pytest.approx(2.4)
    assert gates["lag_vn"]["status"] == "pass" and gates["lag_vn"]["value"] == pytest.approx(0.975)
    assert report["privacy_lint"] == {
        "passed": True,
        "ngram": 3,
        "names_checked": report["privacy_lint"]["names_checked"],
    }
    # the run directory keeps hypotheses, the report has no ids or text
    text = report_path.read_text(encoding="utf-8")
    for needle in ("t01", "t02", "Mình", "Linh"):
        assert needle not in text
    assert (run_dir / "hyp" / "t01.transcript.json").is_file()
    assert (run_dir / "scores.json").is_file()
    assert "files" in capsys.readouterr().out


def test_ci_smoke_command_with_out(tiny, tiny_hyp, tmp_path):
    out = tmp_path / "out"
    assert main(["run", "--dataset", str(tiny), "--system", f"files:{tiny_hyp}", "--run-id", "ci",
                 "--tasks", "asr,diar,notes", "--out", str(out)]) == 0  # fmt: skip
    assert len(list((out / "ci").glob("report-*.json"))) == 1
    assert not (tiny / "runs").exists()


def test_committed_fixture_is_not_modified_by_ci_command(tmp_path):
    ds, hyp = FIXTURES / "tiny", FIXTURES / "tiny-hyp"
    before = sorted(p.relative_to(FIXTURES) for p in FIXTURES.rglob("*"))
    assert main(["run", "--dataset", str(ds), "--system", f"files:{hyp}", "--run-id", "ci",
                 "--tasks", "asr,diar,notes", "--out", str(tmp_path)]) == 0  # fmt: skip
    assert sorted(p.relative_to(FIXTURES) for p in FIXTURES.rglob("*")) == before


def test_unsupported_task_is_skipped_not_a_crash(dataset, fake_ghi, monkeypatch, capsys):
    monkeypatch.setenv("FAKE_GHI_MODE", "notimpl")
    code = main(["run", "--dataset", str(dataset), "--system", f"ghi:{fake_ghi}", "--run-id", "ni"])
    assert code == 0
    err = capsys.readouterr().err
    assert "system does not support asr yet" in err
    run = yaml.safe_load((dataset / "runs" / "ni" / "run.yaml").read_text(encoding="utf-8"))
    assert {"asr", "diar"} <= set(run["unsupported"])
    report = json.loads(
        next((dataset / "runs" / "ni").glob("report-ghi-*.json")).read_text(encoding="utf-8")
    )
    assert report["overall"]["der"] is None and all(g["status"] == "n/a" for g in report["gates"])


def test_ghi_system_measures_perf(dataset, fake_ghi):
    code = main(["run", "--dataset", str(dataset), "--system", f"ghi:{fake_ghi}", "--run-id", "g",
                 "--tasks", "asr,diar,notes,stream", "--realtime", "--notes-input", "pipeline,gold"])  # fmt: skip
    assert code == 0
    run_dir = dataset / "runs" / "g"
    perf = json.loads((run_dir / "hyp" / "t01.perf.json").read_text(encoding="utf-8"))
    assert perf["asr"]["wall_s"] > 0 and perf["asr"]["audio_s"] == 30.0
    assert (run_dir / "hyp" / "t01.events.ndjson").is_file()
    assert (run_dir / "hyp" / "t01.notes-gold.json").is_file()
    assert (
        (run_dir / "hyp" / "t01.rttm").read_text(encoding="utf-8").startswith("SPEAKER t01 1 1.200")
    )
    report = json.loads(next(run_dir.glob("report-ghi-*.json")).read_text(encoding="utf-8"))
    assert report["overall"]["rtf"] is not None
    assert {e["source"] for e in report["llm"]} == {"pipeline", "gold"}
    assert report["system"]["spec"] == "ghi:" + Path(str(fake_ghi)).name


def test_score_and_report_rerun(dataset, tiny_hyp):
    main(["run", "--dataset", str(dataset), "--system", f"files:{tiny_hyp}", "--run-id", "r"])
    run_dir = dataset / "runs" / "r"
    assert main(["score", "--run", str(run_dir)]) == 0
    assert (
        main(
            [
                "report",
                "--run",
                str(run_dir),
                "--gates",
                str(FIXTURES.parent.parent / "ghi_eval" / "gates.yaml"),
            ]
        )
        == 0
    )
    shutil.rmtree(dataset / "runs")


def test_collar_flag_recorded(tiny, tiny_hyp, tmp_path):
    out = tmp_path / "o"
    base = [
        "--dataset",
        str(tiny),
        "--system",
        f"files:{tiny_hyp}",
        "--run-id",
        "c",
        "--tasks",
        "diar",
        "--out",
        str(out),
    ]
    assert main(["run", *base, "--collar", "0"]) == 0
    run_dir = out / "c"
    rep = json.loads(next(run_dir.glob("report-*.json")).read_text(encoding="utf-8"))
    assert rep["der_collar"] == 0.0
    assert "der_collar: 0.0" in next(run_dir.glob("report-*.md")).read_text(encoding="utf-8")
    assert main(["report", "--run", str(run_dir), "--collar", "0.25"]) == 0
    rep = json.loads(next(run_dir.glob("report-*.json")).read_text(encoding="utf-8"))
    assert rep["der_collar"] == 0.25


def test_events_without_words_still_score_commit_lag(dataset, tiny_hyp):
    """Older producers omit `words`: final/partial lag work, caption lag is null, gates n/a."""
    hyp = tiny_hyp / "hyp"
    (hyp / "t02.events.ndjson").write_text(events(2.5, words=False), encoding="utf-8")
    text = json.loads(events(1.0).splitlines()[1])
    text.pop("words")  # field missing entirely
    (hyp / "t01.events.ndjson").write_text(json.dumps(text) + "\n", encoding="utf-8")
    assert main(["run", "--dataset", str(dataset), "--system", f"files:{tiny_hyp}", "--run-id", "old",
                 "--realtime", "--tasks", "stream"]) == 0  # fmt: skip
    report = json.loads(
        next((dataset / "runs" / "old").glob("report-*.json")).read_text(encoding="utf-8")
    )
    o = report["overall"]
    assert o["final_lag_p95"] == pytest.approx(2.425) and o["caption_lag_p95"] is None
    assert o["lag_files"] == 0
    gates = {g["id"]: g["status"] for g in report["gates"]}
    # realtime run, files streamed, but no final event carries words: incomplete, not n/a
    assert gates["lag_en"] == "incomplete" and gates["lag_vn"] == "incomplete"
