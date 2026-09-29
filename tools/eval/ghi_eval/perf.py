# SPDX-License-Identifier: Apache-2.0
"""Harness-side wall time and peak RSS measurement."""

from __future__ import annotations

import subprocess
import threading
import time
from dataclasses import dataclass

import psutil

POLL_S = 0.05
MB = 1024 * 1024


@dataclass
class Completed:
    returncode: int
    stdout: str
    stderr: str
    wall_s: float
    peak_rss_mb: float | None


def _tree_rss(proc: psutil.Process) -> int:
    total = 0
    try:
        procs = [proc, *proc.children(recursive=True)]
    except psutil.Error:
        return 0
    for p in procs:
        try:
            total += p.memory_info().rss
        except psutil.Error:
            pass
    return total


class _Sampler(threading.Thread):
    """Polls the RSS of a process tree every POLL_S and keeps the maximum."""

    def __init__(self, proc: psutil.Process):
        super().__init__(daemon=True)
        self.proc = proc
        self.peak = 0
        self._stop_event = threading.Event()

    def run(self) -> None:
        while not self._stop_event.is_set():
            self.peak = max(self.peak, _tree_rss(self.proc))
            self._stop_event.wait(POLL_S)

    def finish(self) -> None:
        self._stop_event.set()
        self.join()
        self.peak = max(self.peak, _tree_rss(self.proc))


class PeakRSS:
    """Context manager: peak RSS of the *current* process while the block runs (in-process adapters)."""

    def __init__(self) -> None:
        self.peak_rss_mb: float | None = None
        self.wall_s = 0.0
        self._sampler: _Sampler | None = None

    def __enter__(self) -> PeakRSS:
        self._t0 = time.perf_counter()
        self._sampler = _Sampler(psutil.Process())
        self._sampler.start()
        return self

    def __exit__(self, *exc) -> None:
        assert self._sampler is not None
        self._sampler.finish()
        self.wall_s = time.perf_counter() - self._t0
        self.peak_rss_mb = round(self._sampler.peak / MB, 1) if self._sampler.peak else None


def kill_tree(proc: subprocess.Popen) -> None:
    """Kill a process and everything it spawned."""
    try:
        victims = [*psutil.Process(proc.pid).children(recursive=True)]
    except psutil.Error:
        victims = []
    for p in victims:
        try:
            p.kill()
        except psutil.Error:
            pass
    proc.kill()


def run_measured(cmd: list[str], *, timeout: float | None = None) -> Completed:
    """Run `cmd` (no shell), capturing text output, wall time and peak RSS of the process tree.

    Raises subprocess.TimeoutExpired (after killing the whole tree) past `timeout` seconds,
    and OSError if the program cannot be started.
    """
    t0 = time.perf_counter()
    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    sampler = _Sampler(psutil.Process(proc.pid))
    sampler.start()
    try:
        out, err = proc.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        kill_tree(proc)
        proc.communicate()
        raise
    finally:
        sampler.finish()
    wall = time.perf_counter() - t0
    peak = round(sampler.peak / MB, 1) if sampler.peak else None
    return Completed(proc.returncode, out, err, wall, peak)


def rtf(wall_s: float, audio_s: float | None) -> float | None:
    return wall_s / audio_s if audio_s else None
