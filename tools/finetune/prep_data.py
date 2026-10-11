# SPDX-License-Identifier: Apache-2.0
"""Training data for the Vietnamese Nemotron fine-tune (runs on the GPU box).

    prep_data.py fetch  DATA [--yodas-shards 20] [--yodas-en-shards 0] [--libri-files 7] [--fosd]
    prep_data.py label  DATA BASE.nemo                                # base-model transcript per clip
    prep_data.py select DATA [--auto-frac 0.5] [--join-frac 0.5]      # filter, join -> train.json, val.json
    prep_data.py probe  DATA BASE.nemo CAND.nemo [...]                # dropped-speech check, base vs candidate

Sources (licence-clean only; nothing non-commercial):
  vimedcss  tensorxt/ViMedCSS train, CC BY 4.0. Videos that also appear in its test split are
            dropped: the benchmark reads that test split, and 898 of its 995 videos are in train.
  vietmed   leduckhai/VietMed train, MIT (no recording is shared with the test split).
  yodas     espnet/yodas vi000 (uploader captions of CC-licensed videos), CC BY 3.0.
  fosd      doof-ferb/fpt_fosd, a copy of FPT Open Speech (Mendeley k9sxg2twv4), CC BY 4.0; ~30 h read.
  libri     openslr/librispeech_asr clean train.100, CC BY 4.0: English replay. Its target is the
            base model's own transcript, so English output (case, punctuation, numbers as words)
            is held where it is.
  yodas_en  espnet/yodas en000 shards 1.. (shard 0 is the probe's), CC BY 3.0: varied English
            replay, also the model's own transcript.

Targets follow the base model's output format: cased, punctuated, numbers as words, and the
language tag token last ("... chuyển. <vi-VN>"). FLEURS is deliberately not used, so it stays
an untouched control in the benchmark.
"""

from __future__ import annotations

import argparse
import csv
import difflib
import io
import json
import os
import random
import re
import sys
import tarfile
import unicodedata
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))  # ghi_eval is copied next to this file
from ghi_eval import numwords, textnorm  # noqa: E402

SR = 16000
REV = {
    "tensorxt/ViMedCSS": "main",
    "leduckhai/VietMed": "main",
    "espnet/yodas": "main",
    "openslr/librispeech_asr": "main",
    "doof-ferb/fpt_fosd": "e6ff950456eeaef7a257e79bf7f4597bb39c264c",
}
TAG = re.compile(r"\s*<[a-z]{2,3}-[A-Z]{2}>\s*")
SILENCE = 0.4  # s of zeros after each part of a joined example (the eval kit's long-form gap)


# ---------------------------------------------------------------- fetch


def hf(repo: str, path: str, root: Path) -> Path:
    from huggingface_hub import hf_hub_download

    return Path(hf_hub_download(repo, path, repo_type="dataset", revision=REV[repo], local_dir=root / "hf"))


def write_wav(path: Path, audio, sr: int) -> float:
    import numpy as np
    import soundfile as sf

    if audio.ndim > 1:
        audio = audio.mean(axis=1)
    if sr != SR:
        import soxr

        audio = soxr.resample(audio, sr, SR)
    path.parent.mkdir(parents=True, exist_ok=True)
    sf.write(path, np.clip(audio, -1, 1), SR, subtype="PCM_16")
    return len(audio) / SR


def decode(blob: bytes):
    import soundfile as sf

    audio, sr = sf.read(io.BytesIO(blob), dtype="float32")
    return audio, sr


def parquet_rows(args):
    """Decodes one parquet file to wavs; returns raw manifest rows."""
    import pyarrow.parquet as pq

    file, src, lang, text_col, id_col, keep, root = args
    rows = []
    table = pq.read_table(file)
    for i, r in enumerate(table.to_pylist()):
        uid = str(r[id_col]).replace("/", "_") if id_col else f"{Path(file).stem}-{i}"
        if keep is not None and not keep(r):
            continue
        out = root / "wav" / src / f"{uid}.wav"
        try:
            if out.exists():
                import soundfile as sf

                dur = sf.info(out).duration
            else:
                audio, sr = decode(r["audio"]["bytes"])
                dur = write_wav(out, audio, sr)
        except Exception as e:  # a broken clip is skipped, not fatal
            print(f"skip {src} {uid}: {e}", file=sys.stderr)
            continue
        rows.append({"audio_filepath": str(out), "duration": round(dur, 3), "text": r[text_col], "lang": lang, "source": src})
    return rows


