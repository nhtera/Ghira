# SPDX-License-Identifier: Apache-2.0
"""Regression tests for the code-review fixes: privacy of specs, coverage, lint scope, sheets."""

import json
import os
import shutil
import time

import pytest
import yaml
from conftest import FIXTURES, write_silence

from ghi_eval.adapters import ghi_cli, make_adapter
from ghi_eval.adapters.whisper_ref import WhisperRefAdapter
from ghi_eval.cli import main
from ghi_eval.errors import AdapterError, HarnessError
from ghi_eval.llm import JUDGEMENT_COLUMNS, read_judgements, write_judgements
from ghi_eval.manifest import load_manifest, validate
from ghi_eval.metrics import diarization_components
from ghi_eval.privacy import build_index, lint_document, lint_text
from ghi_eval.report import evaluate_gates, public_spec


def run_cli(tiny, tiny_hyp, out, *extra, run_id="x", tasks="asr,diar,notes"):
    return main(["run", "--dataset", str(tiny), "--system", f"files:{tiny_hyp}", "--run-id", run_id,
                 "--tasks", tasks, "--out", str(out), *extra])  # fmt: skip


def load_report(run_dir):
    return json.loads(next(run_dir.glob("report-*.json")).read_text(encoding="utf-8"))


# ------------------------------------------------------------------ H1 privacy of specs and paths
@pytest.mark.parametrize(
    ("spec", "expected"),
    [
        ("ghi", "ghi"),
        ("ghi:/Users/alice/bin/ghi", "ghi:ghi"),
        ("ghi:C:\\Users\\alice\\ghi.exe", "ghi:ghi.exe"),
        ("nemo_ref:/home/bob/cfg.yaml", "nemo-ref:cfg.yaml"),
        ("whisper-ref:/models/customer-x/large", "whisper-ref:large"),
        ("whisper_ref:Systran/faster-whisper-small", "whisper-ref:faster-whisper-small"),
        ("files:/Volumes/customer/hyp", "files"),
    ],
)
def test_public_spec_has_no_paths(spec, expected):
    assert public_spec(spec) == expected


def test_whisper_version_is_basename():
    v = WhisperRefAdapter(
        "whisper-ref:/private/customer/model", "/private/customer/model"
    ).version()
    assert "customer" not in v and "/" not in v and v.endswith("model")


def test_unreadable_adapter_version_is_unknown(tiny, tiny_hyp, tmp_path, monkeypatch):
    from ghi_eval.adapters.files import FilesAdapter

    def boom(self):
        raise HarnessError("/secret/path/thing")

    monkeypatch.setattr(FilesAdapter, "version", boom)
    run_cli(tiny, tiny_hyp, tmp_path)
    run = yaml.safe_load((tmp_path / "x" / "run.yaml").read_text(encoding="utf-8"))
    assert run["system"]["version"] == "unknown"


@pytest.fixture
def index(tiny):
    return build_index(load_manifest(tiny))


@pytest.mark.parametrize("bad", ["/Users/alice/x", "C:\\Users\\alice", "C:", "a\\b", "dir/file"])
def test_path_like_strings_fail_lint(index, bad):
    doc = {"schema": "ghi.eval-report/1", "system": {"spec": bad, "version": "1"}}
    assert "path" in {f["kind"] for f in lint_document(doc, index).findings}
    assert "path" in {f["kind"] for f in lint_text(f"- system: {bad}\n", index).findings}


def test_clean_strings_pass_path_rule(index):
    doc = {"schema": "ghi.eval-report/1", "generated": "2026-09-29T14:24:25Z",
           "system": {"spec": "ghi:ghi.exe", "version": "asr=parakeet"},
           "gates": [{"status": "n/a", "metric": "der"}]}  # fmt: skip
    assert lint_document(doc, index).passed
    assert lint_text("| n/a | der |\n- system: ghi:ghi.exe 0.1.0\n", index).passed


