# Re-clustering past the 8-speaker cap (phase 14d S3)

Aggregate numbers only. System: Nemotron 3 diarizer (capped at 8) vs the same
diarizer run on ~120 s windows (cut at quiet points), each (window, speaker)
embedded with CAM++ (our window policy), average-linkage clustering on cosine
across windows, clusters with under 8 s of speech joined to their nearest
neighbour. DER at collar 0.25 s, overlap scored.

Data (VoxConverse test, CC-BY-4.0, not in git): `scripts/fetch_public_sets.py --sets voxconverse --subset many|ctrl --out data/<name>`;
run `vox_eval` in `crates/ghi-core/tests/recluster.rs`, score with `ghi-eval recluster-report`.

## 16 files with 9-21 reference speakers (1.96 h; 207 reference speakers)

| Variant | DER | confusion | missed | speaker-count MAE | speakers found |
|---|---|---|---|---|---|
| capped (today) | 27.57% | 12.97 | 13.93 | 5.06 | 126 |
| re-cluster, threshold 0.3 | 17.34% | 11.11 | 5.54 | 4.44 | 136 |
| 0.4 | 9.07% | 5.39 | 2.99 | 3.56 | 150 |
| 0.5 | 7.13% | 3.87 | 2.56 | 3.19 | 156 |
| **0.6 (shipped)** | **7.13%** | 3.87 | 2.56 | 3.19 | 156 |
| 0.7 | 7.13% | 3.87 | 2.56 | 3.19 | 156 |

Identical from 0.5 to 0.7: the average-linkage scores fall in two clear groups
(same voice above 0.7, different voices below 0.5), and the threshold is verified to reach the
clustering (0.3 and 0.4 differ; a unit test checks it). After the review fixes (labels of one
window never merge; voice-less speakers never take a neighbour's label) DER improved from 9.5% to 7.1%.

## Control: 12 files with 7-8 speakers (89 reference speakers)

Capped 5.92% DER, MAE 0.42; every re-cluster variant identical (fewer than 9 clusters
come out, so the capped result is kept): no change.

## Runtime (release, M-series, Metal diarizer + CPU CAM++)

Capped diarization 89 s per hour of audio; the re-cluster analysis (windowed
diarization + embedding) 95 s per hour of audio. It runs only when the diarizer
saturates, so the final pass pays about 95 s per audio hour extra on those meetings.

Gate result: beats the capped baseline on DER (27.6 -> 7.1%) and speaker-count
error (5.1 -> 3.2) and leaves <= 8-speaker files unchanged, so `recluster::ENABLED = true`.
Caveats: English broadcast-style audio; undercounts still (156 of 207 speakers found).