class NotInTest:
    def __init__(self, videos: set[str]):
        self.videos = videos

    def __call__(self, row) -> bool:
        return row.get("original_video_link") not in self.videos


def yodas_rows(args):
    """One YODAS shard (sub = vi000 / en000) to wavs; returns raw manifest rows."""
    sub, i, src, lang, root = args
    name = f"{i:08d}"
    texts = {}
    for line in open(hf("espnet/yodas", f"data/{sub}/text/{name}.txt", root), encoding="utf-8"):
        uid, _, text = line.rstrip("\n").partition(" ")
        texts[uid] = text
    durs = {}
    for line in open(hf("espnet/yodas", f"data/{sub}/duration/{name}.txt", root), encoding="utf-8"):
        uid, d = line.split()[:2]
        durs[uid] = float(d)
    tar_path = hf("espnet/yodas", f"data/{sub}/audio/{name}.tar.gz", root)
    out_dir = root / "wav" / src
    out_dir.mkdir(parents=True, exist_ok=True)
    rows = []
    with tarfile.open(tar_path) as tar:
        for m in tar:
            if not m.isfile() or not m.name.endswith(".wav"):
                continue
            uid = Path(m.name).stem
            if uid not in texts:
                continue
            out = out_dir / f"{uid}.wav"
            if not out.exists():
                out.write_bytes(tar.extractfile(m).read())
            rows.append({"audio_filepath": str(out), "duration": durs.get(uid, 0.0), "text": texts[uid], "lang": lang, "source": src})
    tar_path.unlink()  # 1 GB each; the wavs are what we keep
    return rows


def fetch(root: Path, yodas_shards: int, yodas_en_shards: int, libri_files: int, fosd: bool, workers: int) -> None:
    from huggingface_hub import list_repo_files

    root.mkdir(parents=True, exist_ok=True)
    jobs = []

    test_csv = hf("tensorxt/ViMedCSS", "ViMedCSS-Metadata/test_set.csv", root)
    test_videos = {r["original_video_link"] for r in csv.DictReader(open(test_csv, encoding="utf-8"))}
    files = sorted(f for f in list_repo_files("tensorxt/ViMedCSS", repo_type="dataset") if f.startswith("data/train-"))
    for f in files:
        jobs.append((hf("tensorxt/ViMedCSS", f, root), "vimedcss", "vi-VN", "segment_text", "segment_id", NotInTest(test_videos), root))

    jobs.append((hf("leduckhai/VietMed", "data/train-00000-of-00001.parquet", root), "vietmed", "vi-VN", "text", "utterance_id", None, root))

    libri = sorted(f for f in list_repo_files("openslr/librispeech_asr", repo_type="dataset") if f.startswith("clean/train.100/"))
    for f in libri[:libri_files]:
        jobs.append((hf("openslr/librispeech_asr", f, root), "libri", "en-US", "text", "id", None, root))

    if fosd:
        files = sorted(f for f in list_repo_files("doof-ferb/fpt_fosd", repo_type="dataset", revision=REV["doof-ferb/fpt_fosd"]) if f.endswith(".parquet"))
        for f in files:
            jobs.append((hf("doof-ferb/fpt_fosd", f, root), "fosd", "vi-VN", "transcription", None, None, root))

    rows = []
    with ProcessPoolExecutor(workers) as ex:
        for part in ex.map(parquet_rows, jobs):
            rows += part
            print(f"{part[0]['source'] if part else '?'}: +{len(part)} clips", flush=True)
        # Shards download and unpack in parallel; en000 shard 0 is left for the probe.
        shards = [("vi000", i, "yodas", "vi-VN", root) for i in range(yodas_shards)]
        shards += [("en000", i, "yodas_en", "en-US", root) for i in range(1, yodas_en_shards + 1)]
        for (sub, i, *_), part in zip(shards, ex.map(yodas_rows, shards)):
            rows += part
            print(f"yodas {sub} {i:08d}: +{len(part)} clips", flush=True)

    with open(root / "raw.json", "w", encoding="utf-8") as f:
        for r in rows:
            if 0.3 <= r["duration"] <= 20:
                f.write(json.dumps(r, ensure_ascii=False) + "\n")
    hours = {}
    for r in rows:
        hours[r["source"]] = hours.get(r["source"], 0) + r["duration"] / 3600
    print("hours:", {k: round(v, 1) for k, v in hours.items()})


