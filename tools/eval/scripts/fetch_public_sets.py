# SPDX-License-Identifier: Apache-2.0
"""Fetch small public datasets in the eval-kit layout (docs/formats.md section 1).

Sets (all free to download for this use, see LICENCES below):

  fleurs-vi   Google FLEURS Vietnamese, test utterances     ASR only, CC-BY-4.0
  fleurs-en   Google FLEURS English (en_us), test utterances ASR only, CC-BY-4.0
  ami-sdm     AMI meetings, far-field mic Array1-01          diarization, CC-BY-4.0
  voxconverse VoxConverse test files                         diarization, CC-BY-4.0
  vimedcss    ViMedCSS Vietnamese-English code-switch speech ASR only, CC-BY-4.0

Data lands in tools/eval/data/<set>/ (git-ignored). Nothing is committed.
Only the standard library and PyYAML are used.

    python scripts/fetch_public_sets.py --sets fleurs-vi,ami-sdm
    python scripts/fetch_public_sets.py --sets voxconverse --with-published-hyp
    python scripts/fetch_public_sets.py --sets all --dry-run
"""

from __future__ import annotations

import argparse
import array
import hashlib
import io
import json
import os
import re
import shutil
import ssl
import sys
import tarfile
import urllib.error
import urllib.parse
import urllib.request
import wave
import zipfile
from pathlib import Path

import yaml

DEFAULT_OUT = Path(__file__).resolve().parent.parent / "data"
UA = "ghi-eval-fetch/0.1 (+https://github.com/ghira)"
TIMEOUT = 60
ALL_SETS = ("fleurs-vi", "fleurs-en", "ami-sdm", "voxconverse", "vimedcss")
DEFAULT_SETS = ("fleurs-vi", "ami-sdm")

# --- Sources, pinned to immutable revisions -------------------------------------------------

FLEURS_REV = "70bb2e84b976b7e960aa89f1c648e09c59f894dd"  # google/fleurs, verified 2026-09-29
FLEURS_BASE = f"https://huggingface.co/datasets/google/fleurs/resolve/{FLEURS_REV}/data/vi_vn"
FLEURS_TSV = f"{FLEURS_BASE}/test.tsv"
FLEURS_TSV_SHA256 = "eb744c2be5f677b49c35527d428a4f647294bd0f67abd790c296e91efaaffb93"
FLEURS_TAR = f"{FLEURS_BASE}/audio/test.tar.gz"  # ~544 MB; streamed, we stop after N files
FLEURS_SMALL_N = 25
FLEURS_EN_BASE = f"https://huggingface.co/datasets/google/fleurs/resolve/{FLEURS_REV}/data/en_us"
FLEURS_EN_TSV = f"{FLEURS_EN_BASE}/test.tsv"
FLEURS_EN_TSV_SHA256 = "74c046239374deeb60fa63f258f907388093a32bcaa3140965f70ef05c79f7ca"
FLEURS_EN_TAR = f"{FLEURS_EN_BASE}/audio/test.tar.gz"  # ~290 MB; streamed, we stop after N files

