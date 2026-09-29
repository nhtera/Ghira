# SPDX-License-Identifier: Apache-2.0
"""Digits to spoken words (Vietnamese, English) so references and hypotheses compare equal.

Both sides of a comparison go through the same functions: digit strings are read out in
a canonical way and spoken-number variants are folded onto that reading.

Vietnamese canonical reading uses only the plain units (không một hai ba bốn năm sáu bảy
tám chín), `mươi`, `trăm`, `linh`, `nghìn`, `triệu`, `tỷ`; the variants ngàn, lẻ, tư, mốt,
lăm are mapped onto nghìn, linh, bốn, một, năm as whole tokens.

English canonical reading is the cardinal ("one thousand nine hundred sixty seven", no
"and"). Years written in words ("nineteen sixty seven") are folded onto the cardinal, so
a reference "1967" matches either spoken form.
"""

from __future__ import annotations

import re

MAX_DIGITS = 12

# ------------------------------------------------------------------ Vietnamese
VI_UNITS = "không một hai ba bốn năm sáu bảy tám chín".split()
VI_SCALES = ["", "nghìn", "triệu", "tỷ"]
VI_VARIANTS = {"ngàn": "nghìn", "lẻ": "linh", "tư": "bốn", "mốt": "một", "lăm": "năm"}


def _vi_group(g: int, full: bool) -> list[str]:
    """Read 0 < g < 1000. `full` = a higher non-zero group exists, so hundreds are spoken."""
    h, t, u = g // 100, g // 10 % 10, g % 10
    out: list[str] = []
    if h or full:
        out += [VI_UNITS[h], "trăm"]
    if t == 0:
        if u:
            out += (["linh"] if (h or full) else []) + [VI_UNITS[u]]
    elif t == 1:
        out += ["mười"] + ([VI_UNITS[u]] if u else [])
    else:
        out += [VI_UNITS[t], "mươi"] + ([VI_UNITS[u]] if u else [])
    return out


