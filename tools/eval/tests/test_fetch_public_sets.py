# SPDX-License-Identifier: Apache-2.0
"""Offline tests for scripts/fetch_public_sets.py: the network is replaced by a fake."""

from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import struct
import tarfile
import wave
import zipfile
from pathlib import Path

import pytest
import yaml

SCRIPT = Path(__file__).resolve().parent.parent / "scripts" / "fetch_public_sets.py"
FILE_KEYS = {
    "id", "audio", "tracks", "rttm", "ref", "notes_ref", "lang", "setting", "playback",
    "speakers", "duration_s", "persons",
}  # fmt: skip


def load_script():
    spec = importlib.util.spec_from_file_location("fetch_public_sets", SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def wav_bytes(seconds: float = 1.0, rate: int = 16000) -> bytes:
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(b"\x00\x00" * int(seconds * rate))
    return buf.getvalue()


def float_wav_bytes(seconds: float = 0.5, rate: int = 16000) -> bytes:
    """A 32-bit IEEE float WAV, the way FLEURS ships its audio."""
    n = int(seconds * rate)
    body = struct.pack("<" + "f" * n, *[0.5] * n)
    fmt = struct.pack("<HHIIHH", 3, 1, rate, rate * 4, 4, 32)
    chunks = b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(body)) + body
    return b"RIFF" + struct.pack("<I", 4 + len(chunks)) + b"WAVE" + chunks


def rttm(fid: str, spk: list[str]) -> bytes:
    lines = [f"SPEAKER {fid} 1 {i}.000 0.500 <NA> <NA> {s} <NA> <NA>" for i, s in enumerate(spk)]
    return ("\n".join(lines) + "\n").encode()


class FakeNet:
    """Serves bytes by URL and records every request."""

    def __init__(self):
        self.files: dict[str, bytes] = {}
        self.requests: list[str] = []
        self.tokens: list[str] = []

    def http_get(self, url, headers=None):
        self.requests.append(url)
        data = self.files[url]
        rng = (headers or {}).get("Range")
        if rng:
            a, b = rng.removeprefix("bytes=").split("-")
            return data[int(a) : int(b) + 1]
        return data

    def http_get_hf(self, url, token):
        self.tokens.append(token)
        return self.http_get(url)

    def http_stream(self, url):
        self.requests.append(url)
        return io.BytesIO(self.files[url])

    def http_size(self, url):
        return len(self.files[url])

    def http_download(self, url, dest):
        self.requests.append(url)
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(self.files[url])
        return sha(self.files[url])


@pytest.fixture()
def fps(monkeypatch):
    mod = load_script()
    net = FakeNet()
    for name in ("http_get", "http_get_hf", "http_stream", "http_size", "http_download"):
        monkeypatch.setattr(mod, name, getattr(net, name))
    mod.net = net
    return mod


def check_manifest(set_dir: Path, ids: set[str]) -> dict:
    m = yaml.safe_load((set_dir / "manifest.yaml").read_text(encoding="utf-8"))
    assert m["version"] == 1
    assert isinstance(m["name"], str)
    assert m["names"] == []
    assert {e["id"] for e in m["files"]} == ids
    for e in m["files"]:
        assert set(e) <= FILE_KEYS
        assert {"id", "audio", "lang", "setting", "playback", "speakers"} <= set(e)
        assert e["lang"] in {"vi", "en", "mixed"}
        assert e["setting"] in {"room", "call", "other"}
        assert e["playback"] in {"headphones", "speakers", "na"}
        assert isinstance(e["speakers"], int) and e["speakers"] >= 1
        assert (set_dir / e["audio"]).is_file()
        assert e["duration_s"] > 0
        if "rttm" in e:
            assert (set_dir / e["rttm"]).is_file()
            assert Path(e["rttm"]).stem == e["id"]
        if "ref" in e:
            assert (set_dir / e["ref"]).is_file()
    assert (set_dir / "LICENSE-NOTICE.txt").read_text().startswith("Licence:")
    return m


