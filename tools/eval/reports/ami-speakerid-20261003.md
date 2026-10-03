# Speaker-ID accuracy on AMI (CAM++ zh_en via ghi-speech `voice`)

Aggregate numbers only. Data: AMI test series ES2004, IS1009, TS3003, EN2002
(a-d each; same four people per series), far-field Array1-01, 16 meetings,
63 speaker-meetings. Our real window policy: single-speaker turns, 6.1/4.1/2.1 s
crops, at most 8 windows per speaker per meeting, mean then L2-normalised.
Reproduce: fetch with `scripts/fetch_public_sets.py --sets ami-sdm --subset full`, run
`cargo test -p ghi-core --features voice --release --test voice_real -- --ignored ami_dump_voices`,
then `ghi-eval speakerid-report --dataset data/ami-sdm --voices data/ami-sdm/voices.json --fill-persons --out reports/ami-speakerid.json`.

| Trial set | Trials (targets) | EER | Max non-target | Threshold at FAR 1% | at FAR 0.1% | FRR at 0.70 | FAR at 0.50 |
|---|---|---|---|---|---|---|---|
| Pairwise cross-meeting, all | 1860 (93) | 0.0% | 0.581 | 0.467 | 0.536 | 6.5% | 0.6% |
| Pairwise, same series (same room/mic) | 372 (93) | 0.0% | 0.581 | 0.531 | 0.582 | 6.5% | 3.6% |
| Profile (mean of other meetings) vs held-out meeting | 249 (63) | 0.0% | 0.550 | 0.535 | 0.550 | 3.2% | 4.3% |

Score distributions (percentiles 1/5/25/50/75/95/99), pairwise same series:
target 0.595 0.682 0.839 0.895 0.926 0.943 0.948; non-target -0.031 0.020 0.123 0.203 0.316 0.467 0.523.

Caveats: English only; one far-field channel per series; 93 / 63 targets, so
rare tails (a 0.1% FAR) are not well measured; cross-lingual and call-mode
(speakerphone) conditions are not covered. Vietnamese needs its own check.