# ------------------------------------------------------------------ H2 coverage
def test_coverage_keys_and_run_errors(tiny, tiny_hyp, tmp_path):
    assert run_cli(tiny, tiny_hyp, tmp_path) == 0
    rep = load_report(tmp_path / "x")
    assert rep["run"]["errors"] == 0
    for block in (rep["overall"], *rep["slices"]):
        assert {"asr_files", "diar_files", "lag_files", "notes_files"} <= set(block)
    assert rep["overall"]["asr_files"] == 3 and rep["overall"]["diar_files"] == 3
    assert rep["overall"]["lag_files"] == 0 and rep["overall"]["notes_files"] == 1


def test_missing_hypothesis_makes_gate_incomplete(tiny, tiny_hyp, tmp_path):
    (tiny_hyp / "hyp" / "t02.rttm").unlink()  # t02 has a reference, no hypothesis
    assert run_cli(tiny, tiny_hyp, tmp_path) == 0
    rep = load_report(tmp_path / "x")
    assert rep["overall"]["diar_files"] == 2
    gates = {g["id"]: g for g in rep["gates"]}
    assert gates["der_overall"]["status"] == "incomplete"
    assert gates["der_call"]["status"] == "n/a"  # the only call file has no hypothesis at all
    assert gates["der_vn_room"]["status"] == "pass"  # complete slice unaffected
    (tiny_hyp / "hyp" / "t02.transcript.json").write_text("{}", encoding="utf-8")  # invalid
    assert (
        main(
            [
                "run",
                "--dataset",
                str(tiny),
                "--system",
                f"files:{tiny_hyp}",
                "--run-id",
                "y",
                "--tasks",
                "asr",
                "--out",
                str(tmp_path),
            ]
        )
        == 1
    )  # fmt: skip (invalid output is an error)
    rep = load_report(tmp_path / "y")
    assert rep["overall"]["asr_files"] == 2 and rep["run"]["errors"] == 1
    assert {g["id"]: g for g in rep["gates"]}["syl_wer_vn_room"]["status"] == "pass"


def test_reference_without_any_hypothesis_is_na_not_incomplete():
    files = [
        {"lang": "vi", "setting": "room", "speakers": "1-2", "expects": {"asr": True, "diar": True}}
    ]
    gates = [{"id": "g", "metric": "der", "slice": {}, "max": 0.1}]
    run = {"pass": "final", "realtime": False, "tasks": ["diar"]}
    assert evaluate_gates(files, gates, run)[0]["status"] == "n/a"


def test_gates_respect_pass_and_realtime():
    rec = {"lang": "vi", "setting": "room", "speakers": "1-2", "expects": {"asr": True, "diar": False},
           "asr": {"metric": "syl_wer", "errors": 0, "ref_tokens": 10},
           "lag": {"partial": [1.0], "final": [1.0]}}  # fmt: skip
    gates = [{"id": "w", "metric": "syl_wer", "slice": {}, "max": 0.1},
             {"id": "l", "metric": "final_lag_p95", "slice": {}, "max": 2.0}]  # fmt: skip
    base = {"pass": "final", "realtime": True, "tasks": ["asr", "stream"]}
    assert [g["status"] for g in evaluate_gates([rec], gates, base)] == ["pass", "pass"]
    live = {**base, "pass": "live", "realtime": False}
    assert [g["status"] for g in evaluate_gates([rec], gates, live)] == ["n/a", "n/a"]