def test_dry_run_downloads_nothing(fps, tmp_path, capsys):
    rc = fps.main(["--sets", "all", "--out", str(tmp_path), "--dry-run", "--with-published-hyp"])
    assert rc == 0
    assert fps.net.requests == []
    assert list(tmp_path.iterdir()) == []
    out = capsys.readouterr().out
    for name in fps.ALL_SETS:
        assert name in out
    assert "huggingface.co/datasets/google/fleurs" in out


def test_unknown_set_is_a_usage_error(fps, tmp_path):
    with pytest.raises(SystemExit) as e:
        fps.main(["--sets", "nope", "--out", str(tmp_path)])
    assert e.value.code == 2


def test_fleurs(fps, tmp_path, monkeypatch):
    monkeypatch.setattr(fps, "FLEURS_SMALL_N", 2)
    tsv = (
        b"1\t111.wav\tRaw one.\traw one\tp h\t10\tMALE\n"
        b"1\t222.wav\tRaw two.\traw two\tp h\t10\tFEMALE\n"
        b"1\t333.wav\tRaw three.\traw three\tp h\t10\tFEMALE\n"
    )
    monkeypatch.setattr(fps, "FLEURS_TSV_SHA256", sha(tsv))
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        for name in ("111", "222", "333"):
            data = float_wav_bytes(0.5)
            info = tarfile.TarInfo(f"test/{name}.wav")
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
    fps.net.files[fps.FLEURS_TSV] = tsv
    fps.net.files[fps.FLEURS_TAR] = buf.getvalue()

    assert fps.main(["--sets", "fleurs-vi", "--out", str(tmp_path)]) == 0
    m = check_manifest(tmp_path / "fleurs-vi", {"fleurs_111", "fleurs_222"})
    assert all(e["lang"] == "vi" and e["setting"] == "other" for e in m["files"])
    with wave.open(str(tmp_path / "fleurs-vi/audio/fleurs_111.wav")) as w:  # converted to PCM16
        assert (w.getsampwidth(), w.getframerate(), w.getnframes()) == (2, 16000, 8000)
        assert int.from_bytes(w.readframes(1), "little", signed=True) == 16383
    assert (tmp_path / "fleurs-vi/refs/fleurs_111.txt").read_text() == "raw one\n"

    # Second run is resume-safe: the audio is there, so the archive is not streamed again.
    fps.net.requests.clear()
    assert fps.main(["--sets", "fleurs-vi", "--out", str(tmp_path)]) == 0
    assert fps.FLEURS_TAR not in fps.net.requests


def test_pinned_hash_mismatch_aborts(fps, tmp_path):
    fps.net.files[fps.FLEURS_TSV] = b"tampered"
    with pytest.raises(SystemExit, match="SHA-256 mismatch"):
        fps.main(["--sets", "fleurs-vi", "--out", str(tmp_path)])


def test_ami(fps, tmp_path, monkeypatch):
    monkeypatch.setattr(fps, "AMI_SMALL", ["ES2004a"])
    r = rttm("ES2004a", ["A", "B", "C", "A"])
    monkeypatch.setitem(fps.AMI_RTTM_SHA256, "ES2004a", sha(r))
    fps.net.files[fps.AMI_RTTM.format(id="ES2004a")] = r
    fps.net.files[fps.AMI_WAV.format(id="ES2004a")] = wav_bytes(2.0)

    assert fps.main(["--sets", "ami-sdm", "--out", str(tmp_path)]) == 0
    m = check_manifest(tmp_path / "ami-sdm", {"ES2004a"})
    e = m["files"][0]
    assert (e["lang"], e["setting"], e["speakers"]) == ("en", "room", 3)
    assert e["duration_s"] == 2.0


def test_vimedcss(fps, tmp_path, monkeypatch):
    monkeypatch.setattr(fps, "VIMED_SMALL_N", 2)
    rows = [
        {
            "row": {
                "segment_id": f"Med_CS-0-{i}",
                "segment_text": f"câu {i} có từ gen ",
                "audio": [{"src": f"https://example.test/{i}.wav", "type": "audio/wav"}],
            }
        }
        for i in range(2)
    ]
    fps.net.files[fps.VIMED_ROWS.format(offset=0, length=2)] = json.dumps({"rows": rows}).encode()
    for i in range(2):
        fps.net.files[f"https://example.test/{i}.wav"] = wav_bytes(1.0)

    assert fps.main(["--sets", "vimedcss", "--out", str(tmp_path)]) == 0
    m = check_manifest(tmp_path / "vimedcss", {"Med_CS-0-0", "Med_CS-0-1"})
    assert all(e["lang"] == "mixed" for e in m["files"])
    assert (tmp_path / "vimedcss/refs/Med_CS-0-0.txt").read_text(encoding="utf-8") == (
        "câu 0 có từ gen\n"
    )


