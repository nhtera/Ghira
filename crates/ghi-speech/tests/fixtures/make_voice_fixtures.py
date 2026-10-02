# SPDX-License-Identifier: Apache-2.0
"""Regenerates the voice-embedder parity fixtures (no recordings, no model in git).

    uv run --with onnxruntime --with numpy --with kaldi-native-fbank \
        crates/ghi-speech/tests/fixtures/make_voice_fixtures.py models/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx

The signal is integer-only (LCG noise + two triangle waves), so Rust rebuilds it
bit-exactly (see tests/voice.rs `signal`). Reference fbank: kaldi-native-fbank
(what sherpa-onnx uses); reference embeddings: onnxruntime on that fbank.

Writes little-endian f32 files next to this script:
  fbank_head.f32 / fbank_tail.f32  first / last 20 raw fbank frames (20 x 80)
  fbank_mean.f32                   per-bin mean over all frames (80)
  emb_200/400/600.f32              embedding (192) of the first 200 / 400 / 600 frames
"""
import sys
from pathlib import Path

import kaldi_native_fbank as knf
import numpy as np
import onnxruntime as ort

N = 97000  # 604 frames
SHIFT, FRAME = 160, 400


def signal(n):
    out = np.empty(n, dtype=np.int16)
    s = 12345
    for i in range(n):
        s = (s * 1103515245 + 12345) & 0x7FFFFFFF
        noise = ((s >> 16) & 0x7FF) - 1024
        tri1 = abs((i * 37) % 200 - 100) * 60 - 3000  # period 200 -> 80 Hz
        tri2 = abs((i * 11) % 64 - 32) * 90 - 1440  # period 64 -> 250 Hz
        out[i] = noise + tri1 + tri2
    return out


def fbank(x):
    o = knf.FbankOptions()
    o.frame_opts.dither = 0.0
    o.frame_opts.snip_edges = True
    o.frame_opts.samp_freq = 16000
    o.frame_opts.remove_dc_offset = True
    o.frame_opts.window_type = "povey"
    o.frame_opts.preemph_coeff = 0.97
    o.mel_opts.num_bins = 80
    o.mel_opts.low_freq = 20.0
    o.mel_opts.high_freq = 0.0
    o.use_energy = False
    f = knf.OnlineFbank(o)
    f.accept_waveform(16000, (x.astype(np.float32) / 32768.0).tolist())
    f.input_finished()
    return np.stack([f.get_frame(i) for i in range(f.num_frames_ready)]).astype(np.float32)


def main():
    here = Path(__file__).parent
    sess = ort.InferenceSession(sys.argv[1], providers=["CPUExecutionProvider"])
    x = signal(N)
    frames = fbank(x)
    assert frames.shape == (604, 80), frames.shape
    frames[:20].tofile(here / "fbank_head.f32")
    frames[-20:].tofile(here / "fbank_tail.f32")
    frames.mean(axis=0).astype(np.float32).tofile(here / "fbank_mean.f32")
    for t in (200, 400, 600):
        # Same window the embedder takes: the first t frames' worth of samples.
        win = fbank(x[: FRAME + (t - 1) * SHIFT])
        assert win.shape[0] == t
        win = win - win.mean(axis=0, keepdims=True)
        emb = sess.run(None, {"x": win[None]})[0][0].astype(np.float32)
        emb.tofile(here / f"emb_{t}.f32")
        print(t, emb[:4])


main()
