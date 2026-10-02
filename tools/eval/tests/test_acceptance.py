# SPDX-License-Identifier: Apache-2.0
import json

import pytest
import yaml
from conftest import write_silence

from ghi_eval.acceptance import (
    RT7_NOTES_AFTER_STOP_S,
    parse_external,
    run_acceptance,
    timing_checks,
)
from ghi_eval.cli import main
from ghi_eval.errors import HarnessError
from ghi_eval.manifest import load_manifest

DURATIONS = {"t01": 30.0, "t02": 20.0, "t03": 24.0}


def events(final_lag):
    rows = []
    for i, t in enumerate(("partial", "final", "end")):
        row = {"schema": "ghi.event/1", "type": t, "seq": i, "wall_s": 3.0 + final_lag * (t != "partial"),
               "audio_start": 0.0, "audio_end": 3.0, "text": "", "lang": None, "speaker": None}  # fmt: skip
        if t == "final":
            row["words"] = [{"start": 0.0, "end": 1.0, "text": "a", "shown_s": 1.5},
                            {"start": 1.0, "end": 3.0, "text": "b", "shown_s": 3.0 + final_lag}]  # fmt: skip
        else:
            row["words"] = None
        rows.append(row)
    return "".join(json.dumps(r) + "\n" for r in rows)


@pytest.fixture
def dataset(tiny, tiny_hyp):
    for fid, dur in DURATIONS.items():
        write_silence(tiny / "audio" / f"{fid}.wav", dur)
    doc = yaml.safe_load((tiny / "manifest.yaml").read_text(encoding="utf-8"))
    for f in doc["files"]:
        f.pop("duration_s")
    (tiny / "manifest.yaml").write_text(yaml.safe_dump(doc, allow_unicode=True), encoding="utf-8")
    (tiny_hyp / "hyp" / "t01.events.ndjson").write_text(events(1.0), encoding="utf-8")
    (tiny_hyp / "hyp" / "t02.events.ndjson").write_text(events(2.5), encoding="utf-8")
    return tiny


def acc(dataset, system, tmp_path, **kw):
    return run_acceptance(
        [dataset], system, out=tmp_path / "out", stamp="s1", log=lambda m: None, **kw
    )


def test_files_system_routes_gates_and_writes_a_clean_report(dataset, tiny_hyp, tmp_path):
    report, json_path, md_path = acc(dataset, f"files:{tiny_hyp}", tmp_path)
    assert report["schema"] == "ghi.acceptance-report/1"
    gates = {g["id"]: g for g in report["gates"]}
    # DER and word errors come from the final run, lag from the live run
    assert gates["der_overall"]["run"] == "final" and gates["der_overall"]["status"] == "fail"
    assert (
        gates["syl_wer_vn_room"]["run"] == "final" and gates["syl_wer_vn_room"]["status"] == "pass"
    )
    assert gates["lag_en"]["run"] == "live" and gates["lag_en"]["status"] == "fail"
    assert gates["lag_vn"]["status"] == "pass"
    assert gates["syl_wer_vn_call"]["status"] == "n/a"
    assert report["verdict"] == "fail"
    assert report["summary"]["fail"] == 2
    # a files: system measures no wall time, so the RT-7 checks have no data
    assert {t["id"]: t["status"] for t in report["timing"]} == {
        "notes_after_stop": "no_data",
        "refined_notes": "no_data",
    }
    assert [e["status"] for e in report["external"]] == ["not_run"] * 3
    assert report["datasets"] == [{"name": "tiny-fixture", "files": 3, "hours": 0.02}]
    # the aggregate report carries no ids, names, reference text or paths
    text = json_path.read_text(encoding="utf-8") + md_path.read_text(encoding="utf-8")
    for needle in ("t01", "t02", "t03", "Linh", "Mình", str(tmp_path)):
        assert needle not in text
    assert report["privacy_lint"]["passed"] is True
    assert md_path.read_text(encoding="utf-8").startswith("# Ghira acceptance report")
    # each run also got its own standard report next to its hypotheses
    assert len(list((dataset / "runs" / "acc-s1-final").glob("report-*.json"))) == 1
    assert len(list((dataset / "runs" / "acc-s1-live").glob("report-*.json"))) == 1


def test_fake_ghi_timing_passes_and_best_effort_floor(dataset, fake_ghi, tmp_path):
    report, _, _ = acc(dataset, f"ghi:{fake_ghi}", tmp_path, external={"crash_safety": "pass"})
    timing = {t["id"]: t for t in report["timing"]}
    assert timing["notes_after_stop"]["status"] == "pass"
    assert timing["notes_after_stop"]["files"] == 3
    assert (
        timing["refined_notes"]["status"] == "pass" and timing["refined_notes"]["worst_ratio"] < 1
    )
    assert {e["id"]: e["status"] for e in report["external"]}["crash_safety"] == "pass"
    assert report["runs"]["live"]["tasks"] == ["asr", "diar", "stream", "notes"]