def build_vox(fps, monkeypatch, ids):
    zbuf = io.BytesIO()
    with zipfile.ZipFile(zbuf, "w", zipfile.ZIP_STORED) as z:
        z.writestr("voxconverse_test_wav/", b"")
        for i in ids:
            z.writestr(f"voxconverse_test_wav/{i}.wav", wav_bytes(1.5))
        z.writestr("__MACOSX/voxconverse_test_wav/._x.wav", b"junk")
    fps.net.files[fps.VOX_ZIP] = zbuf.getvalue()
    monkeypatch.setattr(fps, "VOX_SMALL", ids)
    for rev in (fps.VOX_REV, fps.VOX_REV_V002):
        for i in ids:
            r = rttm(i, ["spk00", "spk01"])
            fps.net.files[fps.VOX_RTTM.format(rev=rev, id=i)] = r
            monkeypatch.setitem(fps.VOX_RTTM_SHA256, (rev, i), sha(r))


def test_voxconverse_with_published_hyp(fps, tmp_path, monkeypatch, capsys):
    ids = ["aepyx", "aggyz"]
    build_vox(fps, monkeypatch, ids)
    hyp = b"".join(rttm(i, ["h1", "h2"]) for i in ids + ["other"])
    ev = (
        b"                    diarization error rate     total   correct\n"
        b"item\n"
        b"aepyx  10.00  100.00  90.00  90.0  4.00  4.0  3.00  3.0  3.00  3.0\n"
        b"aggyz  20.00  100.00  80.00  80.0  5.00  5.0  5.00  5.0  10.00  10.0\n"
        b"other  99.00  100.00  1.00  1.0  0.00  0.0  0.00  0.0  0.00  0.0\n"
        b"TOTAL  12.76  300.00  1.00  1.0  0.00  0.0  0.00  0.0  0.00  0.0\n"
    )
    monkeypatch.setattr(fps, "PUB_RTTM", (fps.PUB_RTTM[0], sha(hyp)))
    monkeypatch.setattr(fps, "PUB_EVAL", (fps.PUB_EVAL[0], sha(ev)))
    fps.net.files[fps.PUB_RTTM[0]] = hyp
    fps.net.files[fps.PUB_EVAL[0]] = ev

    monkeypatch.setenv("HF_TOKEN", "hf_secret_token")
    rc = fps.main(["--sets", "voxconverse", "--with-published-hyp", "--out", str(tmp_path)])
    assert rc == 0
    d = tmp_path / "voxconverse"
    m = check_manifest(d, set(ids))
    assert m["name"] == "voxconverse-v0.0.2"
    assert fps.VOX_RTTM.format(rev=fps.VOX_REV_V002, id="aepyx") in fps.net.requests
    assert fps.VOX_RTTM.format(rev=fps.VOX_REV, id="aepyx") not in fps.net.requests
    assert fps.net.tokens == ["hf_secret_token"] * 2  # only the two gated files use the token
    assert "hf_secret_token" not in capsys.readouterr().out
    # Hypotheses are split per file, laid out as a files: run directory.
    assert sorted(p.name for p in (d / "published-hyp/hyp").iterdir()) == [
        "aepyx.rttm",
        "aggyz.rttm",
    ]
    assert "aepyx" in (d / "published-hyp/hyp/aepyx.rttm").read_text()
    # Pooled published DER for the subset: (10 + 20) / 200 -> errors 10+20 over 200 = 15 %.
    info = json.loads((d / "published.json").read_text())
    assert info["subset_der_percent"] == 15.0
    assert info["published_der_percent_full_test_set"] == 12.76
    assert "collar 0.0" in info["conditions"]


