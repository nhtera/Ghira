# SPDX-License-Identifier: Apache-2.0
# Weight-space blends of Nemotron fine-tunes (all from the same base).
#   blend.py lerp BASE.nemo FT.nemo ALPHA OUT.nemo     theta = base + alpha * (ft - base)
#   blend.py soup OUT.nemo A.nemo B.nemo [...]          theta = mean(A, B, ...)
# The output copies the first fine-tune's archive (config, tokenizer) and swaps
# model_weights.ckpt. Non-float tensors (counters) come from that fine-tune.
import io
import sys
import tarfile

import torch


def weights(path):
    with tarfile.open(path) as tar:
        for m in tar.getmembers():
            if m.name.endswith("model_weights.ckpt"):
                return torch.load(io.BytesIO(tar.extractfile(m).read()), map_location="cpu", weights_only=True)
    raise SystemExit(f"no weights in {path}")


def write(template, out, sd):
    buf = io.BytesIO()
    torch.save(sd, buf)
    data = buf.getvalue()
    with tarfile.open(template) as src, tarfile.open(out, "w") as dst:
        for m in src.getmembers():
            if m.name.endswith("model_weights.ckpt"):
                info = tarfile.TarInfo(m.name)
                info.size = len(data)
                dst.addfile(info, io.BytesIO(data))
            elif m.isfile():
                dst.addfile(m, src.extractfile(m))
            else:
                dst.addfile(m)


def is_float(t):
    return torch.is_tensor(t) and t.is_floating_point()


def main():
    mode = sys.argv[1]
    if mode == "lerp":
        base_p, ft_p, alpha, out = sys.argv[2], sys.argv[3], float(sys.argv[4]), sys.argv[5]
        base, ft = weights(base_p), weights(ft_p)
        assert base.keys() == ft.keys(), set(base) ^ set(ft)
        sd = {}
        for k, v in ft.items():
            if is_float(v):
                b = base[k].to(torch.float32)
                sd[k] = (b + alpha * (v.to(torch.float32) - b)).to(v.dtype)
            else:
                sd[k] = v
        write(ft_p, out, sd)
    elif mode == "soup":
        out, parts = sys.argv[2], sys.argv[3:]
        sds = [weights(p) for p in parts]
        sd = {}
        for k, v in sds[0].items():
            if is_float(v):
                sd[k] = (sum(s[k].to(torch.float32) for s in sds) / len(sds)).to(v.dtype)
            else:
                sd[k] = v
        write(parts[0], out, sd)
    else:
        raise SystemExit(__doc__)
    print("wrote", out)


if __name__ == "__main__":
    main()
