# SPDX-License-Identifier: Apache-2.0
import pytest
from conftest import write_silence

from ghi_eval.cli import main
from ghi_eval.errors import ManifestError
from ghi_eval.manifest import load_manifest, speaker_bucket, validate


def test_buckets():
    assert [speaker_bucket(n) for n in (1, 2, 3, 5, 6, 8)] == [
        "1-2",
        "1-2",
        "3-5",
        "3-5",
        "6+",
        "6+",
    ]


def test_tiny_manifest_validates_without_audio(tiny):
    errors, warnings, stats = validate(load_manifest(tiny))
    assert errors == []
    assert any("audio missing" in w for w in warnings)
    assert stats["hours"] == pytest.approx(74 / 3600)


def test_missing_audio_without_duration_is_an_error(tiny):
    text = (
        (tiny / "manifest.yaml").read_text(encoding="utf-8").replace("    duration_s: 30.0\n", "")
    )
    (tiny / "manifest.yaml").write_text(text, encoding="utf-8")
    errors, _, _ = validate(load_manifest(tiny))
    assert any("no duration_s" in e for e in errors)
    assert main(["validate", "--dataset", str(tiny)]) == 1


def test_non_wav_audio_is_reported(tiny):
    (tiny / "audio").mkdir()
    (tiny / "audio" / "t01.wav").write_bytes(b"ID3 this is not a wav")
    errors, _, _ = validate(load_manifest(tiny))
    assert any("not a readable PCM WAV" in e for e in errors)


def test_real_audio_duration_and_mismatch(tiny):
    write_silence(tiny / "audio" / "t01.wav", 5.0)
    _, warnings, _ = validate(load_manifest(tiny))
    assert any("differs from audio" in w for w in warnings)


def test_rttm_id_mismatch(tiny):
    p = tiny / "labels" / "t01.rttm"
    p.write_text(p.read_text(encoding="utf-8").replace("t01", "zzz"), encoding="utf-8")
    errors, _, _ = validate(load_manifest(tiny))
    assert any("file-id column" in e for e in errors)


def test_structural_errors(tmp_path):
    (tmp_path / "manifest.yaml").write_text(
        "version: 2\nfiles:\n  - {id: 'a b', audio: x.wav, lang: fr, setting: room, playback: na, speakers: 0}\n",
        encoding="utf-8",
    )
    with pytest.raises(ManifestError) as exc:
        load_manifest(tmp_path)
    msg = str(exc.value)
    for frag in ("version must be 1", "id must match", "lang must be", "speakers must be"):
        assert frag in msg