def test_notes_denominator_counts_failed_outputs(tiny, tiny_hyp, tmp_path, fake_ghi, monkeypatch):
    # notes task fails for every file: outputs missing + errors recorded -> invalid, not dropped
    out = tmp_path / "o"
    ds = tiny
    write_silence(ds / "audio" / "t01.wav", 30.0)
    write_silence(ds / "audio" / "t02.wav", 20.0)
    write_silence(ds / "audio" / "t03.wav", 24.0)
    assert main(["run", "--dataset", str(ds), "--system", f"ghi:{fake_ghi}", "--run-id", "n",
                 "--tasks", "asr", "--out", str(out)]) == 0  # fmt: skip
    monkeypatch.setenv("FAKE_GHI_MODE", "fail")
    main(["run", "--dataset", str(ds), "--system", f"ghi:{fake_ghi}", "--run-id", "n2",
          "--tasks", "notes", "--out", str(out)])  # fmt: skip
    # no transcripts in n2 -> notes never ran for those files -> nothing to count
    assert load_report(out / "n2")["llm"] == []
    monkeypatch.setenv("FAKE_GHI_MODE", "ok")
    assert main(["run", "--dataset", str(ds), "--system", f"ghi:{fake_ghi}", "--run-id", "n3",
                 "--tasks", "asr,notes", "--out", str(out)]) == 0  # fmt: skip
    assert load_report(out / "n3")["llm"][0]["schema_valid"] == 1.0
    # now the notes call itself fails after transcripts exist
    run_dir = out / "n3"
    for f in (run_dir / "hyp").glob("*.notes.json"):
        f.unlink()
    doc = yaml.safe_load((run_dir / "run.yaml").read_text(encoding="utf-8"))
    doc["errors"] = [{"file": "t01", "task": "notes", "source": "pipeline", "error": "boom"}]
    (run_dir / "run.yaml").write_text(yaml.safe_dump(doc), encoding="utf-8")
    assert main(["report", "--run", str(run_dir)]) == 0
    entry = load_report(run_dir)["llm"][0]
    assert entry["files"] == 1 and entry["schema_valid"] == 0.0


# ------------------------------------------------------------------ M1 empty reference RTTM
def test_empty_reference_rttm(tiny):
    c = diarization_components([], [], 10.0)
    assert c["jer_speakers"] == 0 and c["ref_speakers"] == 0
    (tiny / "labels" / "t01.rttm").write_text("", encoding="utf-8")
    errors, _, _ = validate(load_manifest(tiny))
    assert any("no turns" in e for e in errors)


# ------------------------------------------------------------------ M2 lint scope
def test_names_and_ids_do_not_hit_fixed_vocabulary(tiny, tiny_hyp, tmp_path):
    text = (tiny / "manifest.yaml").read_text(encoding="utf-8")
    text = text.replace('names: [Linh, Minh, "Công ty ABC"]', "names: [Vi, Pass, Mixed, Eer]")
    (tiny / "manifest.yaml").write_text(text, encoding="utf-8")
    # a file id equal to a setting value
    for sub in ("labels/{}.rttm", "refs/{}.txt"):
        os.rename(tiny / sub.format("t03"), tiny / sub.format("room"))
    (tiny / "manifest.yaml").write_text(
        (tiny / "manifest.yaml").read_text(encoding="utf-8").replace("t03", "room"),
        encoding="utf-8",
    )
    (tiny / "labels" / "room.rttm").write_text(
        (tiny / "labels" / "room.rttm").read_text(encoding="utf-8").replace("t03", "room"),
        encoding="utf-8",
    )
    for suffix in ("transcript.json", "rttm"):
        os.rename(tiny_hyp / "hyp" / f"t03.{suffix}", tiny_hyp / "hyp" / f"room.{suffix}")
    (tiny_hyp / "hyp" / "room.rttm").write_text(
        (tiny_hyp / "hyp" / "room.rttm").read_text(encoding="utf-8").replace("t03", "room"),
        encoding="utf-8",
    )
    assert run_cli(tiny, tiny_hyp, tmp_path) == 0
    assert list((tmp_path / "x").glob("report-*.md"))


def test_names_still_caught_in_free_text_fields(tiny):
    idx = build_index(load_manifest(tiny))
    doc = {"schema": "ghi.eval-report/1", "dataset": {"name": "Acme Minh"}}
    assert "name" in {f["kind"] for f in lint_document(doc, idx).findings}
    assert "name" in {f["kind"] for f in lint_text("- dataset: Acme Minh, 3 files\n", idx).findings}
    assert "name" in {f["kind"] for f in lint_text("| linh_gate | der |\n", idx).findings}