# ---------------------------------------------------------------- label


def label(root: Path, nemo: str, batch: int) -> None:
    """Base-model transcript for every clip (target_lang = the clip's language)."""
    import torch
    import nemo.collections.asr as nemo_asr

    m = nemo_asr.models.ASRModel.restore_from(nemo, map_location="cuda" if torch.cuda.is_available() else "cpu")
    m.eval()
    rows = [json.loads(line) for line in open(root / "raw.json", encoding="utf-8")]
    done = {}
    out_path = root / "hyp.json"
    if out_path.exists():
        for line in open(out_path, encoding="utf-8"):
            r = json.loads(line)
            done[r["audio_filepath"]] = r["hyp"]
    todo = [r for r in rows if r["audio_filepath"] not in done]
    print(f"label: {len(todo)} of {len(rows)} clips to go", flush=True)
    import soundfile as sf

    # Some YODAS segments past the end of a video list 20 s but hold no samples; they crash
    # the preprocessor. An empty hyp makes select drop them.
    long_enough = {r["audio_filepath"]: sf.info(r["audio_filepath"]).frames >= 0.3 * SR for r in todo}
    empty = [r for r in todo if not long_enough[r["audio_filepath"]]]
    todo = [r for r in todo if long_enough[r["audio_filepath"]]]
    step = 2000
    with open(out_path, "a", encoding="utf-8") as out:
        for r in empty:
            out.write(json.dumps({"audio_filepath": r["audio_filepath"], "hyp": ""}) + "\n")
        print(f"label: {len(empty)} clips with under 0.3 s of audio skipped", flush=True)
        for i in range(0, len(todo), step):
            chunk = todo[i : i + step]
            man = root / "label_chunk.json"
            with open(man, "w", encoding="utf-8") as f:
                for r in chunk:
                    f.write(json.dumps({"audio_filepath": r["audio_filepath"], "duration": r["duration"], "text": "", "lang": r["lang"], "target_lang": r["lang"]}) + "\n")
            with torch.autocast("cuda", dtype=torch.bfloat16, enabled=torch.cuda.is_available()), torch.inference_mode():
                hyps = m.transcribe(str(man), batch_size=batch, verbose=False, num_workers=4)
            for r, h in zip(chunk, hyps):
                text = h.text if hasattr(h, "text") else str(h)
                out.write(json.dumps({"audio_filepath": r["audio_filepath"], "hyp": text}, ensure_ascii=False) + "\n")
            out.flush()
            print(f"label: {min(i + step, len(todo))}/{len(todo)}", flush=True)


# ---------------------------------------------------------------- select


def clean_caption(text: str, lang: str) -> str | None:
    """Caption -> model-style target text, or None when the caption is not usable."""
    text = unicodedata.normalize("NFC", text).strip()
    if not text or "[" in text or "(" in text or "♪" in text:
        return None
    for ch in text:
        cat = unicodedata.category(ch)
        if cat in ("So", "Sk", "Co", "Cn") or (ch.isalpha() and not ("LATIN" in unicodedata.name(ch, ""))):
            return None  # emoji, symbols, non-Latin script
    text = numwords.expand_digits(text, "en" if lang.startswith("en") else "vi")
    text = re.sub(r"\s+", " ", text).strip()
    text = re.sub(r"\s+([,.?!;:])", r"\1", text)
    return text or None


