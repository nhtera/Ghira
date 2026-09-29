# SPDX-License-Identifier: Apache-2.0
"""Label converters: ELAN .eaf, Audacity label tracks and RTTM (all local, offline)."""

from __future__ import annotations

import json
import os
import unicodedata
import xml.etree.ElementTree as ET
from datetime import UTC, datetime
from pathlib import Path

from .errors import HarnessError
from .rttm import Turn, clean_label, format_rttm, read_rttm


def _nfc(s: str) -> str:
    return unicodedata.normalize("NFC", s)


# ---------------------------------------------------------------- ELAN -> RTTM + text


def parse_eaf(path: Path) -> list[tuple[float, float, str, str]]:
    """(start, end, speaker, text) for every aligned annotation, sorted by start.

    Speaker = the tier's PARTICIPANT if set, else its TIER_ID. Only ALIGNABLE_ANNOTATION
    with both time slots resolved is used; REF_ANNOTATION (dependent tiers) is ignored.
    """
    try:
        root = ET.parse(path).getroot()
    except (ET.ParseError, OSError) as exc:
        raise HarnessError(f"{path.name}: not a readable .eaf ({exc})") from exc
    slots: dict[str, float] = {}
    for ts in root.iterfind("./TIME_ORDER/TIME_SLOT"):
        if ts.get("TIME_VALUE") is not None:
            try:
                slots[ts.get("TIME_SLOT_ID", "")] = int(ts.get("TIME_VALUE")) / 1000.0
            except ValueError as exc:
                raise HarnessError(f"{path.name}: TIME_VALUE is not an integer") from exc
    out = []
    for tier in root.iterfind("./TIER"):
        speaker = clean_label(tier.get("PARTICIPANT") or tier.get("TIER_ID") or "")
        if not speaker:
            continue
        for ann in tier.iterfind("./ANNOTATION/ALIGNABLE_ANNOTATION"):
            a, b = (
                slots.get(ann.get("TIME_SLOT_REF1", "")),
                slots.get(ann.get("TIME_SLOT_REF2", "")),
            )
            if a is None or b is None or b < a:
                continue
            value = ann.findtext("ANNOTATION_VALUE") or ""
            text = _nfc(" ".join(value.split()))
            if text:
                out.append((a, b, speaker, text))
    out.sort(key=lambda r: (r[0], r[1], r[2]))
    return out


def eaf_to_files(eaf: Path, file_id: str, rttm_out: Path, ref_out: Path) -> tuple[int, int]:
    """Write RTTM and reference text from an .eaf. Returns (turns, speakers)."""
    rows = parse_eaf(eaf)
    rttm_out.parent.mkdir(parents=True, exist_ok=True)
    ref_out.parent.mkdir(parents=True, exist_ok=True)
    rttm_out.write_text(
        format_rttm(file_id, [Turn(a, b, s) for a, b, s, _ in rows]), encoding="utf-8", newline="\n"
    )
    ref_out.write_text("".join(t + "\n" for *_, t in rows), encoding="utf-8", newline="\n")
    return len(rows), len({s for _, _, s, _ in rows})


# ---------------------------------------------------------------- Audacity <-> RTTM


def audacity_to_turns(path: Path) -> list[Turn]:
    """Audacity label track: `start<TAB>end<TAB>label`; the label is the speaker."""
    turns = []
    for n, line in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        if not line.strip() or line.startswith("\\"):  # blank, or spectral-selection line
            continue
        parts = line.split("\t", 2)
        if len(parts) < 3:
            raise HarnessError(f"{path.name}:{n}: expected start<TAB>end<TAB>label")
        try:
            start, end = float(parts[0]), float(parts[1])
        except ValueError as exc:
            raise HarnessError(f"{path.name}:{n}: bad time") from exc
        label = clean_label(parts[2])
        if end > start and label:
            turns.append(Turn(start, end, label))
    return turns


def turns_to_audacity(turns: list[Turn]) -> str:
    return "".join(
        f"{t.start:.6f}\t{t.end:.6f}\t{t.speaker}\n"
        for t in sorted(turns, key=lambda t: (t.start, t.end, t.speaker))
    )


def rttm_to_audacity(rttm: Path, out: Path) -> int:
    turns = read_rttm(rttm)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(turns_to_audacity(turns), encoding="utf-8", newline="\n")
    return len(turns)


# ---------------------------------------------------------------- run -> draft .eaf


