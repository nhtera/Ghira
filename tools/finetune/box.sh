#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs ON the rented GPU box (pytorch/pytorch CUDA image), from the folder vast.sh pushed:
#   prep_data.py make_config.py ghi_eval/ box.sh
# Every stage leaves a marker in $W/state, so re-running after a stop resumes where it was.
#   STEPS=5000 BATCH=160 ACCUM=2 YODAS=20 LIBRI=7 bash box.sh
set -euo pipefail
W=${W:-/workspace}
HERE=$(cd "$(dirname "$0")" && pwd)
NEMO_SHA=${NEMO_SHA:-e4a34fd25f895ecd7f64f0e56bc288165e078da9}   # NeMo main, 2026-10-10 (dry run on this)
BASE_REV=ea30d66debe3740a08b573244286791d423d6b3e                  # nvidia/nemotron-3.5-asr-streaming-0.6b
STEPS=${STEPS:-5000} BATCH=${BATCH:-160} ACCUM=${ACCUM:-2} YODAS=${YODAS:-20} LIBRI=${LIBRI:-7}
mkdir -p "$W/state"
cd "$W"
done_() { [ -f "$W/state/$1" ]; }
mark() { touch "$W/state/$1"; echo "== $1 done $(date -u +%H:%M:%S)"; }

if ! done_ env; then
  apt-get update -qq && apt-get install -y -qq git libsndfile1 >/dev/null
  pip install -q Cython packaging
  pip install -q "nemo_toolkit[asr,cu12] @ git+https://github.com/NVIDIA-NeMo/NeMo.git@$NEMO_SHA" soxr soundfile pyarrow huggingface_hub
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
if ! done_ fetch; then python "$HERE/prep_data.py" fetch "$W/data" --yodas-shards "$YODAS" --libri-files "$LIBRI"; mark fetch; fi
if ! done_ label; then python "$HERE/prep_data.py" label "$W/data" "$BASE" --batch 64; mark label; fi
if ! done_ select; then python "$HERE/prep_data.py" select "$W/data"; mark select; fi

python "$HERE/make_config.py" "$BASE" "$W/ft.yaml" --train "$W/data/train.json" --val "$W/data/val.json" \
  --exp-dir "$W/exp" --steps "$STEPS" --batch-duration "$BATCH" --accumulate "$ACCUM" --workers 8
export PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True
if ! done_ train; then python nemo-src/examples/asr/speech_to_text_finetune.py --config-path="$W" --config-name=ft.yaml; mark train; fi
ls -la "$W/exp/nemotron-vi-ft/checkpoints/"

# The run's own .nemo is the LAST step; export the checkpoint with the lowest val_wer instead
if ! done_ best; then
  python - "$BASE" "$W/exp/nemotron-vi-ft/checkpoints" "$W/best.nemo" <<'EOF'
import glob, re, sys, torch
from nemo.collections.asr.models import ASRModel
base, ckdir, out = sys.argv[1:]
ck = min(glob.glob(f"{ckdir}/*val_wer=*.ckpt"), key=lambda p: float(re.search(r"val_wer=([0-9.]+?)-", p)[1]))
print("best", ck)
m = ASRModel.restore_from(base, map_location="cpu")
m.load_state_dict(torch.load(ck, map_location="cpu", weights_only=False)["state_dict"], strict=True)
m.save_to(out)
EOF
  mark best
fi
