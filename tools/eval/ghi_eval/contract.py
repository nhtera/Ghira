# SPDX-License-Identifier: Apache-2.0
"""JSON Schema validation of the `ghi` CLI documents and the eval report."""

from __future__ import annotations

import json
from functools import cache
from pathlib import Path
from typing import Any

from jsonschema import Draft202012Validator

SCHEMA_DIR = Path(__file__).parent / "schemas"

KINDS = ("transcript", "diarization", "notes", "bench", "version", "event", "error", "eval-report")


@cache
def _validator(kind: str) -> Draft202012Validator:
    if kind not in KINDS:
        raise ValueError(f"unknown schema kind: {kind}")
    schema = json.loads((SCHEMA_DIR / f"{kind}.schema.json").read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator(schema)


def validation_errors(doc: Any, kind: str) -> list[str]:
    """Human-readable problems of `doc` against schema `kind` (empty list = valid).

    Messages carry the JSON path but not the offending value, so they never echo text.
    """
    errors = []
    for err in sorted(
        _validator(kind).iter_errors(doc), key=lambda e: list(map(str, e.absolute_path))
    ):
        path = "/".join(str(p) for p in err.absolute_path) or "<root>"
        errors.append(f"{path}: {err.validator} check failed")
    if not errors and kind == "transcript":
        errors.extend(_transcript_extra(doc))
    return errors


def _transcript_extra(doc: dict) -> list[str]:
    errors = []
    ids = [s["id"] for s in doc["segments"]]
    if len(set(ids)) != len(ids):
        errors.append("segments: ids are not unique")
    for s in doc["segments"]:
        if s["end"] < s["start"]:
            errors.append(f"segments/{s['id']}: end before start")
    return errors


def is_valid(doc: Any, kind: str) -> bool:
    return not validation_errors(doc, kind)