AMI_REV = "2509d8933721023fab4def2618aabd5c28eb82e9"  # BUTSpeechFIT/AMI-diarization-setup
AMI_RTTM = (
    f"https://raw.githubusercontent.com/BUTSpeechFIT/AMI-diarization-setup/{AMI_REV}"
    "/only_words/rttms/test/{id}.rttm"
)
AMI_WAV = "https://groups.inf.ed.ac.uk/ami/AMICorpusMirror/amicorpus/{id}/audio/{id}.Array1-01.wav"
AMI_TEST = (
    "IS1009a IS1009b IS1009c IS1009d ES2004a ES2004b ES2004c ES2004d "
    "TS3003a TS3003b TS3003c TS3003d EN2002a EN2002b EN2002c EN2002d"
).split()
AMI_SMALL = ["ES2004a", "IS1009a"]  # 34 and 27 MB of audio, ~31 min
AMI_RTTM_SHA256 = {
    "IS1009a": "ba38d35ca567f3f1e061d90fdc33579ce60d2060fc8664eaeb77d2e0fcd88b01",
    "IS1009b": "5cb6ae3948c09ce8356991cc79bb593abc1f0d23f1dbadeeb88e17e055283e67",
    "IS1009c": "356ec69ac61ceec7f95ea66108a6dfe917e83e977f66487c525de5c18b02bbb2",
    "IS1009d": "512f5b84fefaa9a5049ace143ef1a24b219baca6fcfe64a9a8c30b7331fe2e4a",
    "ES2004a": "9869c6146c2fd9595403edb36c2caeda65c12ffa2c0af4ce48d6814b673fd5a9",
    "ES2004b": "97a04fa14a37249b09581969939e5024324bbe3fc9c7a77e19976af41147ad7d",
    "ES2004c": "fccef2416fe556133db1784f4f755029fd8961d2929b69e102375240e95493ed",
    "ES2004d": "bb9fdfc402391bd2ff88d5a0caef159a0825d18ff85b346d4c28ce7bf1a714b6",
    "TS3003a": "57137f098ee3be9b8d77c9faa4b16e9155e821bdbf21b974dd8771d7e646d185",
    "TS3003b": "c728c1b1305a977a5adc32830c8299f1ee6084dee113a7e56f6d9cce540e5b3a",
    "TS3003c": "62e491fd0e4265b74bb114c17d1351de7c907af6582a2efbd4f765211ec44d52",
    "TS3003d": "dfdc8e4721d13c9e7a63dee29a32d19b534ed3b99cb5abeda6a4185432a07f86",
    "EN2002a": "a93919ae5f18e476614b7e5a2713c4dc6bba23e95149fba18dbe68602b13ee40",
    "EN2002b": "c4e6ef3f9f493d82d14664a9669479aca378de05639e4640f02f8398e2ef714a",
    "EN2002c": "e595fa5953a923cbd62ffc881bd9725df98a6587bc14730dd279937773953bca",
    "EN2002d": "f11eeb70b0e1721b5b4b831a891d2b70c0b1a6ad368d0495d73c998335148463",
}

VOX_REV = "24bf60be297701cd7e4ef18550c6d390c1b87365"  # joonson/voxconverse master (v0.3)
VOX_REV_V002 = "55b8bfcd343f3eeee75984151d04f91ab78aa243"  # branch ver0.2 (v0.0.2)
VOX_RTTM = "https://raw.githubusercontent.com/joonson/voxconverse/{rev}/test/{id}.rttm"
VOX_ZIP = (
    "https://www.robots.ox.ac.uk/~vgg/data/voxconverse/data/voxconverse_test_wav.zip"  # 4.3 GB
)
VOX_SMALL = ["aepyx", "dohag", "euqef", "fuzfh", "msbyq", "neiye", "xkmqx"]  # shortest, ~7 min
# v0.0.2 and v0.3 references are identical for the small subset (aiqwk is the one file among
# the shortest ones that differs), so both revisions share the hashes below.
# Files with 9+ reference speakers (more than the diarizer's cap of 8), shortest first, for the
# re-cluster eval (`--subset many`): speakers/seconds in the comment.
VOX_MANY = (
    "fpfvy cwbvu xtzoq gukoa ezxso aggyz uqxlg fzwtp mclsr qeejz byapz wlfsf usqam vtzqw "
    "nitgx jeymh"  # 11/121 10/121 11/185 10/247 10/277 13/260 15/288 11/284 11/289 14/292 ...
).split()
# Control for the same eval (`--subset ctrl`): 7-8 reference speakers, where the capped diarizer
# is right or nearly so and re-clustering must not make things worse.
VOX_CTRL = ("nqcpi erslt kpjud gfneh isrps ralnu dzxut cadba aiqwk eoyaz ygrip qadia").split()
VOX_RTTM_SHA256 = {
    (VOX_REV, "aepyx"): "fd5bf3e0ecfacaba66d749e75f39574162b8cb472e7910ecadf0fc949d00e9c9",
    (VOX_REV, "dohag"): "7e16664718898767562103aa8c1c0cc4d725bdf4728ba02439221b9efa73f7da",
    (VOX_REV, "fuzfh"): "eee921cf6480ae486f5a92c9bd79ec05d7a7f213879ed6f63fddf7a58c5fa2b3",
    (VOX_REV, "euqef"): "7ed318b4ceb58d245fb31965bf2f30df1bb530a550cd2631cb47ad2699ca7a6b",
    (VOX_REV, "msbyq"): "63b26d71bbbfb4f1f7805b1e540665897714e378a13829bc31827ff9e4c60131",
    (VOX_REV, "neiye"): "cd70cd411847e2da0cbb3bdca1af1045c737f59bf92b9a1d09a4e0ec7976dbd7",
    (VOX_REV, "xkmqx"): "5ac26edc7f78dd5827db22d75be36963e84e8ebace1a02430e35dea3b641dd30",
}
# For the ids whose v0.0.2 reference equals v0.3, reuse the same hash.
for _i in VOX_SMALL:
    VOX_RTTM_SHA256.setdefault((VOX_REV_V002, _i), VOX_RTTM_SHA256[(VOX_REV, _i)])

