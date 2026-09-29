# SPDX-License-Identifier: Apache-2.0
"""nemo-ref and whisper-ref against fake modules: the real ones are optional extras."""

import subprocess
import sys
import types

import pytest
import yaml
from conftest import write_silence

from ghi_eval.adapters import make_adapter
from ghi_eval.contract import validation_errors
from ghi_eval.errors import HarnessError, NotSupported


def test_optional_extras_not_imported_at_import_time():
    code = "import sys, ghi_eval.cli, ghi_eval.adapters.nemo_ref, ghi_eval.adapters.whisper_ref; assert 'nemo' not in sys.modules and 'faster_whisper' not in sys.modules"
    subprocess.run([sys.executable, "-c", code], check=True)


def test_missing_extras_give_actionable_errors(tmp_path, monkeypatch):
    monkeypatch.setitem(sys.modules, "faster_whisper", None)  # forces ImportError
    monkeypatch.setitem(sys.modules, "nemo.collections.asr", None)
    wav = tmp_path / "a.wav"
    write_silence(wav, 1.0)
    with pytest.raises(HarnessError, match="uv sync --extra whisper"):
        make_adapter("whisper-ref").transcribe("a", wav, lang="auto", pass_="final")
    with pytest.raises(HarnessError, match="uv sync --extra nemo"):
        make_adapter("nemo-ref").transcribe("a", wav, lang="auto", pass_="final")


def test_whisper_ref_with_fake_module(tmp_path, monkeypatch):
    calls = {}

    class Seg:
        def __init__(self, start, end, text):
            self.start, self.end, self.text = start, end, text

    class Model:
        def __init__(self, name, **kw):
            calls["model"] = (name, kw)

        def transcribe(self, path, **kw):
            calls["kw"] = kw
            info = types.SimpleNamespace(language="vi")
            return (s for s in [Seg(0.0, 1.5, " Xin chào "), Seg(1.5, 2.0, "beta")]), info

    monkeypatch.setitem(sys.modules, "faster_whisper", types.SimpleNamespace(WhisperModel=Model))
    wav = tmp_path / "a.wav"
    write_silence(wav, 2.0)
    adapter = make_adapter("whisper-ref:small")
    res = adapter.transcribe("a", wav, lang="vi", pass_="final")
    assert calls["model"][0] == "small" and calls["kw"]["language"] == "vi"
    assert validation_errors(res.value, "transcript") == []
    assert [s["text"] for s in res.value["segments"]] == ["Xin chào", "beta"]
    assert res.value["segments"][0]["lang"] == "vi" and res.value["duration_s"] == pytest.approx(
        2.0
    )
    with pytest.raises(NotSupported):
        adapter.diarize("a", wav, pass_="final")
    with pytest.raises(NotSupported):
        adapter.notes("a", wav, lang="auto")


def install_fake_nemo(monkeypatch, record):
    class Hyp:
        def __init__(self):
            self.text = "hello world"
            self.timestamp = {
                "segment": [
                    {"start": 0.0, "end": 1.0, "segment": "hello"},
                    {"start": 1.0, "end": 2.0, "segment": "world"},
                ]
            }

    class Base:
        def __init__(self, name):
            self.name = name

        def to(self, device):
            record.setdefault("device", device)
            return self

        def eval(self):
            return self

    class ASRModel:
        @staticmethod
        def from_pretrained(model_name):
            record.setdefault("asr", []).append(model_name)
            m = Base(model_name)
            m.transcribe = lambda paths, timestamps=False: (
                record.setdefault("paths", paths) and [Hyp()]
            )
            return m

    class Sortformer:
        @staticmethod
        def from_pretrained(name):
            record["diar"] = name
            m = Base(name)
            m.diarize = lambda audio, batch_size=1: [["0.00 1.50 speaker_0", "1.50 2.00 speaker_1"]]
            return m

    asr = types.ModuleType("nemo.collections.asr")
    asr.models = types.SimpleNamespace(ASRModel=ASRModel, SortformerEncLabelModel=Sortformer)
    collections = types.ModuleType("nemo.collections")
    collections.asr = asr
    nemo = types.ModuleType("nemo")
    nemo.collections = collections
    for name, mod in (
        ("nemo", nemo),
        ("nemo.collections", collections),
        ("nemo.collections.asr", asr),
    ):
        monkeypatch.setitem(sys.modules, name, mod)


def test_nemo_ref_with_fake_module(tmp_path, monkeypatch):
    record = {}
    install_fake_nemo(monkeypatch, record)
    cfg = tmp_path / "cfg.yaml"
    cfg.write_text(
        yaml.safe_dump({"asr_model_vi": "my/vi-model", "device": "cpu"}), encoding="utf-8"
    )
    wav = tmp_path / "stereo44k.wav"
    write_silence(wav, 2.0, rate=44100)
    adapter = make_adapter(f"nemo_ref:{cfg}")
    res = adapter.transcribe("a", wav, lang="vi", pass_="live")
    assert record["asr"] == ["my/vi-model"]
    assert validation_errors(res.value, "transcript") == []
    assert [s["text"] for s in res.value["segments"]] == ["hello", "world"]
    assert res.value["pass"] == "live" and res.value["duration_s"] == pytest.approx(2.0)
    d = adapter.diarize("a", wav, pass_="final")
    assert validation_errors(d.value, "diarization") == []
    assert [t["speaker"] for t in d.value["turns"]] == ["speaker_0", "speaker_1"]
    assert record["diar"] == "nvidia/Nemotron-3-Diarization"
    with pytest.raises(NotSupported):
        adapter.transcribe_stream("a", wav, lang="auto", realtime=True)
