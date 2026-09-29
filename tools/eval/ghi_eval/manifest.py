# SPDX-License-Identifier: Apache-2.0
"""Dataset manifest (formats.md §1): loading, slices and validation."""

from __future__ import annotations

import re
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

import yaml

from .audio import wav_duration
from .errors import HarnessError, ManifestError
from .rttm import read_rttm

LANGS = ("vi", "en", "mixed")
SETTINGS = ("room", "call", "other")
PLAYBACKS = ("headphones", "speakers", "na")
ID_RE = re.compile(r"^[A-Za-z0-9_-]+$")

# Target mix for a customer set (recording guide).
TARGET_HOURS = 10.0
TARGET_LANG_SHARE = {"vi": 0.4, "en": 0.3, "mixed": 0.3}
TARGET_BIG_MEETINGS = (3, 5)  # files with 6-8 speakers


@dataclass
class FileEntry:
    id: str
    audio: Path
    lang: str
    setting: str
    playback: str
    speakers: int
    tracks: dict[str, Path] = field(default_factory=dict)
    rttm: Path | None = None
    ref: Path | None = None
    notes_ref: Path | None = None
    duration_s: float | None = None
    persons: dict[str, str] = field(default_factory=dict)

    @property
    def bucket(self) -> str:
        return speaker_bucket(self.speakers)


@dataclass
class Manifest:
    root: Path
    name: str
    names: list[str]
    files: list[FileEntry]

    def duration(self, entry: FileEntry) -> float | None:
        """`duration_s` from the manifest, else read from the audio, else None."""
        if entry.duration_s is not None:
            return entry.duration_s
        if entry.audio.is_file():
            try:
                return wav_duration(entry.audio)
            except HarnessError:
                return None
        return None


def speaker_bucket(n: int) -> str:
    return "1-2" if n <= 2 else "3-5" if n <= 5 else "6+"


def load_manifest(dataset: Path) -> Manifest:
    """Parse manifest.yaml. Structural problems raise ManifestError (all listed at once)."""
    dataset = Path(dataset)
    path = dataset / "manifest.yaml"
    if not path.is_file():
        raise ManifestError(f"{path}: not found")
    try:
        raw = yaml.safe_load(path.read_text(encoding="utf-8"))
    except yaml.YAMLError as exc:
        raise ManifestError(f"manifest.yaml: invalid YAML ({exc})") from exc
    problems: list[str] = []
    if not isinstance(raw, dict):
        raise ManifestError("manifest.yaml: top level must be a mapping")
    if raw.get("version") != 1:
        problems.append("version must be 1")
    names = raw.get("names") or []
    if not isinstance(names, list) or not all(isinstance(n, str) for n in names):
        problems.append("names must be a list of strings")
        names = []
    files: list[FileEntry] = []
    seen: set[str] = set()
    raw_files = raw.get("files")
    if not isinstance(raw_files, list) or not raw_files:
        problems.append("files must be a non-empty list")
        raw_files = []
    for i, item in enumerate(raw_files):
        where = f"files[{i}]"
        if not isinstance(item, dict):
            problems.append(f"{where}: must be a mapping")
            continue
        fid = str(item.get("id", ""))
        where = f"files[{i}] ({fid or '?'})"
        if not ID_RE.match(fid):
            problems.append(f"{where}: id must match [A-Za-z0-9_-]+")
        if fid in seen:
            problems.append(f"{where}: duplicate id")
        seen.add(fid)
        for key, allowed in (("lang", LANGS), ("setting", SETTINGS), ("playback", PLAYBACKS)):
            if item.get(key) not in allowed:
                problems.append(f"{where}: {key} must be one of {', '.join(allowed)}")
        speakers = item.get("speakers")
        if not isinstance(speakers, int) or isinstance(speakers, bool) or speakers < 1:
            problems.append(f"{where}: speakers must be a positive integer")
            speakers = 1
        if not item.get("audio"):
            problems.append(f"{where}: audio is required")
        dur = item.get("duration_s")
        if dur is not None and (not isinstance(dur, (int, float)) or dur <= 0):
            problems.append(f"{where}: duration_s must be a positive number")
            dur = None
        persons = item.get("persons") or {}
        if not isinstance(persons, dict):
            problems.append(f"{where}: persons must be a mapping")
            persons = {}

        def rel(key: str, _item=item) -> Path | None:
            return dataset / str(_item[key]) if _item.get(key) else None

        files.append(
            FileEntry(
                id=fid,
                audio=rel("audio") or dataset / "audio" / f"{fid}.wav",
                lang=str(item.get("lang")),
                setting=str(item.get("setting")),
                playback=str(item.get("playback")),
                speakers=speakers,
                tracks={str(k): dataset / str(v) for k, v in (item.get("tracks") or {}).items()},
                rttm=rel("rttm"),
                ref=rel("ref"),
                notes_ref=rel("notes_ref"),
                duration_s=float(dur) if dur is not None else None,
                persons={str(k): str(v) for k, v in persons.items()},
            )
        )
    if problems:
        raise ManifestError("manifest.yaml: " + "; ".join(problems))
    return Manifest(dataset, str(raw.get("name") or dataset.name), [str(n) for n in names], files)