# Published system output for VoxConverse test: pyannote.audio speaker-diarization 2.1, MIT
# licence, from the authors' own repository `pyannote/speaker-diarization`. That repository is
# gated on Hugging Face: the user must accept its conditions on the website and pass a token
# in HF_TOKEN. The published numbers come from the authors' .eval file.
PUB_REPO = "pyannote/speaker-diarization"
PUB_GATE_URL = f"https://huggingface.co/{PUB_REPO}"
PUB_BASE = f"{PUB_GATE_URL}/resolve/main/reproducible_research/2.1/VoxConverse.test"
# Not pinned yet: these could only be fetched with a token. A public mirror of the same folder
# had these SHA-256 values; when the script prints matching hashes for the gated files,
# paste them here to pin (then PUB_RTTM/PUB_EVAL hashes are enforced).
#   .rttm c1f14b7889aa7319f2e415abaaf4cee7691b39e17943e352db0cc11df6ffabc4
#   .eval 801f288181a04d08a183693e2327b263f90c99fd3cedc81f42dabb92bb71fb55
PUB_RTTM = (PUB_BASE + ".rttm", None)
PUB_EVAL = (PUB_BASE + ".eval", None)
PUB_DER_ALL = 12.76  # % over the full VoxConverse test set (232 files)
PUB_CONDITIONS = (
    "collar 0.0 s, overlapped speech scored, no oracle VAD; VoxConverse v0.0.2 references"
)

VIMED_ROWS = (
    "https://datasets-server.huggingface.co/rows?dataset=tensorxt/ViMedCSS"
    "&config=default&split=test&offset={offset}&length={length}"
)
VIMED_SMALL_N = 30
VIMED_FULL_N = 1614

LICENCES = {
    "fleurs-en": {
        "licence": "CC-BY-4.0",
        "attribution": (
            'FLEURS (Conneau et al., "FLEURS: Few-shot Learning Evaluation of Universal '
            'Representations of Speech", 2022), Google, CC BY 4.0, via '
            "https://huggingface.co/datasets/google/fleurs"
        ),
    },
    "fleurs-vi": {
        "licence": "CC-BY-4.0",
        "attribution": (
            'FLEURS (Conneau et al., "FLEURS: Few-shot Learning Evaluation of Universal '
            'Representations of Speech", 2022), Google, CC BY 4.0, via '
            "https://huggingface.co/datasets/google/fleurs"
        ),
    },
    "ami-sdm": {
        "licence": "CC-BY-4.0 (audio, AMI corpus); Apache-2.0 (BUT reference RTTMs)",
        "attribution": (
            "AMI Meeting Corpus (Carletta et al., 2005), http://groups.inf.ed.ac.uk/ami, "
            "CC BY 4.0. Reference RTTMs: BUTSpeechFIT/AMI-diarization-setup (Landini et al., "
            "VBx, Computer Speech & Language 2022), Apache-2.0, 'only_words' setup."
        ),
    },
    "voxconverse": {
        "licence": "CC-BY-4.0 (annotations and audio); video copyright stays with the owners",
        "attribution": (
            'VoxConverse (Chung et al., "Spot the conversation: speaker diarisation in the '
            'wild", Interspeech 2020), https://github.com/joonson/voxconverse, CC BY 4.0. '
            "Research use; the copyright of the videos remains with the original owners."
        ),
    },
    "vimedcss": {
        "licence": "CC-BY-4.0 (as declared by the dataset authors)",
        "attribution": (
            "ViMedCSS: A Vietnamese Medical Code-Switching Speech Dataset & Benchmark "
            "(LREC 2026), https://huggingface.co/datasets/tensorxt/ViMedCSS, CC BY 4.0. "
            "Audio is cut from public YouTube videos; do not redistribute it."
        ),
    },
    "published-hyp": {
        "licence": "MIT (pyannote.audio pipeline outputs)",
        "attribution": (
            "pyannote/speaker-diarization 2.1 reproducible-research outputs (Bredin & "
            "Laurent, ICASSP 2021), MIT. Downloaded from the authors' gated repository with the "
            "user's own Hugging Face token."
        ),
    },
}

SIZES = {
    "fleurs-en": "small ~20 MB (25 utterances, ~5 min); full ~290 MB",
    "fleurs-vi": "small ~20 MB (25 utterances, ~5 min); full ~544 MB",
    "ami-sdm": "small ~60 MB (2 meetings, ~31 min); full ~700 MB (16 meetings)",
    "voxconverse": "small ~15 MB (7 files, ~7 min; the host is slow, allow ~10 min); full ~4.3 GB zip (232 files)",
    "vimedcss": "small ~40 MB (30 utterances); full ~2 GB (1614 utterances)",
}


