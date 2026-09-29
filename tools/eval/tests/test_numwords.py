# SPDX-License-Identifier: Apache-2.0
import pytest

from ghi_eval.metrics import token_errors
from ghi_eval.textnorm import normalize

VI = {
    "5": "năm",
    "15": "mười năm",
    "21": "hai mươi một",
    "24": "hai mươi bốn",
    "25": "hai mươi năm",
    "36": "ba mươi sáu",
    "53": "năm mươi ba",
    "105": "một trăm linh năm",
    "1005": "một nghìn linh năm",  # "không trăm" dropped before "linh"
    "1967": "một nghìn chín trăm sáu mươi bảy",
    "2.000.000": "hai triệu",
    "3,5%": "ba phẩy năm phần trăm",
    "10": "mười",
    "0": "không",
    "1.000": "một nghìn",
    "1.000.005": "một triệu linh năm",
    "0,05": "không phẩy không năm",
    "3,25": "ba phẩy hai mươi năm",  # fractions read as a cardinal
    "1,000": "một nghìn",  # English-style grouping
    "10,000,000": "mười triệu",
    "1.000,5": "một nghìn phẩy năm",
}


@pytest.mark.parametrize(("digits", "words"), VI.items())
def test_vietnamese_digits_read_as_words(digits, words):
    assert normalize(digits, "vi") == words.split()
    assert normalize(f"có {digits} người", "vi") == ["có", *words.split(), "người"]


@pytest.mark.parametrize(
    ("spoken", "canonical"),
    [
        ("mười lăm", "mười năm"),
        ("hai mươi mốt", "hai mươi một"),
        ("hai mươi tư", "hai mươi bốn"),
        ("hai mươi lăm", "hai mươi năm"),
        ("một ngàn lẻ năm", "một nghìn linh năm"),
    ],
)
def test_vietnamese_variants_fold_onto_canonical(spoken, canonical):
    assert normalize(spoken, "vi") == canonical.split()


def test_mixed_uses_vietnamese_rules():
    assert normalize("deadline 15", "mixed") == ["deadline", "mười", "năm"]


EN = {
    "1,000": "one thousand",
    "1967": "one thousand nine hundred sixty seven",
    "2.5": "two point five",
    "15%": "fifteen percent",
    "105": "one hundred five",
    "2,000,000": "two million",
}


@pytest.mark.parametrize(("digits", "words"), EN.items())
def test_english_digits_read_as_words(digits, words):
    assert normalize(f"it was {digits}", "en") == ["it", "was", *words.split()]


@pytest.mark.parametrize(
    "spoken",
    [
        "one thousand nine hundred sixty seven",
        "one thousand nine hundred and sixty seven",
        "nineteen sixty seven",
    ],
)
def test_english_1967_matches_year_and_cardinal_readings(spoken):
    assert normalize(spoken, "en") == normalize("1967", "en")
    assert token_errors("in 1967", f"in {spoken}", "en").errors == 0


def test_english_years_and_and():
    assert normalize("twenty twenty six", "en") == normalize("2026", "en")
    assert normalize("one hundred and five", "en") == normalize("105", "en")
    assert normalize("nineteen oh five", "en") == normalize("1905", "en")
    assert normalize("one two three", "en") == ["one", "two", "three"]  # not a cardinal


@pytest.mark.parametrize(
    "text", ["lúc 10:30 nhé", "mã abc123", "số 0912345678", "ngày 12/03", "v1.5.3"]
)
def test_other_digit_strings_stay_digits(text):
    assert any(any(c.isdigit() for c in t) for t in normalize(text, "vi"))
    assert "".join(normalize(text, "vi")) == "".join(normalize(text))


def test_without_language_numbers_are_untouched():
    assert normalize("có 53 người") == ["có", "53", "người"]


def test_fleurs_round_trip_has_zero_errors():
    ref = "trận này đã giúp đội tuyển kết thúc chuỗi thua 5 trận liền"
    hyp = "trận này đã giúp đội tuyển kết thúc chuỗi thua năm trận liền"
    assert token_errors(ref, hyp, "vi").errors == 0
    assert token_errors("53 tuổi", "năm mươi ba tuổi", "vi").errors == 0
    assert token_errors("năm 1967", "năm một nghìn chín trăm sáu mươi bảy", "vi").errors == 0
    assert token_errors(ref, hyp).errors == 1  # without a language the digit still costs


def test_vietnamese_comma_rules():
    assert normalize("3,250", "vi") == normalize("3.250", "vi")  # exactly 3 digits: grouping
    assert normalize("3,2", "vi")[1] == "phẩy" and normalize("3,2500", "vi")[1] == "phẩy"


@pytest.mark.parametrize(
    ("a", "b"),
    [
        ("một nghìn lẻ năm", "một nghìn không trăm linh năm"),
        ("một nghìn linh năm", "1005"),
        ("hai mốt", "21"),
        ("hai tư", "24"),
        ("hai lăm", "25"),
        ("ba lăm", "35"),
        ("chín mốt", "91"),
    ],
)
def test_vietnamese_number_variants_agree(a, b):
    assert normalize(a, "vi") == normalize(b, "vi")


def test_tens_without_muoi_only_before_colloquial_units():
    assert normalize("ba bốn người", "vi") == ["ba", "bốn", "người"]  # 3-4 people, not 34
    assert normalize("một mốt", "vi") == ["một", "một"]  # only 2-9 take the implicit "mươi"


def test_ranges_stay_digits_on_both_sides():
    for lang in ("en", "vi"):
        assert normalize("5-10 people", lang)[:2] == ["5", "10"]
        assert normalize("5 - 10", lang) != ["5", "10"]  # spaced: both ends are read
    assert token_errors("from 5-10", "from 5-10", "en").errors == 0


def test_english_and_inside_numbers():
    assert normalize("two thousand and five", "en") == normalize("2005", "en")
    assert normalize("one hundred and twenty three", "en") == normalize("123", "en")
    assert normalize("bread and butter", "en") == ["bread", "and", "butter"]
    assert normalize("two and a half", "en") == ["two", "and", "a", "half"]
