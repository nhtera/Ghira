#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs ON the rented GPU box (pytorch/pytorch CUDA image), from the folder vast.sh pushed:
#   prep_data.py make_config.py vimedcss_bench_videos.txt ghi_eval/ box.sh
# Every stage leaves a marker in $W/state, so re-running after a stop resumes where it was.
#   STEPS=6500 LR=3e-5 WARMUP=300 VAL_EVERY=500 BATCH=160 ACCUM=2 YODAS=80 YODAS_EN=25 LIBRI=7 AUTO=0.5 JOIN=0.5 NARROW=0.15 bash box.sh
# Pilot 1 (lr 1e-4, 5000 steps) dropped whole English utterances; pilot 2 (lr 3e-5, 2000 steps on
# 128 h) plateaued halfway and dropped the first words of live lines under the auto prompt. These
# defaults: all of YODAS vi000 + FOSD, more varied English, half the clips on the auto prompt.
# Run 3 still lost a new speaker's first seconds in a continuous stream (one tag ending every
# target taught "nothing after a tag"): targets now keep the model's own tags and half the clips
# are joined 2-3 at a time.
# Run 5: medical data 9 -> 24 h ViMedCSS (only the benchmark's own videos held out) and 5 -> 10 h
# VietMed (+ dev), val held out per source by video/speaker, 15% telephone-band clips, ~1 epoch.
# Its val_wer was lowest at step 250 (a third of val is the base model's own words), so the
# top-3-by-val export is an early model: SNAPS keeps the weights at later steps too, and late.nemo
# is their mean with the final step.
# Run 7: VIETMED_UN shards of VietMed's unlabeled medical audio (~8.6 h each), labelled by TEACHER
# (vast.sh push puts it at $W/teacher.nemo; else the base model) and kept where Whisper agrees.
set -euo pipefail
W=${W:-/workspace}
HERE=$(cd "$(dirname "$0")" && pwd)
NEMO_SHA=${NEMO_SHA:-e4a34fd25f895ecd7f64f0e56bc288165e078da9}   # NeMo main, 2026-10-10 (dry run on this)
BASE_REV=ea30d66debe3740a08b573244286791d423d6b3e                  # nvidia/nemotron-3.5-asr-streaming-0.6b
STEPS=${STEPS:-6500} LR=${LR:-3e-5} WARMUP=${WARMUP:-300} VAL_EVERY=${VAL_EVERY:-500}
BATCH=${BATCH:-160} ACCUM=${ACCUM:-2} YODAS=${YODAS:-80} YODAS_EN=${YODAS_EN:-25} LIBRI=${LIBRI:-7}
AUTO=${AUTO:-0.5} JOIN=${JOIN:-0.5} NARROW=${NARROW:-0.15} AVG=${AVG:-3} SNAPS=${SNAPS:-"5000 6000"}
VIETMED_UN=${VIETMED_UN:-0} WBATCH=${WBATCH:-128}   # Whisper batch: 32 left the A100 at 36% (823 clips/min)
SNAPS=${SNAPS//,/ }   # vast.sh run passes no spaces: SNAPS=6000,7000
mkdir -p "$W/state"
cd "$W"
done_() { [ -f "$W/state/$1" ]; }
mark() { touch "$W/state/$1"; echo "== $1 done $(date -u +%H:%M:%S)"; }

if ! done_ env; then
  apt-get update -qq && apt-get install -y -qq git libsndfile1 >/dev/null
  pip install -q Cython packaging
  # nemo's cu12 extra pins torch==2.11.0+cu129, which only PyTorch's own index serves; torchaudio must match it
  pip install -q --extra-index-url https://download.pytorch.org/whl/cu129 \
    "nemo_toolkit[asr,cu12] @ git+https://github.com/NVIDIA-NeMo/NeMo.git@$NEMO_SHA" "torchaudio==2.11.0" soxr soundfile pyarrow huggingface_hub
  pip uninstall -y -q torchvision   # the image's build is for its old torch and nothing here needs it
  pip install -q "numba-cuda[cu12]" "nvidia-nvjitlink-cu12==12.9.86" "Brotli>=1.2"
  # RNNT loss needs numba-cuda; its NVVM (cuda-nvcc 12.9) must match nvJitLink, or kernels fail to link. The image's Brotli 1.1 breaks huggingface_hub downloads of compressed files
  rm -rf nemo-src && git init -q nemo-src && git -C nemo-src remote add origin https://github.com/NVIDIA-NeMo/NeMo.git
  git -C nemo-src fetch -q --depth 1 origin "$NEMO_SHA" && git -C nemo-src checkout -q FETCH_HEAD
  python -c "import torch, nemo; print('torch', torch.__version__, 'cuda', torch.cuda.is_available(), torch.cuda.get_device_name(0), 'nemo', nemo.__version__)"
  mark env
fi

if ! done_ base; then
  python - <<EOF
from huggingface_hub import hf_hub_download
hf_hub_download("nvidia/nemotron-3.5-asr-streaming-0.6b", "nemotron-3.5-asr-streaming-0.6b.nemo", revision="$BASE_REV", local_dir="$W/base")
EOF
  mark base
fi
BASE=$W/base/nemotron-3.5-asr-streaming-0.6b.nemo
TEACHER=${TEACHER:-$([ -f "$W/teacher.nemo" ] && echo "$W/teacher.nemo" || echo "$BASE")}

# (if-blocks, not `a || b`: set -e must stop the run when a stage fails)
if ! done_ fetch; then python "$HERE/prep_data.py" fetch "$W/data" --yodas-shards "$YODAS" --yodas-en-shards "$YODAS_EN" --libri-files "$LIBRI" --fosd --vietmed-un-shards "$VIETMED_UN"; mark fetch; fi
if ! done_ label; then python "$HERE/prep_data.py" label "$W/data" "$BASE" --batch 64 --exclude vietmed_un; mark label; fi
if [ "$VIETMED_UN" -gt 0 ]; then
  if ! done_ teacher; then echo "teacher $TEACHER"; python "$HERE/prep_data.py" label "$W/data" "$TEACHER" --batch 64 --sources vietmed_un --out hyp_teacher.json; mark teacher; fi
  if ! done_ whisper; then python "$HERE/prep_data.py" whisper "$W/data" --sources vietmed_un --batch "$WBATCH"; mark whisper; fi
fi
if ! done_ select; then python "$HERE/prep_data.py" select "$W/data" --auto-frac "$AUTO" --join-frac "$JOIN" --narrow-frac "$NARROW"; mark select; fi

python "$HERE/make_config.py" "$BASE" "$W/ft.yaml" --train "$W/data/train.json" --val "$W/data/val.json" \
  --exp-dir "$W/exp" --steps "$STEPS" --batch-duration "$BATCH" --accumulate "$ACCUM" --lr "$LR" --warmup "$WARMUP" \
  --val-every "$VAL_EVERY" --workers 8
export PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True
CKDIR=$W/exp/nemotron-vi-ft/checkpoints
if ! done_ train; then
  # Copies the weights of the run's last checkpoint when it reaches a SNAPS step. One "epoch" is
  # VAL_EVERY batches = VAL_EVERY/ACCUM optimizer steps.
  mkdir -p "$W/snap"
  python - "$CKDIR" "$W" $((VAL_EVERY / ACCUM)) $SNAPS > "$W/snap.log" 2>&1 <<'EOF' &
import glob, os, re, sys, time, torch
ck, w, per, want = sys.argv[1], sys.argv[2], int(sys.argv[3]), {int(s) for s in sys.argv[4:]}
while want and not os.path.exists(f"{w}/state/train"):
    for p in glob.glob(ck + "/*-last.ckpt"):
        step = (int(re.search(r"epoch=(\d+)", p)[1]) + 1) * per
        out = f"{w}/snap/step{step}.pt"
        if step in want and not os.path.exists(out):
            m = os.path.getmtime(p); time.sleep(60)
            if not os.path.exists(p) or os.path.getmtime(p) != m:
                continue
            try:
                sd = torch.load(p, map_location="cpu", mmap=True, weights_only=False)["state_dict"]
                torch.save({k: v.clone() for k, v in sd.items()}, out + ".tmp"); os.replace(out + ".tmp", out)
                want.discard(step); print("saved", out, flush=True)
            except Exception as e:
                print("retry", step, e, flush=True)
    time.sleep(60)
print("snap done, missing:", sorted(want), flush=True)
EOF
  snap=$!; trap 'kill $snap 2>/dev/null || true' EXIT   # a failed train must not leave it polling
  python nemo-src/examples/asr/speech_to_text_finetune.py --config-path="$W" --config-name=ft.yaml; mark train
fi
ls -la "$CKDIR"

# The run's own .nemo is the LAST step. Export the checkpoint with the lowest val_wer (best.nemo)
# and the mean of the AVG lowest (avg.nemo; averaging usually smooths a fine-tune's noise).
if ! done_ best; then
  python - "$BASE" "$CKDIR" "$W" "$AVG" <<'EOF'
import glob, re, sys, torch
from nemo.collections.asr.models import ASRModel
base, ckdir, out, k = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
cks = sorted((p for p in glob.glob(f"{ckdir}/*val_wer=*.ckpt") if not p.endswith("-last.ckpt")), key=lambda p: float(re.search(r"val_wer=([0-9.]+?)-", p)[1]))
m = ASRModel.restore_from(base, map_location="cpu")
sd = lambda p: torch.load(p, map_location="cpu", weights_only=False)["state_dict"]
print("best", cks[0])
m.load_state_dict(sd(cks[0]), strict=True)
m.save_to(f"{out}/best.nemo")
print("avg", cks[:k])
acc = None
for p in cks[:k]:
    s = sd(p)
    acc = {n: t.double().clone() for n, t in s.items()} if acc is None else {n: acc[n] + s[n].double() for n in acc}
ref = sd(cks[0])
m.load_state_dict({n: (acc[n] / len(cks[:k])).to(ref[n].dtype) if ref[n].is_floating_point() else ref[n] for n in acc}, strict=True)
m.save_to(f"{out}/avg.nemo")
EOF
  mark best
fi

# last.nemo: the final step; late.nemo: the mean of the SNAPS steps and the final one.
if ! done_ late; then
  python - "$BASE" "$CKDIR" "$W" <<'EOF'
import glob, sys, torch
from nemo.collections.asr.models import ASRModel
base, ckdir, out = sys.argv[1:4]
last = torch.load(glob.glob(f"{ckdir}/*-last.ckpt")[0], map_location="cpu", mmap=True, weights_only=False)["state_dict"]
m = ASRModel.restore_from(base, map_location="cpu")
m.load_state_dict(last, strict=True)
m.save_to(f"{out}/last.nemo")
snaps = sorted(glob.glob(f"{out}/snap/step*.pt"))
print("late", snaps, "+ last")
parts = [torch.load(p, map_location="cpu", weights_only=False) for p in snaps] + [last]
m.load_state_dict({k: (sum(p[k].double() for p in parts) / len(parts)).to(last[k].dtype) if last[k].is_floating_point() else last[k] for k in last}, strict=True)
m.save_to(f"{out}/late.nemo")
EOF
  mark late
fi

# Dropped-speech check against the base model, on val and on YouTube English never trained on
if ! done_ probe; then python "$HERE/prep_data.py" probe "$W/data" "$BASE" "$W/avg.nemo" "$W/late.nemo" "$W/last.nemo" | tee "$W/probe.txt"; mark probe; fi