# --- Network primitives (tests replace these four) ------------------------------------------


def _ssl_context() -> ssl.SSLContext:
    try:
        import certifi

        return ssl.create_default_context(cafile=certifi.where())
    except ImportError:
        return ssl.create_default_context()


def require_https(url: str) -> str:
    """Only https URLs are fetched: no file://, http:// or other schemes, even from JSON."""
    if urllib.parse.urlparse(url).scheme != "https":
        raise SystemExit(f"refusing to fetch a non-https URL: {url[:80]}")
    return url


def _request(url: str, headers: dict[str, str] | None = None, method: str = "GET"):
    req = urllib.request.Request(
        require_https(url), headers={"User-Agent": UA, **(headers or {})}, method=method
    )
    return urllib.request.urlopen(req, timeout=TIMEOUT, context=_ssl_context())


def http_get(url: str, headers: dict[str, str] | None = None) -> bytes:
    """Read a whole (small) response."""
    with _request(url, headers) as r:
        return r.read()


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def http_get_hf(url: str, token: str) -> bytes:
    """GET from huggingface.co with a Bearer token.

    Redirects are followed by hand and the token is sent only to huggingface.co itself,
    never to the CDN host a download redirects to. The token is never printed.
    """
    opener = urllib.request.build_opener(
        _NoRedirect, urllib.request.HTTPSHandler(context=_ssl_context())
    )
    for _ in range(5):
        require_https(url)
        headers = {"User-Agent": UA}
        parts = urllib.parse.urlparse(url)
        if parts.scheme == "https" and parts.hostname == "huggingface.co":
            headers["Authorization"] = f"Bearer {token}"
        try:
            with opener.open(urllib.request.Request(url, headers=headers), timeout=TIMEOUT) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code in (301, 302, 303, 307, 308) and e.headers.get("Location"):
                url = urllib.parse.urljoin(url, e.headers["Location"])
                continue
            if e.code in (401, 403):
                raise SystemExit(
                    f"Hugging Face refused the request (HTTP {e.code}). Accept the conditions "
                    f"at {PUB_GATE_URL} while logged in, and check that HF_TOKEN is a valid "
                    "read token of that account."
                ) from None
            raise
    raise SystemExit("too many redirects")


def http_stream(url: str):
    """Return a file-like object that streams the response; the caller closes it."""
    return _request(url)


def http_size(url: str) -> int:
    with _request(url, method="HEAD") as r:
        return int(r.headers["Content-Length"])


