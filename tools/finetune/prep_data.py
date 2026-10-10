# SPDX-License-Identifier: Apache-2.0
"""Training data for the Vietnamese Nemotron fine-tune (runs on the GPU box).

    prep_data.py fetch  DATA [--yodas-shards 20] [--libri-files 7]   # download, 16 kHz wav, raw.json
    prep_data.py label  DATA BASE.nemo                                # base-model transcript per clip
    prep_data.py select DATA                                          # filter -> train.json, val.json

Sources (licence-clean only; nothing non-commercial):
  vimedcss  tensorxt/ViMedCSS train, CC BY 4.0. Videos that also appear in its test split are
            dropped: the benchmark reads that test split, and 898 of its 995 videos are in train.
  vietmed   leduckhai/VietMed train, MIT (no recording is shared with the test split).
  yodas     espnet/yodas vi000 (uploader captions of CC-licensed videos), CC BY 3.0.
  libri     openslr/librispeech_asr clean train.100, CC BY 4.0: English replay. Its target is the
            base model's own transcript, so English output (case, punctuation, numbers as words)
            is held where it is.

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
}
TAG = re.compile(r"\s*<[a-z]{2,3}-[A-Z]{2}>\s*")


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
    for r in table.to_pylist():
        uid = str(r[id_col]).replace("/", "_")
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


def fetch(root: Path, yodas_shards: int, libri_files: int, workers: int) -> None:
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

    rows = []
    with ProcessPoolExecutor(workers) as ex:
        for part in ex.map(parquet_rows, jobs):
            rows += part
            print(f"{part[0]['source'] if part else '?'}: +{len(part)} clips", flush=True)

    for i in range(yodas_shards):
        name = f"{i:08d}"
        texts = {}
        for line in open(hf("espnet/yodas", f"data/vi000/text/{name}.txt", root), encoding="utf-8"):
            uid, _, text = line.rstrip("\n").partition(" ")
            texts[uid] = text
        durs = {}
        for line in open(hf("espnet/yodas", f"data/vi000/duration/{name}.txt", root), encoding="utf-8"):
            uid, d = line.split()[:2]
            durs[uid] = float(d)
        tar_path = hf("espnet/yodas", f"data/vi000/audio/{name}.tar.gz", root)
        out_dir = root / "wav" / "yodas"
        out_dir.mkdir(parents=True, exist_ok=True)
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
                rows.append({"audio_filepath": str(out), "duration": durs.get(uid, 0.0), "text": texts[uid], "lang": "vi-VN", "source": "yodas"})
        tar_path.unlink()  # 1 GB each; the wavs are what we keep
        print(f"yodas {name}: {len(texts)} captions", flush=True)

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
    step = 2000
    with open(out_path, "a", encoding="utf-8") as out:
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


def transfer_format(caption: str, hyp: str) -> str:
    """Lower-case unpunctuated caption + model transcript -> caption words in the model's style.

    Where the words agree the model's surface form (case, punctuation) is used; elsewhere the
    caption word stays as written. Only used for sets whose captions carry no formatting.
    """
    cap = caption.split()
    hyp_words = hyp.split()
    key = lambda w: "".join(ch for ch in w.lower() if ch.isalnum())  # noqa: E731
    sm = difflib.SequenceMatcher(a=[key(w) for w in cap], b=[key(w) for w in hyp_words], autojunk=False)
    out = []
    for op, a0, a1, b0, b1 in sm.get_opcodes():
        out += hyp_words[b0:b1] if op == "equal" else cap[a0:a1]
    s = " ".join(out)
    return s[:1].upper() + s[1:]


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
    "vimedcss": (0.6, "caption"),  # human transcripts; drop only misaligned clips
    "vietmed": (0.6, "format"),  # human, but lower-case and unpunctuated
    "yodas": (0.3, "caption"),  # uploader captions: often loose, keep the close ones
    "libri": (0.15, "model"),  # English replay: the model's own words
}


def select(root: Path, val_n: int, seed: int) -> None:
    hyps = {}
    for line in open(root / "hyp.json", encoding="utf-8"):
        r = json.loads(line)
        hyps[r["audio_filepath"]] = TAG.sub(" ", r["hyp"]).strip()
    kept, stats = [], {}
    for line in open(root / "raw.json", encoding="utf-8"):
        r = json.loads(line)
        src, lang = r["source"], r["lang"]
        hyp = hyps.get(r["audio_filepath"])
        s = stats.setdefault(src, {"in": 0, "kept": 0, "hours": 0.0})
        s["in"] += 1
        if hyp is None:
            continue
        limit, mode = RULES[src]
        cap = clean_caption(r["text"], lang)
        if cap is None or wer(cap, hyp, lang) > limit:
            continue
        if mode == "model":
            target = hyp
        elif mode == "format":
            target = transfer_format(cap, hyp)
        else:
            target = cap
        kept.append({"audio_filepath": r["audio_filepath"], "duration": r["duration"], "text": f"{target} <{lang}>", "lang": lang, "target_lang": lang, "source": src})
        s["kept"] += 1
        s["hours"] += r["duration"] / 3600
    random.Random(seed).shuffle(kept)
    val, train = kept[:val_n], kept[val_n:]
    for name, rows in (("train", train), ("val", val)):
        with open(root / f"{name}.json", "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r, ensure_ascii=False) + "\n")
    for src, s in stats.items():
        print(f"{src:9s} kept {s['kept']}/{s['in']} clips, {s['hours']:.1f} h")
    print(f"train {len(train)} clips, {sum(r['duration'] for r in train) / 3600:.1f} h; val {len(val)}")


def main() -> None:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("fetch")
    f.add_argument("data", type=Path)
    f.add_argument("--yodas-shards", type=int, default=20)
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
    a = ap.parse_args()
    if a.cmd == "fetch":
        fetch(a.data, a.yodas_shards, a.libri_files, a.workers)
    elif a.cmd == "label":
        label(a.data, a.nemo, a.batch)
    else:
        select(a.data, a.val, a.seed)


if __name__ == "__main__":
    main()