def test_voxconverse_default_uses_v03_refs(fps, tmp_path, monkeypatch):
    build_vox(fps, monkeypatch, ["aepyx"])
    assert fps.main(["--sets", "voxconverse", "--out", str(tmp_path)]) == 0
    m = check_manifest(tmp_path / "voxconverse", {"aepyx"})
    assert m["name"] == "voxconverse"
    assert not (tmp_path / "voxconverse/published-hyp").exists()
    assert fps.VOX_RTTM.format(rev=fps.VOX_REV_V002, id="aepyx") not in fps.net.requests


def test_range_file_matches_plain_read(fps):
    data = bytes(range(256)) * 9000  # > 2 blocks
    fps.net.files["https://x.test/f"] = data
    rf = fps.RangeFile("https://x.test/f")
    rf.seek(-100, 2)
    assert rf.read() == data[-100:]
    rf.seek(1_500_000)
    assert rf.read(10) == data[1_500_000:1_500_010]
    rf.seek(5)
    assert rf.read(3_000_000) == data[5 : 5 + 3_000_000]


def test_published_subset_der_ignores_other_files(fps):
    ev = "a  10.0  50.0  1 1 2.5  1 2.5  1 0 0\nb  90.0  50.0  1 1 1.0 1 1.0 1 0 0\n"
    assert fps.published_subset_der(ev, ["a"])["subset_der_percent"] == 10.0


def test_published_hyp_without_token_stops_before_any_download(fps, tmp_path, monkeypatch):
    monkeypatch.delenv("HF_TOKEN", raising=False)
    with pytest.raises(SystemExit, match="accept the conditions"):
        fps.main(["--sets", "voxconverse", "--with-published-hyp", "--out", str(tmp_path)])
    assert fps.net.requests == []


def test_hf_token_is_not_sent_to_the_cdn(monkeypatch):
    mod = load_script()
    seen = []

    class Resp(io.BytesIO):
        def __enter__(self):
            return self

        def __exit__(self, *a):
            self.close()

    class Opener:
        def open(self, req, timeout=None):
            seen.append((req.full_url, req.get_header("Authorization")))
            if "huggingface.co" in req.full_url.split("/")[2] and "cdn" not in req.full_url:
                raise mod.urllib.error.HTTPError(
                    req.full_url, 302, "found", {"Location": "https://cdn.example.net/blob"}, None
                )
            return Resp(b"data")

    monkeypatch.setattr(mod.urllib.request, "build_opener", lambda *a: Opener())
    assert mod.http_get_hf("https://huggingface.co/x/resolve/main/f", "tok") == b"data"
    assert seen[0] == ("https://huggingface.co/x/resolve/main/f", "Bearer tok")
    assert seen[1] == ("https://cdn.example.net/blob", None)


def test_non_https_urls_are_refused(fps, tmp_path):
    for url in ("file:///etc/passwd", "http://example.test/a.wav", "ftp://example.test/a"):
        with pytest.raises(SystemExit, match="non-https"):
            fps.fetch_big(url, tmp_path / "x.wav")
        with pytest.raises(SystemExit, match="non-https"):
            fps.require_https(url)
    assert not (tmp_path / "x.wav").exists()
    assert fps.net.requests == []


def test_real_stream_refuses_file_scheme(monkeypatch):
    mod = load_script()  # the unpatched network primitives
    with pytest.raises(SystemExit, match="non-https"):
        mod.http_stream("file:///etc/passwd")


def test_hf_token_needs_https_and_exact_host(monkeypatch):
    mod = load_script()
    seen = []

    class Opener:
        def open(self, req, timeout=None):
            seen.append((req.full_url, req.get_header("Authorization")))
            raise mod.urllib.error.HTTPError(
                req.full_url, 302, "found", {"Location": "https://huggingface.co.evil.test/x"}, None
            )

    monkeypatch.setattr(mod.urllib.request, "build_opener", lambda *a: Opener())
    with pytest.raises(SystemExit):
        mod.http_get_hf("https://huggingface.co/a", "tok")
    # first hop carries the token; the look-alike host, and any repeat hop, must not
    assert seen[0][1] == "Bearer tok"
    assert all(auth is None for url, auth in seen[1:])
    with pytest.raises(SystemExit, match="non-https"):
        mod.http_get_hf("http://huggingface.co/a", "tok")