def merge(caption: str, hyp: str) -> str:
    """Caption + model transcript (with its language tags) -> target that matches the audio.

    Subtitle cuts are loose (only ~10% of YODAS clips agree with the model on both edge words):
    a caption may hold words cut off in the audio, or miss words the audio has. Before the first
    and after the last agreeing word the model's words are used; between them the caption's
    (the human correction) where they differ, and the model's surface form (case, punctuation)
    where they agree, so unpunctuated captions don't teach unpunctuated output.

    The model's language tags stay where it put them (at sentence ends, so a clip cut
    mid-sentence ends without one). The streaming runtime keeps decoding after a tag, so a tag
    must not be learned as "nothing follows": runs 2-3 put one tag at the end of every clip and
    then dropped the first seconds of the next speaker in a continuous stream.
    """
    cap = caption.split()
    tokens = hyp.split()
    words, tags_after = [], {}  # tags_after[j]: tags right after hyp word j (-1: before any)
    for t in tokens:
        if TAG.fullmatch(t):
            tags_after.setdefault(len(words) - 1, []).append(t)
        else:
            words.append(t)
    key = lambda w: "".join(ch for ch in w.lower() if ch.isalnum())  # noqa: E731
    ops = difflib.SequenceMatcher(a=[key(w) for w in cap], b=[key(w) for w in words], autojunk=False).get_opcodes()
    eq = [i for i, op in enumerate(ops) if op[0] == "equal"]
    out = []
    for i, (op, a0, a1, b0, b1) in enumerate(ops):
        if op == "equal" or not eq or i < eq[0] or i > eq[-1]:
            for j in range(b0, b1):
                out.append(words[j])
                out += tags_after.get(j, [])
        else:  # a differing span between agreeing words: the caption's words, then the span's tags
            out += cap[a0:a1]
            for j in range(b0, b1):
                out += tags_after.get(j, [])
    text = " ".join(out)
    return text[:1].upper() + text[1:]


def wer(ref: str, hyp: str, lang: str) -> float:
    r = textnorm.normalize(ref, "en" if lang.startswith("en") else "vi")
    h = textnorm.normalize(hyp, "en" if lang.startswith("en") else "vi")
    if not r:
        return 1.0
    sm = difflib.SequenceMatcher(a=r, b=h, autojunk=False)
    matched = sum(b.size for b in sm.get_matching_blocks())
    return (len(r) + len(h) - 2 * matched) / (2 * len(r))  # symmetric edit proxy, cheap and enough to filter


# Per source: maximum caption-vs-model disagreement kept, and where the target text comes from.
RULES = {
    "vimedcss": (0.6, "merge"),  # human transcripts; drop only misaligned clips
    "vietmed": (0.6, "merge"),  # human, but lower-case and unpunctuated
    "fosd": (0.4, "merge"),  # human, read speech, lower-case and unpunctuated
    "yodas": (0.3, "merge"),  # uploader captions: often loose, keep the close ones
    "libri": (0.15, "model"),  # English replay: the model's own words
    "yodas_en": (0.15, "model"),
}


def join_audio(args) -> None:
    """Writes one joined example: the parts with a pause after each (not after the last)."""
    import numpy as np
    import soundfile as sf

    out, parts, gaps = args
    pcm = []
    for path, gap in zip(parts, gaps):
        audio, sr = sf.read(path, dtype="float32")
        pcm += [audio, np.zeros(int(gap * sr), dtype="float32")]
    sf.write(out, np.concatenate(pcm[:-1]), SR, subtype="PCM_16")


