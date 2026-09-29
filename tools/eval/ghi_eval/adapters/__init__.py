# SPDX-License-Identifier: Apache-2.0
"""Systems under test. `make_adapter("ghi:/path/ghi")` etc. (formats.md §3)."""

from __future__ import annotations

from ..errors import HarnessError
from .base import Adapter, Measured

__all__ = ["Adapter", "Measured", "make_adapter"]


def make_adapter(spec: str) -> Adapter:
    kind, _, arg = spec.partition(":")
    kind = kind.replace("_", "-")
    if kind == "ghi":
        from .ghi_cli import GhiCliAdapter

        return GhiCliAdapter(spec, arg or None)
    if kind == "files":
        from .files import FilesAdapter

        if not arg:
            raise HarnessError("files: needs a directory (files:<dir>)")
        return FilesAdapter(spec, arg)
    if kind == "nemo-ref":
        from .nemo_ref import NemoRefAdapter

        return NemoRefAdapter(spec, arg or None)
    if kind == "whisper-ref":
        from .whisper_ref import WhisperRefAdapter

        return WhisperRefAdapter(spec, arg or None)
    raise HarnessError(
        f"unknown system '{spec}' (ghi[:path], files:<dir>, nemo-ref[:cfg], whisper-ref[:model])"
    )