def vi_words(n: int) -> list[str]:
    if n == 0:
        return ["không"]
    if n >= 10**9:
        return vi_words(n // 10**9) + ["tỷ"] + (vi_words(n % 10**9) if n % 10**9 else [])
    groups = []
    while n:
        groups.append(n % 1000)
        n //= 1000
    out: list[str] = []
    for i in range(len(groups) - 1, -1, -1):
        if groups[i]:
            out += _vi_group(groups[i], full=i < len(groups) - 1 and bool(out)) + (
                [VI_SCALES[i]] if VI_SCALES[i] else []
            )
    return out


# ------------------------------------------------------------------ English
EN_UNITS = (
    "zero one two three four five six seven eight nine ten eleven twelve thirteen fourteen "
    "fifteen sixteen seventeen eighteen nineteen"
).split()
EN_TENS = "_ _ twenty thirty forty fifty sixty seventy eighty ninety".split()
EN_SCALES = [(10**9, "billion"), (10**6, "million"), (1000, "thousand")]
EN_VALUE = {w: i for i, w in enumerate(EN_UNITS)}
EN_VALUE.update({w: 10 * i for i, w in enumerate(EN_TENS) if w != "_"})


def _en_below_1000(g: int) -> list[str]:
    out: list[str] = []
    if g >= 100:
        out += [EN_UNITS[g // 100], "hundred"]
        g %= 100
    if g >= 20:
        out.append(EN_TENS[g // 10])
        g %= 10
        if g:
            out.append(EN_UNITS[g])
    elif g or not out:
        out.append(EN_UNITS[g])
    return out


def en_words(n: int) -> list[str]:
    if n == 0:
        return ["zero"]
    out: list[str] = []
    for value, name in EN_SCALES:
        if n >= value:
            out += _en_below_1000(n // value) + [name]
            n %= value
    if n:
        out += _en_below_1000(n)
    return out


def _parse_below_1000(tokens: list[str], i: int) -> tuple[int, int] | None:
    """Longest cardinal in 1..999 starting at i (optional 'and' after hundred)."""
    start, total, cur = i, 0, None
    if i < len(tokens) and tokens[i] in EN_VALUE and EN_VALUE[tokens[i]] < 20:
        cur = EN_VALUE[tokens[i]]
        i += 1
    elif i < len(tokens) and tokens[i] in EN_VALUE:
        cur = EN_VALUE[tokens[i]]
        i += 1
        if i < len(tokens) and tokens[i] in EN_VALUE and 0 < EN_VALUE[tokens[i]] < 10:
            cur += EN_VALUE[tokens[i]]
            i += 1
    if cur is not None and i < len(tokens) and tokens[i] == "hundred" and cur < 10:
        total, cur = cur * 100, None
        i += 1
        j = i + 1 if i < len(tokens) and tokens[i] == "and" else i
        sub = _parse_below_1000(tokens, j) if j < len(tokens) else None
        if sub and sub[0] < 100:
            total += sub[0]
            i = sub[1]
        return total, i
    if cur is None:
        return None
    return cur, i if i > start else start


def _parse_cardinal(tokens: list[str], i: int) -> tuple[int, int] | None:
    """Longest cardinal number starting at i, as (value, next index)."""
    total, j, seen = 0, i, False
    for value, name in EN_SCALES:
        got = _parse_below_1000(tokens, j)
        if got and got[1] < len(tokens) and tokens[got[1]] == name:
            total += got[0] * value
            j = got[1] + 1
            seen = True
            # "two thousand and five": drop the "and" when a number follows it
            if j + 1 < len(tokens) and tokens[j] == "and" and _parse_below_1000(tokens, j + 1):
                j += 1
    got = _parse_below_1000(tokens, j)
    if got:
        total += got[0]
        j = got[1]
        seen = True
    return (total, j) if seen else None


def _parse_year(tokens: list[str], i: int) -> tuple[int, int] | None:
    """'nineteen sixty seven', 'twenty twenty', 'nineteen oh five' -> 1967, 2020, 1905."""
    if i >= len(tokens) or tokens[i] not in EN_VALUE or not 11 <= EN_VALUE[tokens[i]] <= 20:
        return None
    hi = EN_VALUE[tokens[i]]
    if hi == 20 and not (
        i + 1 < len(tokens) and (tokens[i + 1] in EN_VALUE or tokens[i + 1] == "oh")
    ):
        return None
    j = i + 1
    if j < len(tokens) and tokens[j] == "oh":
        if j + 1 < len(tokens) and 1 <= EN_VALUE.get(tokens[j + 1], 0) <= 9:
            return hi * 100 + EN_VALUE[tokens[j + 1]], j + 2
        return None
    lo = _parse_below_1000(tokens, j)
    if lo and 10 <= lo[0] <= 99:
        return hi * 100 + lo[0], lo[1]
    return None


def en_fold(tokens: list[str]) -> list[str]:
    """Rewrite runs of English number words as the canonical cardinal ('and' dropped)."""
    out: list[str] = []
    i = 0
    while i < len(tokens):
        got = _parse_year(tokens, i) or _parse_cardinal(tokens, i)
        # a bare "and" or a lone zero-like token is never rewritten
        if got and got[1] > i:
            out += en_words(got[0])
            i = got[1]
        else:
            out.append(tokens[i])
            i += 1
    return out


# ------------------------------------------------------------------ digit spans in text
_NUM = {
    # "1.000,5" dot grouping with a comma decimal; "1,000" / "10,000,000" (a comma followed by
    # exactly three digits, repeating) is English-style grouping an engine may emit; any other
    # comma is a decimal.
    "vi": re.compile(
        r"\d{1,3}(?:\.\d{3})+(?:,\d+)?%?|\d{1,3}(?:,\d{3})+(?![\d,])%?|\d+(?:,\d+)?%?"
    ),
    "en": re.compile(r"\d{1,3}(?:,\d{3})+(?:\.\d+)?%?|\d+(?:\.\d+)?%?"),
}
_DEC = {"vi": ",", "en": "."}
_SEP = {"vi": ".", "en": ","}
_WORDS = {"vi": (vi_words, "phẩy", "phần trăm"), "en": (en_words, "point", "percent")}
_DIGIT_WORDS = {"vi": VI_UNITS, "en": EN_UNITS[:10]}


def _standalone(text: str, start: int, end: int) -> bool:
    """A number that is not part of a time, id, date, range or longer digit string."""
    before = text[start - 1] if start else ""
    after = text[end] if end < len(text) else ""
    after2 = text[end + 1] if end + 1 < len(text) else ""
    before2 = text[start - 2] if start > 1 else ""
    if before and (before.isalnum() or before in "_:/"):
        return False
    if before in ".," and before2.isdigit():
        return False
    if before == "-" and before2.isdigit():  # range "5-10": both ends stay digits
        return False
    if after and (after.isalnum() or after == "_"):
        return False
    if after in ":/.,-" and after2.isdigit():
        return False
    return True


def read_number(token: str, lang: str) -> str | None:
    """Spoken words for a digit token like '1.000', '3,5%' or '53', or None to leave it."""
    words, dec_word, pct_word = _WORDS[lang]
    pct = token.endswith("%")
    body = token.rstrip("%")
    if lang == "vi" and re.fullmatch(r"\d{1,3}(?:,\d{3})+", body):
        body = body.replace(",", "")  # English-style grouping, not a decimal
    whole, _, frac = body.partition(_DEC[lang])
    whole = whole.replace(_SEP[lang], "")
    if not whole.isdigit() or len(whole) > MAX_DIGITS or (len(whole) > 1 and whole[0] == "0"):
        return None
    out = list(words(int(whole)))
    if frac and lang == "vi":  # 3,25 -> ba phẩy hai mươi lăm; leading zeros one by one
        rest = frac.lstrip("0")
        out += [dec_word] + [VI_UNITS[0]] * (len(frac) - len(rest))
        out += words(int(rest)) if rest else []
    elif frac:
        out += [dec_word] + [_DIGIT_WORDS[lang][int(d)] for d in frac]
    if pct:
        out += pct_word.split()
    return " ".join(out)


def expand_digits(text: str, lang: str) -> str:
    """Replace standalone numbers in `text` (already lowercased) with their spoken words."""
    pattern = _NUM[lang]
    out, last = [], 0
    for m in pattern.finditer(text):
        if not _standalone(text, m.start(), m.end()):
            continue
        spoken = read_number(m.group(), lang)
        if spoken is None:
            continue
        out += [text[last : m.start()], " ", spoken, " "]
        last = m.end()
    out.append(text[last:])
    return "".join(out)


_VI_TENS_DIGITS = frozenset(VI_UNITS[2:])
_VI_COLLOQUIAL = frozenset({"mốt", "tư", "lăm"})


def fold_tokens_vi(tokens: list[str]) -> list[str]:
    """Fold spoken variants onto the canonical reading.

    - "hai mốt / hai tư / hai lăm" (tens without "mươi") gain the "mươi": both tokens must be
      number words, the second one a colloquial unit;
    - the variants ngàn, lẻ, tư, mốt, lăm map to nghìn, linh, bốn, một, năm;
    - "không trăm linh" (1005 read in full) drops "không trăm", like "một nghìn lẻ năm".
    """
    spread: list[str] = []
    for i, t in enumerate(tokens):
        spread.append(t)
        if t in _VI_TENS_DIGITS and i + 1 < len(tokens) and tokens[i + 1] in _VI_COLLOQUIAL:
            spread.append("mươi")
    mapped = [VI_VARIANTS.get(t, t) for t in spread]
    out: list[str] = []
    i = 0
    while i < len(mapped):
        if mapped[i : i + 3] == ["không", "trăm", "linh"]:
            i += 2
            continue
        out.append(mapped[i])
        i += 1
    return out