def test_cli_entry_exit_code_follows_the_verdict(dataset, tiny_hyp, tmp_path, capsys):
    out = tmp_path / "cli-out"
    code = main(["acceptance", "--dataset", str(dataset), "--system", f"files:{tiny_hyp}",
                 "--out", str(out), "--stamp", "c1"])  # fmt: skip
    assert code == 1  # DER and lag gates fail on the fixture
    assert "verdict: fail" in capsys.readouterr().out
    (path,) = out.glob("acceptance-*.json")
    # re-aggregate the finished runs without running anything
    code = main(["acceptance", "--dataset", str(dataset), "--system", f"files:{tiny_hyp}",
                 "--out", str(out), "--stamp", "c1", "--aggregate-only"])  # fmt: skip
    assert code == 1 and json.loads(path.read_text(encoding="utf-8"))["verdict"] == "fail"


def test_external_failure_and_validation(dataset, tiny_hyp, tmp_path):
    assert parse_external(["crash_safety=pass", "vn_search=fail"]) == {
        "crash_safety": "pass",
        "vn_search": "fail",
    }
    for bad in ("crash_safety", "nope=pass", "crash_safety=maybe"):
        with pytest.raises(HarnessError):
            parse_external([bad])


def _perf(run_dir, fid, **tasks):
    path = run_dir / "hyp" / f"{fid}.perf.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({k: {"wall_s": v, "audio_s": a} for k, (v, a) in tasks.items()}))


def test_timing_checks_scale_with_duration(tiny, tmp_path):
    manifest = load_manifest(tiny)
    live, final = tmp_path / "live", tmp_path / "final"
    # notes 100 s after stop: fine; refined: 60 min audio in 9 min total
    _perf(live, "t01", notes=(100.0, None))
    _perf(final, "t01", asr=(300.0, 3600.0), diar=(120.0, 3600.0), notes=(120.0, None))
    ok = {t["id"]: t for t in timing_checks([(live, manifest)], [(final, manifest)])}
    assert ok["notes_after_stop"]["status"] == "pass" and ok["notes_after_stop"]["max_s"] == 100.0
    assert ok["refined_notes"]["status"] == "pass"
    assert ok["refined_notes"]["worst_ratio"] == pytest.approx(540 / 600, abs=1e-3)
    assert ok["refined_notes"]["longest_audio_min"] == 60.0

    # 30 min of audio gets 5 min: 6 min fails, though 6 min would pass for an hour
    _perf(live, "t02", notes=(RT7_NOTES_AFTER_STOP_S + 1, None))
    _perf(final, "t02", asr=(200.0, 1800.0), diar=(100.0, 1800.0), notes=(60.0, None))
    bad = {t["id"]: t for t in timing_checks([(live, manifest)], [(final, manifest)])}
    assert bad["notes_after_stop"]["status"] == "fail"
    assert bad["refined_notes"]["status"] == "fail"
    assert bad["refined_notes"]["worst_ratio"] == pytest.approx(360 / 300, abs=1e-3)


def test_verdict_rules():
    from ghi_eval.acceptance import _verdict

    def counts(**kw):
        base = {"pass": 0, "best_effort": 0, "fail": 0, "incomplete": 0, "n/a": 0}
        return {**base, **kw}

    assert _verdict(counts(**{"pass": 7}), False, 0, False, False) == "pass"
    assert (
        _verdict(counts(**{"pass": 6, "best_effort": 1}), False, 0, False, False)
        == "pass_best_effort"
    )
    assert _verdict(counts(**{"pass": 6, "fail": 1}), True, 0, False, False) == "fail"
    assert _verdict(counts(**{"pass": 6, "incomplete": 1}), False, 0, False, False) == "incomplete"
    assert _verdict(counts(**{"pass": 6}), False, 1, False, False) == "incomplete"  # run errors
    assert (
        _verdict(counts(**{"pass": 6}), False, 0, True, False) == "incomplete"
    )  # unsupported task
    # gates without data: incomplete for a customer set, pass_partial for public sets
    assert _verdict(counts(**{"pass": 3, "n/a": 4}), False, 0, False, False) == "incomplete"
    assert _verdict(counts(**{"pass": 3, "n/a": 4}), False, 0, False, True) == "pass_partial"
    assert _verdict(counts(**{"n/a": 7}), False, 0, False, True) == "incomplete"  # nothing measured
