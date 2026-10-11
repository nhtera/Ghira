# SPDX-License-Identifier: Apache-2.0
# Long-form benchmark: joins each run of a subset (0.4 s pauses, like live_bench)
# into one 16 kHz file, transcribes it with each system, scores all runs together.
#   long_form.py <subset_dir> <work_dir> <system=cmd ...>
#   e.g. nemo="/path/ghi transcribe {wav} --pass final"
import json, os, subprocess, sys, wave
import yaml
from ghi_eval.metrics import asr_metric_for, token_errors

subset, work = sys.argv[1], sys.argv[2]
systems = [a.split("=", 1) for a in sys.argv[3:]]
man = yaml.safe_load(open(os.path.join(subset, "manifest.yaml")))
by_id = {f["id"]: f for f in man["files"]}
runs = json.load(open(os.path.join(subset, "runs.json")))
os.makedirs(work, exist_ok=True)
gap = b"\x00\x00" * 6400
totals = {name: [0, 0] for name, _ in systems}
lang = by_id[runs[0][0]].get("lang", "vi")
for r, ids in enumerate(runs):
    wav = os.path.join(work, f"run{r:02d}.wav")
    ref = " ".join(open(os.path.join(subset, by_id[i]["ref"])).read().strip() for i in ids)
    if not os.path.exists(wav):
        pcm = bytearray()
        for i in ids:
            pcm += subprocess.run(
                ["ffmpeg", "-v", "quiet", "-i", os.path.join(subset, by_id[i]["audio"]),
                 "-ac", "1", "-ar", "16000", "-f", "s16le", "-"],
                check=True, capture_output=True).stdout + gap
        with wave.open(wav, "wb") as w:
            w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000); w.writeframes(bytes(pcm))
    for name, cmd in systems:
        out = os.path.join(work, f"run{r:02d}.{name}.json")
        if not os.path.exists(out):
            res = subprocess.run(cmd.format(wav=wav), shell=True, capture_output=True, text=True)
            if res.returncode != 0:
                print(f"FAIL {name} run{r}: {res.stderr[-300:]}", file=sys.stderr)
                continue
            open(out, "w").write(res.stdout)
        hyp = " ".join(s["text"] for s in json.load(open(out))["segments"])
        e = token_errors(ref, hyp, lang)
        totals[name][0] += e.errors
        totals[name][1] += e.ref_tokens
for name, (err, n) in totals.items():
    print(f"{os.path.basename(subset)} long {name} {asr_metric_for(lang)}={err / max(n, 1):.4f} tokens={n}")
