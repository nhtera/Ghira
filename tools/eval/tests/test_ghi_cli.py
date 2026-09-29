# SPDX-License-Identifier: Apache-2.0
import json
import os
import shutil
import subprocess

import pytest

from ghi_eval.adapters import make_adapter
from ghi_eval.adapters.ghi_cli import GhiCliAdapter, parse_error_line
from ghi_eval.errors import AdapterError, InvalidOutput, NotSupported


@pytest.fixture
def adapter(fake_ghi):
    return make_adapter(f"ghi:{fake_ghi}")


def test_version_and_documents(adapter, tmp_path):
    assert isinstance(adapter, GhiCliAdapter)
    assert adapter.version() == "0.1.0"
    audio = tmp_path / "a.wav"
    res = adapter.transcribe("m001", audio, lang="auto", pass_="final")
    assert res.value["segments"][0]["text"].startswith("Mình chốt")
    assert res.wall_s > 0
    assert adapter.diarize("m001", audio, pass_="final").value["turns"][1]["speaker"] == "S2"
    transcript = tmp_path / "t.json"
    assert adapter.notes("m001", transcript, lang="auto").value["action_items"][0]["owner"] == "S2"


def test_stream_events(adapter, tmp_path):
    res = adapter.transcribe_stream("m001", tmp_path / "a.wav", lang="vi", realtime=True)
    assert [e["type"] for e in res.value] == ["partial", "final", "end"]


def test_exit_3_is_not_supported(adapter, tmp_path, monkeypatch):
    monkeypatch.setenv("FAKE_GHI_MODE", "notimpl")
    with pytest.raises(NotSupported) as exc:
        adapter.transcribe("m001", tmp_path / "a.wav", lang="auto", pass_="final")
    assert "system does not support asr yet" in str(exc.value)
    assert "phase 3" in exc.value.reason


def test_exit_1_and_bad_output(adapter, tmp_path, monkeypatch):
    audio = tmp_path / "a.wav"
    monkeypatch.setenv("FAKE_GHI_MODE", "fail")
    with pytest.raises(AdapterError, match="boom"):
        adapter.diarize("m001", audio, pass_="final")
    monkeypatch.setenv("FAKE_GHI_MODE", "badjson")
    with pytest.raises(AdapterError, match="not one JSON"):
        adapter.diarize("m001", audio, pass_="final")
    monkeypatch.setenv("FAKE_GHI_MODE", "badschema")
    with pytest.raises(InvalidOutput) as exc:
        adapter.notes("m001", audio, lang="auto")
    assert exc.value.raw == {"schema": "ghi.notes/1"}


def test_parse_error_line():
    err = '{"schema":"ghi.error/1","code":"internal","message":"x"}'
    assert parse_error_line("log\n" + err + "\n")["code"] == "internal"
    assert parse_error_line("log only") is None
    assert parse_error_line("") is None
    assert parse_error_line('{"schema":"other"}') is None


def test_missing_binary():
    with pytest.raises(AdapterError, match="not found"):
        make_adapter("ghi:/definitely/not/here/ghi")


@pytest.mark.skipif(not os.environ.get("GHI_BIN"), reason="GHI_BIN not set")
def test_real_binary(tmp_path):
    binary = os.environ["GHI_BIN"]
    out = subprocess.run([binary, "version", "--json"], capture_output=True, text=True, check=True)
    assert json.loads(out.stdout)["schema"] == "ghi.version/1"
    adapter = make_adapter(f"ghi:{shutil.which(binary) or binary}")
    wav = tmp_path / "a.wav"
    from conftest import write_silence

    write_silence(wav, 1.0)
    with pytest.raises(NotSupported):  # exit 3 not_implemented until phase 3
        adapter.transcribe("a", wav, lang="auto", pass_="final")