def test_ngram_check_still_covers_fixed_fields(tiny):
    idx = build_index(load_manifest(tiny))
    doc = {"schema": "sẽ gửi tài liệu"}
    assert "reference_ngram" in {f["kind"] for f in lint_document(doc, idx).findings}


# ------------------------------------------------------------------ M3 judgements sheet
def test_sheet_neutralizes_formulas_and_round_trips(tmp_path):
    row = dict.fromkeys(JUDGEMENT_COLUMNS, "")
    row.update(file="f", source="pipeline", sys_idx=0, sys_text="=HYPERLINK(\"http://x\")",
               cited_text="+1 -2", sys_owner="@bob", ref_text="\tTab")  # fmt: skip
    path = tmp_path / "j.csv"
    write_judgements(path, [row])
    raw = path.read_bytes()
    assert raw.startswith(b"\xef\xbb\xbf")
    text = raw.decode("utf-8-sig")
    assert "'=HYPERLINK" in text and "'+1 -2" in text and "'@bob" in text
    back = read_judgements(path)[0]
    assert back["sys_text"] == row["sys_text"] and back["sys_owner"] == "@bob"


def test_sheet_delimiters_and_bad_encoding(tmp_path):
    cols = [c for c in JUDGEMENT_COLUMNS]
    line = ["f", "pipeline", "0"] + [""] * (len(cols) - 3)
    for delim in (",", ";", "\t"):
        p = tmp_path / "s.csv"
        p.write_text(delim.join(cols) + "\n" + delim.join(line) + "\n", encoding="utf-8-sig")
        assert read_judgements(p)[0]["file"] == "f"
    p.write_bytes("file,source\nMình,x\n".encode("utf-16"))
    with pytest.raises(HarnessError, match="CSV UTF-8"):
        read_judgements(p)
    p.write_bytes(b"file,source\n\xff\xfe,x\n")
    with pytest.raises(HarnessError, match="CSV UTF-8"):
        read_judgements(p)


# ------------------------------------------------------------------ M6 and low items
def test_hf_telemetry_disabled_before_import(monkeypatch, tmp_path):
    monkeypatch.delenv("HF_HUB_DISABLE_TELEMETRY", raising=False)
    monkeypatch.delenv("DO_NOT_TRACK", raising=False)
    wav = tmp_path / "a.wav"
    write_silence(wav, 1.0)
    with pytest.raises(HarnessError):  # extras not installed here
        make_adapter("whisper-ref").transcribe("a", wav, lang="auto", pass_="final")
    assert os.environ["HF_HUB_DISABLE_TELEMETRY"] == "1" and os.environ["DO_NOT_TRACK"] == "1"


def test_low_severity_wrapping(tmp_path, tiny, tiny_hyp):
    from ghi_eval.adapters.files import FilesAdapter
    from ghi_eval.adapters.nemo_ref import NemoRefAdapter
    from ghi_eval.convert import parse_eaf

    eaf = tmp_path / "x.eaf"
    eaf.write_text(
        '<ANNOTATION_DOCUMENT><TIME_ORDER><TIME_SLOT TIME_SLOT_ID="a" TIME_VALUE="1.5"/></TIME_ORDER></ANNOTATION_DOCUMENT>',
        encoding="utf-8",
    )
    with pytest.raises(HarnessError, match="TIME_VALUE"):
        parse_eaf(eaf)
    (tiny_hyp / "hyp" / "t01.events.ndjson").write_text("{not json}\n", encoding="utf-8")
    with pytest.raises(AdapterError):
        FilesAdapter("files:x", str(tiny_hyp)).transcribe_stream(
            "t01", tmp_path / "a.wav", lang="auto", realtime=True
        )
    cfg = tmp_path / "c.yaml"
    cfg.write_text("- a\n- b\n", encoding="utf-8")
    with pytest.raises(HarnessError, match="mapping"):
        NemoRefAdapter("nemo-ref", str(cfg))


