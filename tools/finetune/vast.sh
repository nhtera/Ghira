#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Mac-side helper for the Vietnamese Nemotron fine-tune on a rented Vast.ai GPU.
# Needs: `vastai` CLI with its API key set, and an SSH key registered on the account
# (VAST_SSH_KEY, default ~/.ssh/vast_ed25519). Only public data, NVIDIA's base model and our own
# fine-tunes go to the box; meeting recordings never leave this Mac.
#
#   vast.sh create OFFER_ID [DISK_GB]   rent the offer, wait until it runs, save its id
#   vast.sh push [LABELS_DIR] [TEACHER.nemo]
#                                       copy the scripts (+ the eval kit's text normaliser); with a
#                                       saved hyp.json from an earlier run, the label stage is skipped;
#                                       TEACHER labels the uncaptioned VietMed audio (box.sh VIETMED_UN)
#   vast.sh run [ENV=VAL ...]           start box.sh in the background (resumes by stage)
#   vast.sh status                      last log lines, GPU use, credit
#   vast.sh pull                        fetch best/avg/late/last .nemo, probe, log, manifests to $OUT (./ft-out)
#   vast.sh destroy                     delete the instance and its disk (stops all charges)
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
VAST=${VAST:-vastai}
KEY=${VAST_SSH_KEY:-$HOME/.ssh/vast_ed25519}
STATE=${VAST_STATE:-$HOME/.config/vastai/ghira-ft-instance}
IMAGE=${IMAGE:-pytorch/pytorch:2.8.0-cuda12.8-cudnn9-devel}
OUT=${OUT:-./ft-out}

id_() { cat "$STATE"; }
ssh_target() {
  "$VAST" ssh-url "$(id_)" | sed -E 's#ssh://([^:]+):([0-9]+)#\1 \2#'
}
run_ssh() {
  read -r host port < <(ssh_target)
  ssh -n -i "$KEY" -p "$port" -o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30 "$host" "$@"
}

case "${1:-}" in
  create)
    offer=${2:?offer id}; disk=${3:-150}
    "$VAST" create instance "$offer" --image "$IMAGE" --disk "$disk" --ssh --direct --raw \
      | python3 -I -c "import json,sys; print(json.load(sys.stdin)['new_contract'])" > "$STATE"
    echo "instance $(id_); waiting for it to run..."
    for _ in $(seq 1 90); do
      s=$("$VAST" show instance "$(id_)" --raw | python3 -I -c "import json,sys; print(json.load(sys.stdin).get('actual_status'))")
      [ "$s" = running ] && { echo running; exit 0; }
      sleep 10
    done
    echo "not running after 15 min (status $s): check it, or destroy and pick another offer" >&2; exit 1 ;;
  push)
    read -r host port < <(ssh_target)
    tmp=$(mktemp -d); mkdir -p "$tmp/ft"
    cp "$HERE"/prep_data.py "$HERE"/make_config.py "$HERE"/box.sh "$HERE"/vimedcss_bench_videos.txt "$tmp/ft/"
    cp -r "$ROOT/tools/eval/ghi_eval" "$tmp/ft/"
    rsync -az -e "ssh -i $KEY -p $port -o StrictHostKeyChecking=accept-new" "$tmp/ft/" "$host:/workspace/ft/"
    if [ -n "${2:-}" ]; then
      rsync -az -e "ssh -i $KEY -p $port" --rsync-path="mkdir -p /workspace/data && rsync" "$2/hyp.json" "$host:/workspace/data/"
    fi
    if [ -n "${3:-}" ]; then
      rsync -a --partial -e "ssh -i $KEY -p $port" "$3" "$host:/workspace/teacher.nemo"
    fi
    rm -rf "$tmp"; echo pushed ;;
  run)
    shift; run_ssh "cd /workspace && (env $* setsid nohup bash ft/box.sh >> ft.log 2>&1 < /dev/null &); echo started" ;;
  status)
    run_ssh "tail -n ${2:-15} /workspace/ft.log; ls /workspace/state 2>/dev/null | tr '\n' ' '; echo; nvidia-smi --query-gpu=utilization.gpu,memory.used,memory.total --format=csv,noheader; df -h /workspace | tail -1"
    "$VAST" show user --raw | python3 -I -c "import json,sys; print('credit \$%.2f' % json.load(sys.stdin)['credit'])" ;;
  pull)
    read -r host port < <(ssh_target)
    mkdir -p "$OUT"
    rsync -az --progress -e "ssh -i $KEY -p $port" \
      "$host:/workspace/best.nemo" "$host:/workspace/avg.nemo" "$host:/workspace/late.nemo" "$host:/workspace/last.nemo" "$host:/workspace/probe.txt" \
      "$host:/workspace/ft.log" "$host:/workspace/data/*.json" "$OUT/" ;;
  destroy)
    # -y: the CLI's own prompt reads no stdin here, prints "Aborted." and still exits 0
    "$VAST" destroy instance "$(id_)" -y
    "$VAST" show instances --raw | python3 -I -c "import json,sys; sys.exit(any(i['id']==$(id_) for i in json.load(sys.stdin)))" \
      && rm -f "$STATE" && echo destroyed ;;
  *) sed -n '3,15p' "$0"; exit 2 ;;
esac
