# SPDX-License-Identifier: Apache-2.0
import json
import xml.etree.ElementTree as ET

import pytest
from conftest import write_silence

from ghi_eval.cli import main
from ghi_eval.convert import (
    assign_speaker,
    audacity_to_turns,
    build_eaf,
    parse_eaf,
    turns_to_audacity,
)
from ghi_eval.errors import HarnessError
from ghi_eval.rttm import Turn, read_rttm

EAF = """<?xml version="1.0" encoding="UTF-8"?>
<ANNOTATION_DOCUMENT AUTHOR="" DATE="2026-01-01T00:00:00+00:00" FORMAT="3.0" VERSION="3.0">
  <HEADER MEDIA_FILE="" TIME_UNITS="milliseconds"/>
  <TIME_ORDER>
    <TIME_SLOT TIME_SLOT_ID="ts1" TIME_VALUE="1200"/>
    <TIME_SLOT TIME_SLOT_ID="ts2" TIME_VALUE="3500"/>
    <TIME_SLOT TIME_SLOT_ID="ts3" TIME_VALUE="500"/>
    <TIME_SLOT TIME_SLOT_ID="ts4" TIME_VALUE="900"/>
    <TIME_SLOT TIME_SLOT_ID="ts5"/>
  </TIME_ORDER>
  <TIER LINGUISTIC_TYPE_REF="default-lt" TIER_ID="tier-a" PARTICIPANT="Speaker One">
    <ANNOTATION><ALIGNABLE_ANNOTATION ANNOTATION_ID="a1" TIME_SLOT_REF1="ts1" TIME_SLOT_REF2="ts2">
      <ANNOTATION_VALUE>Mình chốt
 scope nhé.</ANNOTATION_VALUE></ALIGNABLE_ANNOTATION></ANNOTATION>
    <ANNOTATION><ALIGNABLE_ANNOTATION ANNOTATION_ID="a4" TIME_SLOT_REF1="ts4" TIME_SLOT_REF2="ts5">
      <ANNOTATION_VALUE>unaligned, skipped</ANNOTATION_VALUE></ALIGNABLE_ANNOTATION></ANNOTATION>
  </TIER>
  <TIER LINGUISTIC_TYPE_REF="default-lt" TIER_ID="spk2">
    <ANNOTATION><ALIGNABLE_ANNOTATION ANNOTATION_ID="a2" TIME_SLOT_REF1="ts3" TIME_SLOT_REF2="ts4">
      <ANNOTATION_VALUE>Okay.</ANNOTATION_VALUE></ALIGNABLE_ANNOTATION></ANNOTATION>
  </TIER>
  <TIER LINGUISTIC_TYPE_REF="ref-lt" TIER_ID="notes" PARENT_REF="spk2">
    <ANNOTATION><REF_ANNOTATION ANNOTATION_ID="a3" ANNOTATION_REF="a2">
      <ANNOTATION_VALUE>ignored</ANNOTATION_VALUE></REF_ANNOTATION></ANNOTATION>
  </TIER>
</ANNOTATION_DOCUMENT>
"""


def test_eaf_round_trip(tmp_path):
    eaf = tmp_path / "x.eaf"
    eaf.write_text(EAF, encoding="utf-8")
    rows = parse_eaf(eaf)
    assert rows == [
        (0.5, 0.9, "spk2", "Okay."),
        (1.2, 3.5, "Speaker_One", "Mình chốt scope nhé."),
    ]
    assert main(["convert", "eaf", str(eaf), "--id", "m9", "--dataset", str(tmp_path / "ds")]) == 0
    turns = read_rttm(tmp_path / "ds" / "labels" / "m9.rttm", "m9")
    assert [(t.start, t.end, t.speaker) for t in turns] == [
        (0.5, 0.9, "spk2"),
        (1.2, 3.5, "Speaker_One"),
    ]
    lines = (tmp_path / "ds" / "refs" / "m9.txt").read_text(encoding="utf-8").splitlines()
    assert lines == ["Okay.", "Mình chốt scope nhé."]