def validate(manifest: Manifest) -> tuple[list[str], list[str], dict]:
    """Check files and compare hours with the target mix.

    Returns (errors, warnings, stats). Messages may name file ids (they stay on screen,
    they are never written to a report).
    """
    errors: list[str] = []
    warnings: list[str] = []
    hours: dict[tuple[str, str, str], float] = defaultdict(float)
    by_lang: dict[str, float] = defaultdict(float)
    big = 0
    for f in manifest.files:
        dur = f.duration_s
        if f.audio.is_file():
            try:
                real = wav_duration(f.audio)
                if dur is None:
                    dur = real
                elif abs(real - dur) > 1.0:
                    warnings.append(f"{f.id}: duration_s {dur:.1f} differs from audio {real:.1f}")
            except HarnessError as exc:
                errors.append(f"{f.id}: {exc}")
        elif dur is None:
            errors.append(f"{f.id}: audio missing ({f.audio.name}) and no duration_s")
        else:
            warnings.append(f"{f.id}: audio missing ({f.audio.name}); using duration_s")
        for role, p in f.tracks.items():
            if not p.is_file():
                errors.append(f"{f.id}: track '{role}' missing ({p.name})")
        for role, p in (("rttm", f.rttm), ("ref", f.ref), ("notes_ref", f.notes_ref)):
            if p is not None and not p.is_file():
                errors.append(f"{f.id}: {role} missing ({p.name})")
        if f.rttm is not None and f.rttm.is_file():
            try:
                turns = read_rttm(f.rttm, f.id)
                if not turns:
                    errors.append(f"{f.id}: reference RTTM has no turns")
                found = len({t.speaker for t in turns})
                if turns and found != f.speakers:
                    warnings.append(
                        f"{f.id}: manifest says {f.speakers} speakers, RTTM has {found}"
                    )
            except HarnessError as exc:
                errors.append(str(exc))
        if f.setting == "call" and f.playback == "na":
            warnings.append(f"{f.id}: call recording should set playback headphones or speakers")
        h = (dur or 0.0) / 3600.0
        hours[(f.lang, f.setting, f.bucket)] += h
        by_lang[f.lang] += h
        big += f.speakers >= 6
    total = sum(by_lang.values())
    if total < TARGET_HOURS:
        warnings.append(f"total {total:.1f} h is below the {TARGET_HOURS:.0f} h target")
    for lang, share in TARGET_LANG_SHARE.items():
        got = by_lang.get(lang, 0.0) / total if total else 0.0
        if abs(got - share) > 0.10:
            warnings.append(f"lang {lang}: {got:.0%} of hours, target ~{share:.0%}")
    settings = {f.setting for f in manifest.files}
    for s in ("room", "call"):
        if s not in settings:
            warnings.append(f"no '{s}' recordings")
    playbacks = {f.playback for f in manifest.files if f.setting == "call"}
    if "call" in settings and not {"headphones", "speakers"} <= playbacks:
        warnings.append("calls: need both headphones and speakers playback")
    lo, hi = TARGET_BIG_MEETINGS
    if not lo <= big <= hi:
        warnings.append(f"{big} files with 6+ speakers, target {lo}-{hi}")
    stats = {"hours": total, "by_lang": dict(by_lang), "slices": dict(hours), "big_meetings": big}
    return errors, warnings, stats


def format_validation(
    manifest: Manifest, errors: list[str], warnings: list[str], stats: dict
) -> str:
    lines = [f"dataset {manifest.name}: {len(manifest.files)} files, {stats['hours']:.2f} h", ""]
    lines.append("lang    hours  share  target")
    for lang, share in TARGET_LANG_SHARE.items():
        h = stats["by_lang"].get(lang, 0.0)
        got = h / stats["hours"] if stats["hours"] else 0.0
        lines.append(f"{lang:<7} {h:5.2f}  {got:4.0%}   ~{share:.0%}")
    lines += ["", "lang    setting  speakers  hours"]
    for (lang, setting, bucket), h in sorted(stats["slices"].items()):
        lines.append(f"{lang:<7} {setting:<8} {bucket:<9} {h:5.2f}")
    lines.append("")
    lines += [f"error: {e}" for e in errors]
    lines += [f"warning: {w}" for w in warnings]
    lines.append("validate: FAILED" if errors else "validate: ok")
    return "\n".join(lines)