def test_ghi_unstartable_binary(tmp_path, monkeypatch):
    bad = tmp_path / "ghi"
    bad.write_text("not an executable format", encoding="utf-8")
    bad.chmod(0o755)
    monkeypatch.setattr(ghi_cli, "REQUIRE_EXE", False)
    with pytest.raises(AdapterError, match="cannot start"):
        make_adapter(f"ghi:{bad}").transcribe("a", tmp_path / "a.wav", lang="auto", pass_="final")


def test_windows_requires_exe(fake_ghi, monkeypatch):
    monkeypatch.setattr(ghi_cli, "REQUIRE_EXE", True)
    with pytest.raises(AdapterError, match=r"\.exe"):
        make_adapter(f"ghi:{fake_ghi}")


def test_timeout_kills_the_process(fake_ghi, monkeypatch, tmp_path):
    monkeypatch.setenv("FAKE_GHI_MODE", "hang")
    adapter = make_adapter(f"ghi:{fake_ghi}")
    adapter.timeout = 0.5
    t0 = time.perf_counter()
    with pytest.raises(AdapterError, match="timed out"):
        adapter.diarize("a", tmp_path / "a.wav", pass_="final")
    assert time.perf_counter() - t0 < 10


def test_run_timeout_is_recorded_as_error(tiny, tmp_path, fake_ghi, monkeypatch):
    for fid, dur in (("t01", 30.0), ("t02", 20.0), ("t03", 24.0)):
        write_silence(tiny / "audio" / f"{fid}.wav", dur)
    monkeypatch.setenv("FAKE_GHI_MODE", "hang")
    code = main(["run", "--dataset", str(tiny), "--system", f"ghi:{fake_ghi}", "--run-id", "t",
                 "--tasks", "diar", "--timeout", "0.3", "--out", str(tmp_path)])  # fmt: skip
    assert code == 1
    run = yaml.safe_load((tmp_path / "t" / "run.yaml").read_text(encoding="utf-8"))
    assert len(run["errors"]) == 3 and "timed out" in run["errors"][0]["error"]
    assert load_report(tmp_path / "t")["run"]["errors"] == 3


def test_ids_are_validated(tiny, tiny_hyp, tmp_path):
    assert run_cli(tiny, tiny_hyp, tmp_path, run_id="../evil") == 1
    assert not (tmp_path.parent / "evil").exists()
    eaf = tmp_path / "bad name.eaf"
    shutil.copy(FIXTURES / "tiny" / "manifest.yaml", eaf)
    assert main(["convert", "eaf", str(eaf), "--dataset", str(tmp_path / "ds")]) == 1
    assert main(["convert", "audacity", str(eaf), "--rttm", str(tmp_path / "x.rttm")]) == 1


def test_report_filename_has_pass(tiny, tiny_hyp, tmp_path):
    assert run_cli(tiny, tiny_hyp, tmp_path, "--pass", "live") == 0
    names = [p.name for p in (tmp_path / "x").glob("report-*.json")]
    assert len(names) == 1 and names[0].startswith("report-files-live-")
    rep = load_report(tmp_path / "x")
    assert {g["id"]: g["status"] for g in rep["gates"]}["syl_wer_vn_room"] == "n/a"


def test_dead_code_removed():
    import ghi_eval.metrics as m
    from ghi_eval.manifest import Manifest

    assert not hasattr(m, "pooled_rate") and not hasattr(m, "DER_COLLAR_TOTAL")
    assert not hasattr(Manifest, "get")


def test_lag_gate_incomplete_only_when_stream_ran():
    files = [
        {
            "lang": "en",
            "setting": "call",
            "speakers": "1-2",
            "expects": {"asr": False, "diar": False},
        }
    ]
    gates = [{"id": "l", "metric": "caption_lag_p95", "slice": {}, "max": 2.0}]
    run = {"pass": "final", "realtime": True, "tasks": ["stream"]}
    assert evaluate_gates(files, gates, run)[0]["status"] == "incomplete"
    assert (
        evaluate_gates(files, gates, {**run, "unsupported": {"stream": "not implemented"}})[0][
            "status"
        ]
        == "n/a"
    )
    assert evaluate_gates(files, gates, {**run, "realtime": False})[0]["status"] == "n/a"
