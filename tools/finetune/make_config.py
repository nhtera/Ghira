# SPDX-License-Identifier: Apache-2.0
"""Write the fine-tune config for NeMo's speech_to_text_finetune.py from the base .nemo.

The model architecture always comes from the .nemo (init_from_nemo_model). From the config
the script reads train_ds, validation_ds, optim, spec_augment and the trainer. We copy the
base model's own train_ds/validation_ds (so the prompt dictionary, bucketing and the
50/50 auto-vs-language prompt mix stay exactly as NVIDIA trained it) and override only
what a small fine-tune needs: plain (untarred) manifests, our batch size, a gentle LR.

    python make_config.py BASE.nemo OUT.yaml --train train.json --val val.json \
        --steps 6000 --batch-duration 200 [--lr 1e-4] [--devices 1] [--accelerator gpu]
"""

import argparse
import io
import tarfile

from omegaconf import OmegaConf


def base_config(nemo_path: str):
    with tarfile.open(nemo_path) as tar:
        for member in tar.getmembers():
            if member.name.endswith("model_config.yaml"):
                return OmegaConf.load(io.StringIO(tar.extractfile(member).read().decode()))
    raise SystemExit(f"no model_config.yaml in {nemo_path}")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("nemo")
    ap.add_argument("out")
    ap.add_argument("--train", required=True)
    ap.add_argument("--val", required=True)
    ap.add_argument("--exp-dir", default="exp")
    ap.add_argument("--steps", type=int, default=6000)
    ap.add_argument("--batch-duration", type=float, default=200)
    ap.add_argument("--accumulate", type=int, default=1)
    ap.add_argument("--lr", type=float, default=1e-4)
    ap.add_argument("--warmup", type=int, default=500)
    ap.add_argument("--val-every", type=int, default=500)
    ap.add_argument("--devices", type=int, default=1)
    ap.add_argument("--accelerator", default="gpu")
    ap.add_argument("--precision", default="bf16-mixed")
    ap.add_argument("--workers", type=int, default=8)
    a = ap.parse_args()

    base = base_config(a.nemo)

    train = base.train_ds
    train.manifest_filepath = a.train
    train.is_tarred = False
    train.tarred_audio_filepaths = None
    train.shard_manifests = False
    train.batch_duration = a.batch_duration
    train.num_workers = a.workers
    train.pin_memory = a.accelerator == "gpu"
    train.max_duration = 20
    train.num_buckets = 10

    val = base.validation_ds
    val.manifest_filepath = a.val
    val.batch_size = 16
    val.max_cuts = 16
    val.num_workers = min(a.workers, 4)
    val.pin_memory = a.accelerator == "gpu"
    # Validation always uses the language prompt, so val_wer is stable between checks.
    val.default_prompt_mode = "langID"

    cfg = OmegaConf.create(
        {
            "name": "nemotron-vi-ft",
            "init_from_nemo_model": a.nemo,
            "model": {
                "train_ds": train,
                "validation_ds": val,
                "tokenizer": {"update_tokenizer": False},
                "spec_augment": base.spec_augment,
                "optim": {
                    "name": "adamw",
                    "lr": a.lr,
                    "betas": [0.9, 0.98],
                    "weight_decay": 1e-3,
                    "sched": {"name": "CosineAnnealing", "warmup_steps": a.warmup, "min_lr": a.lr / 20},
                },
            },
            "trainer": {
                "devices": a.devices,
                "num_nodes": 1,
                "accelerator": a.accelerator,
                "strategy": "auto",
                "max_epochs": -1,
                "max_steps": a.steps,
                # lhotse loaders have no epoch length: one "epoch" = val_every steps.
                "limit_train_batches": a.val_every,
                "val_check_interval": 1.0,
                "accumulate_grad_batches": a.accumulate,
                "gradient_clip_val": 0.5,
                "precision": a.precision,
                "log_every_n_steps": 25,
                "enable_progress_bar": False,
                "num_sanity_val_steps": 0,
                "enable_checkpointing": False,
                "logger": False,
                "use_distributed_sampler": False,
                "benchmark": False,
            },
            "exp_manager": {
                "exp_dir": a.exp_dir,
                "name": "nemotron-vi-ft",
                "create_tensorboard_logger": True,
                "create_checkpoint_callback": True,
                "checkpoint_callback_params": {
                    "monitor": "val_wer",
                    "mode": "min",
                    "save_top_k": 3,
                    "save_last": True,
                    "always_save_nemo": True,
                },
                # A stopped or replaced box picks up from the last checkpoint.
                "resume_if_exists": True,
                "resume_ignore_no_checkpoint": True,
            },
        }
    )
    OmegaConf.save(cfg, a.out)
    print(f"wrote {a.out}: {a.steps} steps, batch {a.batch_duration}s x{a.accumulate}, lr {a.lr}")


if __name__ == "__main__":
    main()
