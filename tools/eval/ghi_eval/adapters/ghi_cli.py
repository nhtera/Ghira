# SPDX-License-Identifier: Apache-2.0
"""The `ghi` CLI (crates/ghi-cli) as a system under test."""

from __future__ import annotations

import json
import shutil
import subprocess
import sys

from ..contract import validation_errors
from ..errors import AdapterError, InvalidOutput, NotSupported
from ..perf import run_measured
from .base import Adapter, Measured

# ghi exit codes (formats.md §2)
EXIT_RUNTIME, EXIT_USAGE, EXIT_NOT_IMPLEMENTED = 1, 2, 3


# On Windows only a real .exe is run: launching .cmd/.bat files with arguments is unsafe
# (BatBadBut). Tests flip this to use a script launcher.
REQUIRE_EXE = sys.platform == "win32"


def resolve_binary(path: str | None) -> str:
    """`ghi` or `ghi.exe` on PATH, or an explicit path."""
    found = shutil.which(path or "ghi")
    if found is None:
        raise AdapterError(f"ghi binary not found ({path or 'ghi'} is not on PATH)")
    if REQUIRE_EXE and not found.lower().endswith(".exe"):
        raise AdapterError("ghi must be a .exe on Windows")
    return found


def parse_error_line(stderr: str) -> dict | None:
    """The last non-empty stderr line as a `ghi.error/1` document, if it is one."""
    lines = [ln for ln in stderr.splitlines() if ln.strip()]
    if not lines:
        return None
    try:
        doc = json.loads(lines[-1])
    except json.JSONDecodeError:
        return None
    if isinstance(doc, dict) and not validation_errors(doc, "error"):
        return doc
    return None


class GhiCliAdapter(Adapter):
    name = "ghi"

    def __init__(self, spec: str, binary: str | None = None):
        super().__init__(spec)
        self.binary = resolve_binary(binary)

    def _exec(self, task: str, args: list[str]):
        try:
            done = run_measured([self.binary, *args], timeout=self.timeout)
        except subprocess.TimeoutExpired as exc:
            raise AdapterError(f"ghi {args[0]} timed out after {self.timeout:g} s") from exc
        except OSError as exc:
            raise AdapterError(f"ghi {args[0]}: cannot start the binary ({exc.strerror})") from exc
        if done.returncode != 0:
            err = parse_error_line(done.stderr)
            message = err["message"] if err else f"exit code {done.returncode}, no error document"
            if done.returncode == EXIT_NOT_IMPLEMENTED:
                raise NotSupported(task, message)
            kind = "usage error" if done.returncode == EXIT_USAGE else "failed"
            raise AdapterError(f"ghi {args[0]} {kind}: {message}")
        return done

    def _json(self, task: str, args: list[str], kind: str) -> Measured[dict]:
        done = self._exec(task, args)
        try:
            doc = json.loads(done.stdout)
        except json.JSONDecodeError as exc:
            raise AdapterError(f"ghi {args[0]}: stdout is not one JSON document") from exc
        problems = validation_errors(doc, kind)
        if problems:
            raise InvalidOutput(f"ghi {args[0]}: invalid ghi.{kind}/1 ({problems[0]})", raw=doc)
        return Measured(doc, done.wall_s, done.peak_rss_mb)

    def version(self) -> str:
        doc = self._json("version", ["version", "--json"], "version").value
        return doc["ghi"]

    def transcribe(self, file_id, audio, *, lang, pass_):
        args = ["transcribe", str(audio), "--lang", lang, "--pass", pass_, "--json"]
        return self._json("asr", args, "transcript")

    def transcribe_stream(self, file_id, audio, *, lang, realtime):
        args = ["transcribe", str(audio), "--stream", "--lang", lang]
        if realtime:
            args.append("--realtime")
        done = self._exec("stream", args)
        events = []
        for n, line in enumerate(done.stdout.splitlines(), 1):
            if not line.strip():
                continue
            try:
                ev = json.loads(line)
            except json.JSONDecodeError as exc:
                raise AdapterError(f"ghi stream: line {n} is not JSON") from exc
            problems = validation_errors(ev, "event")
            if problems:
                raise InvalidOutput(f"ghi stream: line {n}: {problems[0]}", raw=ev)
            events.append(ev)
        if not events or events[-1]["type"] != "end":
            raise AdapterError("ghi stream: output ended without an 'end' event")
        return Measured(events, done.wall_s, done.peak_rss_mb)

    def diarize(self, file_id, audio, *, pass_):
        return self._json("diar", ["diarize", str(audio), "--pass", pass_, "--json"], "diarization")

    def notes(self, file_id, transcript, *, lang, source="pipeline"):
        args = ["notes", str(transcript), "--lang", lang, "--json"]
        return self._json("notes", args, "notes")
