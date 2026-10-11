#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs ON the rented GPU box (pytorch/pytorch CUDA image), from the folder vast.sh pushed:
#   prep_data.py make_config.py ghi_eval/ box.sh
# Every stage leaves a marker in $W/state, so re-running after a stop resumes where it was.
#   STEPS=5000 LR=3e-5 WARMUP=300 VAL_EVERY=500 BATCH=160 ACCUM=2 YODAS=80 YODAS_EN=25 LIBRI=7 AUTO=0.5 JOIN=0.5 bash box.sh
# Pilot 1 (lr 1e-4, 5000 steps) dropped whole English utterances; pilot 2 (lr 3e-5, 2000 steps on
# 128 h) plateaued halfway and dropped the first words of live lines under the auto prompt. These
# defaults: all of YODAS vi000 + FOSD, more varied English, half the clips on the auto prompt.
# Run 3 still lost a new speaker's first seconds in a continuous stream (one tag ending every
# target taught "nothing after a tag"): targets now keep the model's own tags and half the clips
# are joined 2-3 at a time.
set -euo pipefail
W=${W:-/workspace}
HERE=$(cd "$(dirname "$0")" && pwd)
NEMO_SHA=${NEMO_SHA:-e4a34fd25f895ecd7f64f0e56bc288165e078da9}   # NeMo main, 2026-10-10 (dry run on this)
BASE_REV=ea30d66debe3740a08b573244286791d423d6b3e                  # nvidia/nemotron-3.5-asr-streaming-0.6b
STEPS=${STEPS:-5000} LR=${LR:-3e-5} WARMUP=${WARMUP:-300} VAL_EVERY=${VAL_EVERY:-500}
BATCH=${BATCH:-160} ACCUM=${ACCUM:-2} YODAS=${YODAS:-80} YODAS_EN=${YODAS_EN:-25} LIBRI=${LIBRI:-7}
AUTO=${AUTO:-0.5} JOIN=${JOIN:-0.5} AVG=${AVG:-3}
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

# (if-blocks, not `a || b`: set -e must stop the run when a stage fails)
if ! done_ fetch; then python "$HERE/prep_data.py" fetch "$W/data" --yodas-shards "$YODAS" --yodas-en-shards "$YODAS_EN" --libri-files "$LIBRI" --fosd; mark fetch; fi
if ! done_ label; then python "$HERE/prep_data.py" label "$W/data" "$BASE" --batch 64; mark label; fi
if ! done_ select; then python "$HERE/prep_data.py" select "$W/data" --auto-frac "$AUTO" --join-frac "$JOIN"; mark select; fi

python "$HERE/make_config.py" "$BASE" "$W/ft.yaml" --train "$W/data/train.json" --val "$W/data/val.json" \
  --exp-dir "$W/exp" --steps "$STEPS" --batch-duration "$BATCH" --accumulate "$ACCUM" --lr "$LR" --warmup "$WARMUP" \
  --val-every "$VAL_EVERY" --workers 8
export PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True
if ! done_ train; then python nemo-src/examples/asr/speech_to_text_finetune.py --config-path="$W" --config-name=ft.yaml; mark train; fi
ls -la "$W/exp/nemotron-vi-ft/checkpoints/"

# The run's own .nemo is the LAST step. Export the checkpoint with the lowest val_wer (best.nemo)
# and the mean of the AVG lowest (avg.nemo; averaging usually smooths a fine-tune's noise).
if ! done_ best; then
  python - "$BASE" "$W/exp/nemotron-vi-ft/checkpoints" "$W" "$AVG" <<'EOF'
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

# Dropped-speech check against the base model, on val and on YouTube English never trained on
if ! done_ probe; then python "$HERE/prep_data.py" probe "$W/data" "$BASE" "$W/best.nemo" "$W/avg.nemo" | tee "$W/probe.txt"; mark probe; fi