def assign_speaker(start: float, end: float, turns: list[Turn], fallback: str) -> str:
    """Speaker of the RTTM turn overlapping [start, end] the most (ties: earliest label)."""
    best, best_ov = fallback, 0.0
    overlap: dict[str, float] = {}
    for t in turns:
        ov = min(end, t.end) - max(start, t.start)
        if ov > 0:
            overlap[t.speaker] = overlap.get(t.speaker, 0.0) + ov
    for spk in sorted(overlap):
        if overlap[spk] > best_ov:
            best, best_ov = spk, overlap[spk]
    return best


def build_eaf(
    segments: list[tuple[float, float, str, str]], audio: Path, eaf_dir: Path
) -> ET.ElementTree:
    """An ELAN document with one tier per speaker; `segments` = (start, end, speaker, text)."""
    doc = ET.Element(
        "ANNOTATION_DOCUMENT",
        {
            "AUTHOR": "", "DATE": datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%S+00:00"),
            "FORMAT": "3.0", "VERSION": "3.0",
            "xmlns:xsi": "http://www.w3.org/2001/XMLSchema-instance",
            "xsi:noNamespaceSchemaLocation": "http://www.mpi.nl/tools/elan/EAFv3.0.xsd",
        },
    )  # fmt: skip
    audio = Path(audio).resolve()
    try:
        rel = Path(os.path.relpath(audio, Path(eaf_dir).resolve())).as_posix()
    except ValueError:  # different drive on Windows
        rel = audio.name
    header = ET.SubElement(doc, "HEADER", {"MEDIA_FILE": "", "TIME_UNITS": "milliseconds"})
    ET.SubElement(
        header, "MEDIA_DESCRIPTOR",
        {"MEDIA_URL": audio.as_uri(), "MIME_TYPE": "audio/x-wav", "RELATIVE_MEDIA_URL": rel},
    )  # fmt: skip
    order = ET.SubElement(doc, "TIME_ORDER")
    tiers: dict[str, ET.Element] = {}
    last_end: dict[str, int] = {}
    ts_n = ann_n = 0
    for start, end, speaker, text in sorted(segments, key=lambda s: (s[0], s[1])):
        a = int(round(start * 1000))
        if a < last_end.get(speaker, 0):
            a = last_end[speaker]  # ELAN forbids overlaps inside one tier
        b = max(int(round(end * 1000)), a + 1)
        last_end[speaker] = b
        if speaker not in tiers:
            tiers[speaker] = ET.SubElement(
                doc, "TIER",
                {"LINGUISTIC_TYPE_REF": "default-lt", "TIER_ID": speaker, "PARTICIPANT": speaker},
            )  # fmt: skip
        ids = []
        for ms in (a, b):
            ts_n += 1
            ids.append(f"ts{ts_n}")
            ET.SubElement(order, "TIME_SLOT", {"TIME_SLOT_ID": ids[-1], "TIME_VALUE": str(ms)})
        ann_n += 1
        wrap = ET.SubElement(tiers[speaker], "ANNOTATION")
        al = ET.SubElement(
            wrap, "ALIGNABLE_ANNOTATION",
            {"ANNOTATION_ID": f"a{ann_n}", "TIME_SLOT_REF1": ids[0], "TIME_SLOT_REF2": ids[1]},
        )  # fmt: skip
        ET.SubElement(al, "ANNOTATION_VALUE").text = _nfc(text)
    ET.SubElement(
        doc, "LINGUISTIC_TYPE",
        {"LINGUISTIC_TYPE_ID": "default-lt", "TIME_ALIGNABLE": "true", "GRAPHIC_REFERENCES": "false"},
    )  # fmt: skip
    ET.indent(doc)
    return ET.ElementTree(doc)


def draft_eaf(run_dir: Path, manifest, out_dir: Path) -> list[Path]:
    """One `<id>.eaf` per file with a hypothesis transcript; speakers from the hypothesis RTTM."""
    written = []
    out_dir.mkdir(parents=True, exist_ok=True)
    hyp = Path(run_dir) / "hyp"
    for entry in manifest.files:
        tpath = hyp / f"{entry.id}.transcript.json"
        if not tpath.is_file():
            continue
        doc = json.loads(tpath.read_text(encoding="utf-8"))
        rpath = hyp / f"{entry.id}.rttm"
        turns = read_rttm(rpath) if rpath.is_file() else []
        segs = []
        for s in doc["segments"]:
            if not s["text"].strip():
                continue
            fallback = s.get("speaker") or "spk1"
            segs.append(
                (
                    s["start"],
                    s["end"],
                    clean_label(assign_speaker(s["start"], s["end"], turns, fallback)),
                    s["text"],
                )
            )
        target = out_dir / f"{entry.id}.eaf"
        build_eaf(segs, entry.audio, out_dir).write(target, encoding="utf-8", xml_declaration=True)
        written.append(target)
    return written