def select(root: Path, val_n: int, seed: int, auto_frac: float, join_frac: float, repeat: dict[str, int], workers: int) -> None:
    """Filters the clips, builds targets, and writes train.json / val.json.

    auto_frac of the train examples get the "auto" prompt (NeMo trains only on each example's own
    prompt, so the auto prompt a meeting without a language uses would drift otherwise).
    join_frac of the train clips are joined 2-3 at a time, from different sources and videos,
    with 0.2-1.0 s pauses: a continuous stream carries the previous speaker and the last
    language tag into the next utterance, which single clips never teach. `repeat` repeats
    the small in-domain sets (the medical ones are the gate's hard cases).
    """
    hyps = {}
    for line in open(root / "hyp.json", encoding="utf-8"):
        r = json.loads(line)
        hyps[r["audio_filepath"]] = re.sub(r"\s+", " ", TAG.sub(lambda m: f" {m.group().strip()} ", r["hyp"])).strip()
    kept, stats = [], {}
    for line in open(root / "raw.json", encoding="utf-8"):
        r = json.loads(line)
        src, lang = r["source"], r["lang"]
        hyp = hyps.get(r["audio_filepath"])
        s = stats.setdefault(src, {"in": 0, "kept": 0, "hours": 0.0})
        s["in"] += 1
        if not hyp:
            continue
        limit, mode = RULES[src]
        cap = clean_caption(r["text"], lang)
        if cap is None or wer(cap, TAG.sub(" ", hyp), lang) > limit:
            continue
        target = hyp if mode == "model" else merge(cap, hyp)
        kept.append({"audio_filepath": r["audio_filepath"], "duration": r["duration"], "text": target, "lang": lang, "target_lang": lang, "source": src})
        s["kept"] += 1
        s["hours"] += r["duration"] / 3600
    rng = random.Random(seed)
    rng.shuffle(kept)
    val, train = kept[:val_n], kept[val_n:]
    train += [r for r in train for _ in range(repeat.get(r["source"], 1) - 1)]
    rng.shuffle(train)

    # Join: walk a shuffled pool, taking 2-3 clips of other videos (YODAS ids start with the
    # video id) up to 20 s in total.
    n_join = int(len(train) * join_frac)
    pool, single, jobs, joined = train[:n_join], train[n_join:], [], []
    video = lambda r: Path(r["audio_filepath"]).stem.split("-")[0]  # noqa: E731
    out_dir = root / "wav" / "joined"
    out_dir.mkdir(parents=True, exist_ok=True)
    i = 0
    while i < len(pool):
        parts, want = [pool[i]], rng.choice((2, 3))
        i += 1
        while len(parts) < want and i < len(pool):
            nxt = pool[i]
            if video(nxt) in {video(p) for p in parts} or sum(p["duration"] for p in parts) + nxt["duration"] + 1.0 > 20:
                break
            parts.append(nxt)
            i += 1
        if len(parts) == 1:
            single.append(parts[0])
            continue
        gaps = [round(rng.uniform(0.2, 1.0), 2) for _ in parts]
        out = out_dir / f"j{len(jobs):07d}.wav"
        langs = {p["lang"] for p in parts}
        lang = langs.pop() if len(langs) == 1 else "auto"
        jobs.append((str(out), [p["audio_filepath"] for p in parts], gaps))
        joined.append({"audio_filepath": str(out), "duration": round(sum(p["duration"] for p in parts) + sum(gaps[:-1]), 3), "text": " ".join(p["text"] for p in parts), "lang": lang, "target_lang": lang, "source": "joined"})
    with ProcessPoolExecutor(workers) as ex:
        list(ex.map(join_audio, jobs, chunksize=256))
    train = single + joined
    rng.shuffle(train)
    for r in train:
        if rng.random() < auto_frac:
            r["lang"] = r["target_lang"] = "auto"
    for name, rows in (("train", train), ("val", val)):
        with open(root / f"{name}.json", "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r, ensure_ascii=False) + "\n")
    for src, s in stats.items():
        print(f"{src:9s} kept {s['kept']}/{s['in']} clips, {s['hours']:.1f} h")
    print(f"joined {len(joined)} examples from {sum(len(j[1]) for j in jobs)} clips")
    print(f"train {len(train)} examples, {sum(r['duration'] for r in train) / 3600:.1f} h; val {len(val)}")


# ---------------------------------------------------------------- probe


def probe_set(root: Path, n: int) -> Path:
    """First n clips (2-20 s) of YODAS en000 shard 0: varied real English speech, never trained on."""
    out_man = root / "probe_en.json"
    if out_man.exists():
        return out_man
    texts = {}
    for line in open(hf("espnet/yodas", "data/en000/text/00000000.txt", root), encoding="utf-8"):
        uid, _, text = line.rstrip("\n").partition(" ")
        texts[uid] = text
    durs = {}
    for line in open(hf("espnet/yodas", "data/en000/duration/00000000.txt", root), encoding="utf-8"):
        uid, d = line.split()[:2]
        durs[uid] = float(d)
    tar_path = hf("espnet/yodas", "data/en000/audio/00000000.tar.gz", root)
    out_dir = root / "wav" / "probe_en"
    out_dir.mkdir(parents=True, exist_ok=True)
    rows = []
    with tarfile.open(tar_path) as tar:
        for m in tar:
            uid = Path(m.name).stem
            if not m.isfile() or not m.name.endswith(".wav") or uid not in texts or not 2 <= durs.get(uid, 0) <= 20:
                continue
            out = out_dir / f"{uid}.wav"
            out.write_bytes(tar.extractfile(m).read())
            rows.append({"audio_filepath": str(out), "duration": durs[uid], "text": texts[uid], "lang": "en-US", "target_lang": "en-US"})
            if len(rows) >= n:
                break
    tar_path.unlink()
    with open(out_man, "w", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    return out_man


def transcribe_all(nemo: str, manifest: Path, batch: int) -> list[str]:
    import torch
    import nemo.collections.asr as nemo_asr

    m = nemo_asr.models.ASRModel.restore_from(nemo, map_location="cuda" if torch.cuda.is_available() else "cpu")
    m.eval()
    with torch.autocast("cuda", dtype=torch.bfloat16, enabled=torch.cuda.is_available()), torch.inference_mode():
        hyps = m.transcribe(str(manifest), batch_size=batch, verbose=False, num_workers=4)
    del m
    torch.cuda.empty_cache()
    return [TAG.sub(" ", h.text if hasattr(h, "text") else str(h)).strip() for h in hyps]


def probe(root: Path, base: str, cands: list[str], n: int, batch: int) -> None:
    """Dropped-speech check: the pilot's failure was empty or half outputs on English it should read.

    Per set and model: clips with no output, clips where the output has under half the base
    model's words ("dropped"), clips missing the base's first word in their first three
    ("head"), and the caption error rate. Each set runs with its language prompt and with
    "auto", and as pairs joined by a pause ("second": the second clip's opening words that the
    base reads but the candidate loses). FLEURS stays out of this on purpose.
    """
    key = lambda w: "".join(ch for ch in w.lower() if ch.isalnum())  # noqa: E731
    sets = {"val": root / "val.json", "yodas-en": probe_set(root, n)}
    for name in list(sets):  # the same clips with the auto prompt (a meeting with no language set)
        auto = root / f"probe_{name}_auto.json"
        with open(auto, "w", encoding="utf-8") as f:
            for line in open(sets[name], encoding="utf-8"):
                r = json.loads(line)  # lang is the prompt NeMo reads; text_lang keeps the scoring language
                f.write(json.dumps({**r, "lang": "auto", "target_lang": "auto", "text_lang": r["lang"]}, ensure_ascii=False) + "\n")
        sets[f"{name}-auto"] = auto
    for name in ("val", "yodas-en"):  # pairs joined with a pause: the second speaker follows a tag
        rows = [json.loads(line) for line in open(sets[name], encoding="utf-8")]
        half = len(rows) // 2
        out_dir = root / "wav" / "probe_joined"
        out_dir.mkdir(parents=True, exist_ok=True)
        joined = root / f"probe_{name}_joined.json"
        with open(joined, "w", encoding="utf-8") as f:
            for i, (a, b) in enumerate(zip(rows[:half], rows[half:])):
                wav = out_dir / f"{name}-{i:04d}.wav"
                join_audio((str(wav), [a["audio_filepath"], b["audio_filepath"]], [0.5, 0.5]))
                second = [key(w) for w in TAG.sub(" ", b["text"]).split()][:2]
                f.write(json.dumps({"audio_filepath": str(wav), "duration": a["duration"] + 0.5 + b["duration"], "text": f"{a['text']} {b['text']}", "lang": a["lang"], "target_lang": a["lang"], "second": second}, ensure_ascii=False) + "\n")
        sets[f"{name}-joined"] = joined
    for name, man in sets.items():
        rows = [json.loads(line) for line in open(man, encoding="utf-8")]
        refs = [TAG.sub(" ", r["text"]).strip() for r in rows]
        langs = [r.get("text_lang", r["lang"]) for r in rows]
        base_h = transcribe_all(base, man, batch)
        for label_, nemo in [("base", base)] + [(Path(c).stem, c) for c in cands]:
            hyp = base_h if nemo == base else transcribe_all(nemo, man, batch)
            empty = sum(1 for h, b in zip(hyp, base_h) if not h.split() and len(b.split()) >= 3)
            dropped = sum(1 for h, b in zip(hyp, base_h) if len(b.split()) >= 4 and len(h.split()) < 0.5 * len(b.split()))
            head = sum(1 for h, b in zip(hyp, base_h) if len(b.split()) >= 4 and key(b.split()[0]) not in [key(w) for w in h.split()[:3]])
            # joined sets: the second clip's first two words, read by the base but lost here
            bigram = lambda t: {tuple(key(w) for w in t.split()[i : i + 2]) for i in range(len(t.split()))}  # noqa: E731
            second = sum(1 for r, h, b in zip(rows, hyp, base_h) if len(r.get("second", [])) == 2 and tuple(r["second"]) in bigram(b) and tuple(r["second"]) not in bigram(h))
            err = sum(wer(r, h, lg) for r, h, lg in zip(refs, hyp, langs)) / len(rows)
            print(f"PROBE {name:14s} {label_:10s} clips={len(rows)} empty={empty} dropped={dropped} head={head} second={second} caption_err={err:.3f}", flush=True)


def main() -> None:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("fetch")
    f.add_argument("data", type=Path)
    f.add_argument("--yodas-shards", type=int, default=20)
    f.add_argument("--yodas-en-shards", type=int, default=0)
    f.add_argument("--fosd", action="store_true")
    f.add_argument("--libri-files", type=int, default=7)
    f.add_argument("--workers", type=int, default=os.cpu_count() or 4)
    lb = sub.add_parser("label")
    lb.add_argument("data", type=Path)
    lb.add_argument("nemo")
    lb.add_argument("--batch", type=int, default=64)
    s = sub.add_parser("select")
    s.add_argument("data", type=Path)
    s.add_argument("--val", type=int, default=400)
    s.add_argument("--seed", type=int, default=7)
    s.add_argument("--auto-frac", type=float, default=0.5)
    s.add_argument("--join-frac", type=float, default=0.5)
    s.add_argument("--repeat", default="vimedcss=3,vietmed=3", help="source=times,...")
    s.add_argument("--workers", type=int, default=os.cpu_count() or 4)
    p = sub.add_parser("probe")
    p.add_argument("data", type=Path)
    p.add_argument("nemo")
    p.add_argument("cands", nargs="+")
    p.add_argument("--en-clips", type=int, default=500)
    p.add_argument("--batch", type=int, default=64)
    a = ap.parse_args()
    if a.cmd == "fetch":
        fetch(a.data, a.yodas_shards, a.yodas_en_shards, a.libri_files, a.fosd, a.workers)
    elif a.cmd == "label":
        label(a.data, a.nemo, a.batch)
    elif a.cmd == "probe":
        probe(a.data, a.nemo, a.cands, a.en_clips, a.batch)
    else:
        repeat = {k: int(v) for k, v in (kv.split("=") for kv in a.repeat.split(",") if kv)}
        select(a.data, a.val, a.seed, a.auto_frac, a.join_frac, repeat, a.workers)


if __name__ == "__main__":
    main()