def http_download(url: str, dest: Path) -> str:
    """Stream url to dest atomically. Returns the SHA-256 of what was written."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(dest.name + ".part")
    h = hashlib.sha256()
    with http_stream(url) as r, part.open("wb") as f:
        while chunk := r.read(1 << 20):
            f.write(chunk)
            h.update(chunk)
    part.replace(dest)
    return h.hexdigest()


class RangeFile(io.RawIOBase):
    """Seekable read-only view of a remote file via HTTP Range, with a 1 MiB read-ahead."""

    BLOCK = 1 << 20

    def __init__(self, url: str):
        self.url = url
        self.size = http_size(url)
        self.pos = 0
        self._buf_start = 0
        self._buf = b""

    def readable(self) -> bool:
        return True

    def seekable(self) -> bool:
        return True

    def tell(self) -> int:
        return self.pos

    def seek(self, offset: int, whence: int = 0) -> int:
        base = {0: 0, 1: self.pos, 2: self.size}[whence]
        self.pos = max(0, base + offset)
        return self.pos

    def read(self, n: int = -1) -> bytes:
        if n is None or n < 0:
            n = self.size - self.pos
        n = min(n, self.size - self.pos)
        if n <= 0:
            return b""
        out = b""
        while n > 0:
            off = self.pos - self._buf_start
            if not (0 <= off < len(self._buf)):
                end = min(self.size, self.pos + max(n, self.BLOCK)) - 1
                self._buf = http_get(self.url, {"Range": f"bytes={self.pos}-{end}"})
                self._buf_start = self.pos
                off = 0
            chunk = self._buf[off : off + n]
            if not chunk:
                break
            out += chunk
            self.pos += len(chunk)
            n -= len(chunk)
        return out

    def readinto(self, b) -> int:
        data = self.read(len(b))
        b[: len(data)] = data
        return len(data)


# --- Helpers ---------------------------------------------------------------------------------


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()


def log(msg: str) -> None:
    print(msg, flush=True)


def fetch_small(url: str, dest: Path, sha256: str | None, token: str | None = None) -> bool:
    """Download a small file, verify the pinned hash (or print it). Resume-safe.

    Returns True if the file was downloaded now, False if it was already there and valid.
    """
    if dest.exists() and dest.stat().st_size > 0:
        if sha256 is None or sha256_file(dest) == sha256:
            return False
        log(f"  hash mismatch, fetching again: {dest.name}")
    data = http_get_hf(url, token) if token else http_get(url)
    got = hashlib.sha256(data).hexdigest()
    if sha256 is not None and got != sha256:
        raise SystemExit(f"SHA-256 mismatch for {url}\n  expected {sha256}\n  got      {got}")
    if sha256 is None:
        log(f"  sha256 {got}  {dest.name}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(data)
    return True


def fetch_big(url: str, dest: Path, sha256: str | None = None) -> None:
    """Download a large file. Skips a file that is already complete."""
    if dest.exists() and dest.stat().st_size > 0:
        if sha256 is None or sha256_file(dest) == sha256:
            return
    got = http_download(require_https(url), dest)
    if sha256 is not None and got != sha256:
        dest.unlink(missing_ok=True)
        raise SystemExit(f"SHA-256 mismatch for {url}\n  expected {sha256}\n  got      {got}")
    log(f"  sha256 {got}  {dest.name}")


def to_pcm16(data: bytes) -> bytes:
    """Return a 16-bit PCM WAV. FLEURS ships 32-bit float WAVs, which the harness cannot read."""
    try:
        with wave.open(io.BytesIO(data), "rb"):
            return data  # already PCM
    except wave.Error:
        pass
    if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise ValueError("not a RIFF/WAVE file")
    pos, fmt, samples = 12, None, None
    while pos + 8 <= len(data):
        cid, size = data[pos : pos + 4], int.from_bytes(data[pos + 4 : pos + 8], "little")
        body = data[pos + 8 : pos + 8 + size]
        if cid == b"fmt ":
            fmt = (
                int.from_bytes(body[0:2], "little"),  # 3 = IEEE float
                int.from_bytes(body[2:4], "little"),
                int.from_bytes(body[4:8], "little"),
                int.from_bytes(body[14:16], "little"),
            )
        elif cid == b"data":
            samples = body
        pos += 8 + size + (size & 1)
    if fmt is None or samples is None or fmt[0] != 3 or fmt[3] != 32:
        raise ValueError(f"unsupported WAV format {fmt}")
    floats = array.array("f")
    floats.frombytes(samples[: len(samples) - len(samples) % 4])
    if sys.byteorder == "big":
        floats.byteswap()
    pcm = array.array("h", (int(max(-1.0, min(1.0, x)) * 32767) for x in floats))
    if sys.byteorder == "big":
        pcm.byteswap()
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(fmt[1])
        w.setsampwidth(2)
        w.setframerate(fmt[2])
        w.writeframes(pcm.tobytes())
    return buf.getvalue()


def wav_seconds(path: Path) -> float | None:
    try:
        with wave.open(str(path), "rb") as w:
            return round(w.getnframes() / w.getframerate(), 1)
    except (wave.Error, EOFError):
        return None


def rttm_speakers(path: Path) -> int:
    spk = set()
    for line in path.read_text(encoding="utf-8").splitlines():
        parts = line.split()
        if len(parts) >= 8 and parts[0] == "SPEAKER":
            spk.add(parts[7])
    return len(spk)


def safe_id(text: str) -> str:
    return re.sub(r"[^A-Za-z0-9_-]", "_", text)


def write_manifest(set_dir: Path, name: str, files: list[dict]) -> Path:
    for e in files:
        secs = wav_seconds(set_dir / e["audio"])
        if secs is not None:
            e["duration_s"] = secs
    manifest = {"version": 1, "name": name, "names": [], "files": files}
    path = set_dir / "manifest.yaml"
    path.write_text(yaml.safe_dump(manifest, sort_keys=False, allow_unicode=True), encoding="utf-8")
    return path


def write_notice(set_dir: Path, key: str) -> None:
    info = LICENCES[key]
    (set_dir / "LICENSE-NOTICE.txt").write_text(
        f"Licence: {info['licence']}\nAttribution: {info['attribution']}\n", encoding="utf-8"
    )
    log(f"  licence: {info['licence']}")


# --- Sets ------------------------------------------------------------------------------------


def fetch_fleurs(out: Path, subset: str, **_) -> None:
    _fleurs(out, subset, "fleurs-vi", "vi", FLEURS_TSV, FLEURS_TSV_SHA256, FLEURS_TAR)


def fetch_fleurs_en(out: Path, subset: str, **_) -> None:
    _fleurs(out, subset, "fleurs-en", "en", FLEURS_EN_TSV, FLEURS_EN_TSV_SHA256, FLEURS_EN_TAR)


def _fleurs(
    out: Path, subset: str, name: str, lang: str, tsv_url: str, tsv_sha: str, tar_url: str
) -> None:
    set_dir = out / name
    n_max = None if subset == "full" else FLEURS_SMALL_N
    tsv = set_dir / "test.tsv"
    fetch_small(tsv_url, tsv, tsv_sha)
    rows: dict[str, str] = {}  # id -> normalized transcript
    for line in tsv.read_text(encoding="utf-8").splitlines():
        cols = line.split("\t")
        if len(cols) >= 4:
            rows.setdefault("fleurs_" + Path(cols[1]).stem, cols[3])

    (set_dir / "audio").mkdir(parents=True, exist_ok=True)
    have = len(list((set_dir / "audio").glob("*.wav")))
    if n_max is None or have < n_max:
        log(f"  streaming {tar_url} (stops after {n_max or 'all'} files)")
        stream = http_stream(tar_url)
        try:
            with tarfile.open(fileobj=stream, mode="r|gz") as tar:
                count = 0
                for member in tar:
                    fid = "fleurs_" + Path(member.name).stem
                    if not member.isfile() or fid not in rows or not member.name.endswith(".wav"):
                        continue
                    (set_dir / "audio" / f"{fid}.wav").write_bytes(
                        to_pcm16(tar.extractfile(member).read())
                    )
                    count += 1
                    if n_max is not None and count >= n_max:
                        break
        finally:
            stream.close()

    files = []
    (set_dir / "refs").mkdir(exist_ok=True)
    for wav in sorted((set_dir / "audio").glob("*.wav")):
        fid = wav.stem
        (set_dir / "refs" / f"{fid}.txt").write_text(rows[fid] + "\n", encoding="utf-8")
        files.append(
            {
                "id": fid,
                "audio": f"audio/{fid}.wav",
                "ref": f"refs/{fid}.txt",
                "lang": lang,
                "setting": "other",
                "playback": "na",
                "speakers": 1,
            }
        )
    write_manifest(set_dir, name, files)
    write_notice(set_dir, name)


def fetch_ami(out: Path, subset: str, **_) -> None:
    set_dir = out / "ami-sdm"
    ids = AMI_TEST if subset == "full" else AMI_SMALL
    files = []
    for mid in ids:
        rttm = set_dir / "labels" / f"{mid}.rttm"
        fetch_small(AMI_RTTM.format(id=mid), rttm, AMI_RTTM_SHA256[mid])
        log(f"  {mid}: audio")
        fetch_big(AMI_WAV.format(id=mid), set_dir / "audio" / f"{mid}.wav")
        files.append(
            {
                "id": mid,
                "audio": f"audio/{mid}.wav",
                "rttm": f"labels/{mid}.rttm",
                "lang": "en",
                "setting": "room",
                "playback": "na",
                "speakers": rttm_speakers(rttm),
            }
        )
    write_manifest(set_dir, "ami-sdm", files)
    write_notice(set_dir, "ami-sdm")


def _vox_zip_members(zf: zipfile.ZipFile) -> dict[str, str]:
    """Map file id -> zip member name for the .wav files in the archive."""
    found = {}
    for name in zf.namelist():
        p = Path(name)
        if p.suffix == ".wav" and not p.name.startswith("."):
            found[p.stem] = name
    return found


def _split_rttm(text: str, wanted: list[str]) -> dict[str, list[str]]:
    per: dict[str, list[str]] = {i: [] for i in wanted}
    for line in text.splitlines():
        parts = line.split()
        if len(parts) >= 8 and parts[0] == "SPEAKER" and parts[1] in per:
            per[parts[1]].append(line)
    return per


def published_subset_der(eval_text: str, ids: list[str]) -> dict:
    """Pool the published per-file numbers over ids: (FA + miss + confusion) / total."""
    total = err = 0.0
    per_file = {}
    for line in eval_text.splitlines():
        cols = line.split()
        if len(cols) == 11 and cols[0] in ids:
            der, tot, _, _, fa, _, miss, _, conf, _ = (float(c) for c in cols[1:])
            total += tot
            err += fa + miss + conf
            per_file[cols[0]] = der
    return {
        "subset_der_percent": round(100 * err / total, 2) if total else None,
        "ref_speech_s": round(total, 1),
        "per_file_der_percent": per_file,
    }


def fetch_voxconverse(out: Path, subset: str, with_published_hyp: bool = False, **_) -> None:
    set_dir = out / "voxconverse"
    token = None
    if with_published_hyp:
        token = os.environ.get("HF_TOKEN", "").strip()
        if not token:
            raise SystemExit(
                "--with-published-hyp needs the gated pyannote repository.\n"
                f"  1. Log in to Hugging Face and accept the conditions at {PUB_GATE_URL}\n"
                "  2. Create a read token at https://huggingface.co/settings/tokens\n"
                "  3. Set it: export HF_TOKEN=...   (PowerShell: $env:HF_TOKEN='...')\n"
                "The token is sent only to huggingface.co and is never printed or stored."
            )
    # The published DER was computed against v0.0.2 references, so that flag switches to them.
    rev = VOX_REV_V002 if with_published_hyp else VOX_REV
    label = "voxconverse-v0.0.2" if with_published_hyp else "voxconverse"
    ids = (
        None
        if subset == "full"
        else VOX_MANY
        if subset == "many"
        else VOX_CTRL
        if subset == "ctrl"
        else VOX_SMALL
    )
    log(f"  opening {VOX_ZIP} (only the needed files are read)")
    zf = zipfile.ZipFile(RangeFile(VOX_ZIP))
    members = _vox_zip_members(zf)
    ids = ids or sorted(members)
    files = []
    for fid in ids:
        rttm = set_dir / "labels" / f"{fid}.rttm"
        fetch_small(VOX_RTTM.format(rev=rev, id=fid), rttm, VOX_RTTM_SHA256.get((rev, fid)))
        wav = set_dir / "audio" / f"{fid}.wav"
        if not (wav.exists() and wav.stat().st_size > 0):
            wav.parent.mkdir(parents=True, exist_ok=True)
            part = wav.with_name(wav.name + ".part")
            with zf.open(members[fid]) as src, part.open("wb") as dst:
                shutil.copyfileobj(src, dst, 1 << 20)
            part.replace(wav)
        files.append(
            {
                "id": fid,
                "audio": f"audio/{fid}.wav",
                "rttm": f"labels/{fid}.rttm",
                "lang": "en",
                "setting": "other",
                "playback": "na",
                "speakers": rttm_speakers(rttm),
            }
        )
    write_manifest(set_dir, label, files)
    write_notice(set_dir, "voxconverse")
    if with_published_hyp:
        _published_hyp(set_dir, ids, token)


def _published_hyp(set_dir: Path, ids: list[str], token: str) -> None:
    pub = set_dir / "published-hyp"
    src = set_dir / "_published"
    fetch_small(PUB_RTTM[0], src / "all.rttm", PUB_RTTM[1], token)
    fetch_small(PUB_EVAL[0], src / "all.eval", PUB_EVAL[1], token)
    per = _split_rttm((src / "all.rttm").read_text(encoding="utf-8"), ids)
    (pub / "hyp").mkdir(parents=True, exist_ok=True)
    for fid, lines in per.items():
        if not lines:
            raise SystemExit(f"published hypothesis has no turns for {fid}")
        (pub / "hyp" / f"{fid}.rttm").write_text("\n".join(lines) + "\n", encoding="utf-8")
    info = published_subset_der((src / "all.eval").read_text(encoding="utf-8"), ids)
    info.update(
        {
            "system": "pyannote.audio speaker-diarization 2.1",
            "conditions": PUB_CONDITIONS,
            "published_der_percent_full_test_set": PUB_DER_ALL,
            "source": PUB_BASE + ".{rttm,eval}",
            "check": "ghi-eval score with collar 0.0; expect subset_der_percent within +-1 point",
        }
    )
    (set_dir / "published.json").write_text(json.dumps(info, indent=2) + "\n", encoding="utf-8")
    write_notice_extra(set_dir / "published-hyp")
    log(
        f"  published-hyp -> {pub}; published DER on this subset "
        f"{info['subset_der_percent']}% ({PUB_CONDITIONS})"
    )


def write_notice_extra(dir_: Path) -> None:
    info = LICENCES["published-hyp"]
    (dir_ / "LICENSE-NOTICE.txt").write_text(
        f"Licence: {info['licence']}\nAttribution: {info['attribution']}\n", encoding="utf-8"
    )


def fetch_vimedcss(out: Path, subset: str, **_) -> None:
    set_dir = out / "vimedcss"
    n = VIMED_FULL_N if subset == "full" else VIMED_SMALL_N
    rows, offset = [], 0
    while offset < n:
        length = min(100, n - offset)
        page = json.loads(http_get(VIMED_ROWS.format(offset=offset, length=length)))
        got = [r["row"] for r in page["rows"]]
        if not got:
            break
        rows += got
        offset += len(got)
    files = []
    for row in rows:
        fid = safe_id(row["segment_id"])
        wav = set_dir / "audio" / f"{fid}.wav"
        fetch_big(row["audio"][0]["src"], wav)
        ref = set_dir / "refs" / f"{fid}.txt"
        ref.parent.mkdir(parents=True, exist_ok=True)
        ref.write_text(row["segment_text"].strip() + "\n", encoding="utf-8")
        files.append(
            {
                "id": fid,
                "audio": f"audio/{fid}.wav",
                "ref": f"refs/{fid}.txt",
                "lang": "mixed",
                "setting": "other",
                "playback": "na",
                "speakers": 1,
            }
        )
    write_manifest(set_dir, "vimedcss", files)
    write_notice(set_dir, "vimedcss")


FETCHERS = {
    "fleurs-vi": fetch_fleurs,
    "fleurs-en": fetch_fleurs_en,
    "ami-sdm": fetch_ami,
    "voxconverse": fetch_voxconverse,
    "vimedcss": fetch_vimedcss,
}


def plan(name: str, subset: str, with_hyp: bool, out: Path) -> list[str]:
    """Human-readable list of what a real run would download (no network)."""
    dest = out / name
    if name in ("fleurs-vi", "fleurs-en"):
        n = "all" if subset == "full" else FLEURS_SMALL_N
        tsv, tar = (
            (FLEURS_TSV, FLEURS_TAR) if name == "fleurs-vi" else (FLEURS_EN_TSV, FLEURS_EN_TAR)
        )
        lines = [tsv, f"{tar}  (streamed, first {n} files kept)"]
    elif name == "ami-sdm":
        ids = AMI_TEST if subset == "full" else AMI_SMALL
        lines = [AMI_RTTM.format(id=i) for i in ids] + [AMI_WAV.format(id=i) for i in ids]
    elif name == "voxconverse":
        rev = VOX_REV_V002 if with_hyp else VOX_REV
        ids = (
            ["<all 232 test files>"]
            if subset == "full"
            else VOX_MANY
            if subset == "many"
            else VOX_SMALL
        )
        lines = [f"{VOX_ZIP}  (HTTP Range: only the listed files)"]
        lines += [VOX_RTTM.format(rev=rev, id=i) for i in ids]
        if with_hyp:
            lines += [f"{PUB_RTTM[0]}  (gated: needs HF_TOKEN)", f"{PUB_EVAL[0]}  (gated)"]
    else:
        n = VIMED_FULL_N if subset == "full" else VIMED_SMALL_N
        lines = [VIMED_ROWS.format(offset=0, length=n) + "  (then one wav URL per row)"]
    return [f"{name}: size {SIZES[name]}", f"  into {dest}", *[f"  - {u}" for u in lines]]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument(
        "--sets",
        default=",".join(DEFAULT_SETS),
        help=f"comma-separated: {', '.join(ALL_SETS)}, or 'all' (default: %(default)s)",
    )
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT, help="default: tools/eval/data")
    ap.add_argument(
        "--subset",
        choices=["small", "full", "many", "ctrl"],
        default="small",
        help="voxconverse only: many = 9+ speakers, ctrl = 7-8 (re-cluster eval)",
    )
    ap.add_argument(
        "--with-published-hyp",
        action="store_true",
        help="voxconverse: also write pyannote 2.1's published output as a files: hyp dir",
    )
    ap.add_argument("--dry-run", action="store_true", help="print what would be downloaded")
    args = ap.parse_args(argv)

    names = list(ALL_SETS) if args.sets == "all" else [s.strip() for s in args.sets.split(",")]
    bad = [s for s in names if s not in FETCHERS]
    if bad:
        ap.error(f"unknown set(s): {', '.join(bad)}")

    for name in names:
        log(f"== {name}")
        if args.dry_run:
            log("\n".join(plan(name, args.subset, args.with_published_hyp, args.out)))
            continue
        try:
            FETCHERS[name](args.out, args.subset, with_published_hyp=args.with_published_hyp)
        except OSError as e:  # URLError, timeouts, disk errors
            log(f"  FAILED: {e}\n  Re-run the same command; finished files are skipped.")
            return 1
        log(f"  done -> {args.out / name / 'manifest.yaml'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
