# SPDX-License-Identifier: Apache-2.0
"""PCM WAV helpers on the stdlib `wave` module (no libsndfile: LGPL)."""

from __future__ import annotations

import wave
from pathlib import Path

import numpy as np

from .errors import HarnessError


def wav_duration(path: Path) -> float:
    """Duration in seconds of a PCM WAV file. Raises HarnessError if unreadable."""
    try:
        with wave.open(str(path), "rb") as w:
            rate = w.getframerate()
            if rate <= 0:
                raise HarnessError(f"{path.name}: invalid sample rate")
            return w.getnframes() / rate
    except (wave.Error, EOFError, OSError) as exc:
        raise HarnessError(f"{path.name}: not a readable PCM WAV ({exc})") from exc


def read_mono(path: Path) -> tuple[np.ndarray, int]:
    """Read a PCM WAV as float32 mono in [-1, 1] (channels averaged) and its sample rate."""
    try:
        with wave.open(str(path), "rb") as w:
            channels, width, rate = w.getnchannels(), w.getsampwidth(), w.getframerate()
            raw = w.readframes(w.getnframes())
    except (wave.Error, EOFError, OSError) as exc:
        raise HarnessError(f"{path.name}: not a readable PCM WAV ({exc})") from exc
    if width == 1:
        data = (np.frombuffer(raw, dtype=np.uint8).astype(np.float32) - 128.0) / 128.0
    elif width == 2:
        data = np.frombuffer(raw, dtype="<i2").astype(np.float32) / 32768.0
    elif width == 3:
        b = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3)
        ints = (
            b[:, 0].astype(np.int32)
            | b[:, 1].astype(np.int32) << 8
            | b[:, 2].astype(np.int32) << 16
        )
        ints = np.where(ints >= 1 << 23, ints - (1 << 24), ints)
        data = ints.astype(np.float32) / float(1 << 23)
    elif width == 4:
        data = np.frombuffer(raw, dtype="<i4").astype(np.float32) / float(1 << 31)
    else:
        raise HarnessError(f"{path.name}: unsupported sample width {width}")
    if channels > 1:
        data = data[: len(data) - len(data) % channels].reshape(-1, channels).mean(axis=1)
    return data, rate


def write_mono16(path: Path, data: np.ndarray, rate: int) -> None:
    """Write float samples in [-1, 1] as 16-bit mono PCM WAV."""
    pcm = (np.clip(data, -1.0, 1.0) * 32767.0).astype("<i2")
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(pcm.tobytes())


def to_mono_16k(src: Path, dst: Path) -> float:
    """Convert any PCM WAV to 16 kHz mono 16-bit (what NeMo expects). Returns duration."""
    from math import gcd

    from scipy.signal import resample_poly

    data, rate = read_mono(src)
    if rate != 16000:
        g = gcd(rate, 16000)
        data = resample_poly(data, 16000 // g, rate // g).astype(np.float32)
    write_mono16(dst, data, 16000)
    return len(data) / 16000.0
