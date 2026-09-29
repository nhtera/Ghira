# SPDX-License-Identifier: Apache-2.0
from __future__ import annotations

import shutil
import stat
import sys
import wave
from pathlib import Path

import pytest

FIXTURES = Path(__file__).parent / "fixtures"
CLI_GOLDEN = FIXTURES / "cli"


def write_silence(path: Path, seconds: float, rate: int = 8000) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(b"\x00\x00" * int(seconds * rate))


@pytest.fixture
def tiny(tmp_path: Path) -> Path:
    """A writable copy of the committed tiny dataset (no audio) and its `files:` hypotheses."""
    ds = tmp_path / "tiny"
    shutil.copytree(FIXTURES / "tiny", ds)
    shutil.copytree(FIXTURES / "tiny-hyp", tmp_path / "tiny-hyp")
    return ds


@pytest.fixture
def tiny_hyp(tiny: Path) -> Path:
    return tiny.parent / "tiny-hyp"


FAKE_GHI = """\
import os, sys
from pathlib import Path

GOLD = Path(__GOLD__)
mode = os.environ.get("FAKE_GHI_MODE", "ok")
args = sys.argv[1:]
print("fake ghi log line", file=sys.stderr)
if args[0] != "version":
    if mode == "notimpl":
        print('{"schema":"ghi.error/1","code":"not_implemented","message":"' + args[0] + ': no speech engine yet (phase 3)"}', file=sys.stderr)
        sys.exit(3)
    if mode == "fail":
        print('{"schema":"ghi.error/1","code":"internal","message":"boom"}', file=sys.stderr)
        sys.exit(1)
    if mode == "hang":
        import time
        time.sleep(60)
    if mode == "badjson":
        print("not json")
        sys.exit(0)
    if mode == "badschema":
        print('{"schema":"ghi.notes/1"}')
        sys.exit(0)
name = {"version": "version.json", "diarize": "diarization.json", "notes": "notes.json",
        "bench": "bench.json"}.get(args[0])
if args[0] == "transcribe":
    name = "events.ndjson" if "--stream" in args else "transcript.json"
sys.stdout.write((GOLD / name).read_text(encoding="utf-8"))
"""


@pytest.fixture
def fake_ghi(tmp_path: Path, monkeypatch) -> Path:
    """A `ghi` stand-in that replays the golden documents; behaviour set by FAKE_GHI_MODE."""
    from ghi_eval.adapters import ghi_cli

    monkeypatch.setattr(ghi_cli, "REQUIRE_EXE", False)  # the fake is a script, not a .exe
    script = tmp_path / "fake_ghi.py"
    script.write_text(FAKE_GHI.replace("__GOLD__", repr(str(CLI_GOLDEN))), encoding="utf-8")
    if sys.platform == "win32":
        launcher = tmp_path / "ghi.cmd"
        launcher.write_text(f'@"{sys.executable}" "{script}" %*\r\n', encoding="utf-8")
    else:
        launcher = tmp_path / "ghi"
        launcher.write_text(
            f'#!{sys.executable}\nimport runpy, sys\nsys.argv[0] = {str(script)!r}\nrunpy.run_path({str(script)!r}, run_name="__main__")\n',
            encoding="utf-8",
        )
        launcher.chmod(launcher.stat().st_mode | stat.S_IEXEC)
    return launcher
