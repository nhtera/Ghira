# SPDX-License-Identifier: Apache-2.0
"""Token normalization shared by scoring and the privacy lint."""

from __future__ import annotations

import re
import unicodedata

_BRACKETS = re.compile(r"\[[^\]]*\]")
_APOSTROPHES = "'’ʼ‘`"


def normalize(text: str) -> list[str]:
    """NFC, lowercase, drop `[...]` spans, strip punctuation and symbols, split on whitespace.

    Apostrophes are deleted (don't -> dont); every other Unicode punctuation (P*) or
    symbol (S*) character becomes a space. A Vietnamese syllable is one token.
    """
    text = unicodedata.normalize("NFC", text).lower()
    text = _BRACKETS.sub(" ", text)
    out = []
    for ch in text:
        if ch in _APOSTROPHES:
            continue
        cat = unicodedata.category(ch)
        out.append(" " if cat[0] in "PSZC" or ch.isspace() else ch)
    return "".join(out).split()


def fold(text: str) -> str:
    """Lowercase and strip diacritics (đ -> d), for tolerant name matching."""
    text = unicodedata.normalize("NFD", text.lower().replace("đ", "d").replace("Đ", "d"))
    return unicodedata.normalize("NFC", "".join(c for c in text if unicodedata.category(c) != "Mn"))


def fold_tokens(text: str) -> list[str]:
    return [fold(t) for t in normalize(text)]