def test_eaf_bad_file(tmp_path):
    p = tmp_path / "bad.eaf"
    p.write_text("<nope", encoding="utf-8")
    with pytest.raises(HarnessError):
        parse_eaf(p)


def test_audacity_round_trip(tmp_path):
    labels = tmp_path / "l.txt"
    labels.write_text(
        "0.5\t2.0\tspk 1\n\\\t100.0\t2000.0\n3.000000\t4.5\tspk2\n\n", encoding="utf-8"
    )
    turns = audacity_to_turns(labels)
    assert turns == [Turn(0.5, 2.0, "spk_1"), Turn(3.0, 4.5, "spk2")]
    assert turns_to_audacity(turns) == "0.500000\t2.000000\tspk_1\n3.000000\t4.500000\tspk2\n"
    assert (
        main(["convert", "audacity", str(labels), "--id", "z", "--rttm", str(tmp_path / "z.rttm")])
        == 0
    )
    out = tmp_path / "z.labels.txt"
    assert main(["convert", "rttm-audacity", str(tmp_path / "z.rttm"), "--out", str(out)]) == 0
    assert audacity_to_turns(out) == turns
    labels.write_text("1.0 2.0\n", encoding="utf-8")
    with pytest.raises(HarnessError):
        audacity_to_turns(labels)


def test_assign_speaker_max_overlap():
    turns = [Turn(0, 5, "A"), Turn(5, 20, "B")]
    assert assign_speaker(3, 9, turns, "?") == "B"  # 4 s of B beats 2 s of A
    assert assign_speaker(30, 31, turns, "fallback") == "fallback"


def test_draft_eaf_valid_and_reloadable(tiny, tiny_hyp, tmp_path):
    write_silence(tiny / "audio" / "t01.wav", 30.0)
    out = tmp_path / "out"
    assert (
        main(
            [
                "run",
                "--dataset",
                str(tiny),
                "--system",
                f"files:{tiny_hyp}",
                "--run-id",
                "d",
                "--tasks",
                "asr,diar",
                "--out",
                str(out),
            ]
        )
        == 0
    )
    drafts = tmp_path / "drafts"
    assert (
        main(
            [
                "convert",
                "draft-eaf",
                "--run",
                str(out / "d"),
                "--dataset",
                str(tiny),
                "--out",
                str(drafts),
            ]
        )
        == 0
    )
    eaf = drafts / "t01.eaf"
    root = ET.parse(eaf).getroot()  # valid XML
    assert root.tag == "ANNOTATION_DOCUMENT"
    md = root.find("./HEADER/MEDIA_DESCRIPTOR")
    assert md.get("MEDIA_URL").startswith("file://") and md.get("MEDIA_URL").endswith("t01.wav")
    assert md.get("RELATIVE_MEDIA_URL").endswith("audio/t01.wav")
    assert {t.get("TIER_ID") for t in root.iterfind("TIER")} == {"S1", "S2"}
    rows = parse_eaf(eaf)  # corrections in ELAN come back through `convert eaf`
    assert [(r[2], r[3]) for r in rows] == [
        ("S1", "Mình chốt scope cho beta nhé."),
        ("S2", "Ok Linh sẽ gửi tài liệu scope hôm nay"),
        ("S1", "Còn lịch họp tuần sau thì sao"),
    ]


def test_build_eaf_clamps_overlap_in_tier(tmp_path):
    tree = build_eaf([(0, 5, "A", "one"), (4, 8, "A", "two")], tmp_path / "a.wav", tmp_path)
    path = tmp_path / "o.eaf"
    tree.write(path, encoding="utf-8", xml_declaration=True)
    rows = parse_eaf(path)
    assert rows[0][1] <= rows[1][0]
    json.dumps(rows)
