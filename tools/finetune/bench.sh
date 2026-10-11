#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Mac-side benchmark of a fine-tuned Nemotron ASR candidate, the same for every run:
#   bench.sh setup                    converter venv (models/finetune/convenv)
#   bench.sh convert X.nemo DIR       X.nemo -> DIR/nemotron-3.5-asr-streaming-0.6b.q8_0.gguf (+ the diarizer link)
#   bench.sh run NAME DIR             long-form final pass (FLEURS-vi, ViMedCSS, VietMed, FLEURS-en joined 5 at a time),
#                                     per-clip FLEURS-en + Earnings-21, live WER; results in models/finetune/bench/NAME
# Needs target/release/ghi (--features nemo), the live_bench test, and the eval kit's bench-* sets
# (fetch_public_sets.py + the subsets the eval reports describe). ~25 min per model.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
R=$(cd "$HERE/../.." && pwd)
E=$R/tools/eval
VENV=$R/models/finetune/convenv
G=$R/target/release/ghi
ASR=nemotron-3.5-asr-streaming-0.6b.q8_0.gguf

case "${1:-}" in
  setup)
    uv venv -q -p 3.11 "$VENV"
    uv pip install -q --python "$VENV/bin/python" -r "$R/third_party/NeMo-Speech.cpp/requirements.txt" librosa
    "$VENV/bin/python" -c "import torch, gguf, librosa; print('convenv ok', torch.__version__, librosa.__version__)" ;;
  convert)
    nemo=${2:?X.nemo}; dir=${3:?DIR}; mkdir -p "$dir"
    "$VENV/bin/python" -I "$HERE/safe_convert.py" "$nemo" -o "$dir/$ASR" --outtype q8_0 2>&1 | grep -iE "warn|wrote|error" || true
    ln -sf "$R/models/Nemotron-3-Diarization.q8_0.gguf" "$dir/"
    "$VENV/bin/python" -I -c "
import sys; from gguf import GGUFReader
names = {t.name for t in GGUFReader(sys.argv[1]).tensors}
sys.exit(0 if any('preprocessor' in n for n in names) else 'no preprocessor tensors: is librosa installed?')" "$dir/$ASR"
    shasum -a 256 "$dir/$ASR" ;;
  run)
    name=${2:?NAME}; dir=$(cd "${3:?DIR}" && pwd); out=$R/models/finetune/bench/$name; mkdir -p "$out"
    fe=$E/data/bench-fleurs-en
    if [ ! -f "$fe/runs.json" ]; then
      mkdir -p "$fe" && ln -sfn "$E/data/fleurs-en/audio" "$fe/audio" && ln -sfn "$E/data/fleurs-en/refs" "$fe/refs"
      cp "$E/data/fleurs-en/manifest.yaml" "$fe/"
      python3 -I -c "
import json, re, sys
ids = re.findall(r'^- id: (.+)$', open(sys.argv[1] + '/manifest.yaml').read(), re.M)
json.dump([ids[i:i + 5] for i in range(0, len(ids), 5)], open(sys.argv[1] + '/runs.json', 'w'))" "$fe"
    fi
    w=$out/ghi
    printf '#!/bin/sh\nif [ "$1" = transcribe ]; then shift; exec %s transcribe --asr-model %s "$@"; fi\nexec %s "$@"\n' "$G" "$dir/$ASR" "$G" > "$w"; chmod +x "$w"
    cd "$E"; export GHI_MODELS_DIR=$R/models
    for set in bench-fleurs-vi bench-vimedcss bench-vietmed bench-fleurs-en; do
      uv run --locked python -I "$HERE/long_form.py" "data/$set" "$out/long-$set" m="$w transcribe {wav} --pass final" 2>>"$out/long.err"
    done
    for set in fleurs-en earnings21; do
      uv run --locked ghi-eval run --dataset "data/$set" --system "ghi:$w" --tasks asr --pass final --run-id "$set" --out "$out" > "$out/$set.log" 2>&1
      echo "$name $set clip $(grep -E '^\| all \| all' "$out/$set"/report-*.md | awk -F'|' '{print "wer="$10}')"
    done
    cd "$R"
    GHI_MODELS_DIR=$dir GHI_LIVE_BENCH_SPEED=0 GHI_LIVE_BENCH_SECONDS=4000 GHI_LIVE_BENCH_SETS=kit:bench-fleurs-vi,kit:bench-vimedcss,kit:bench-vietmed,en \
      GHI_LIVE_BENCH_LABEL=$name cargo test -q -p ghi-core --features nemo --release --test live_bench -- --ignored --nocapture > "$out/live.log" 2>&1
    grep BENCH "$out/live.log" | sed -E 's/\| commit.*wer=/ wer=/' ;;
  *) sed -n '3,9p' "$0"; exit 2 ;;
esac
