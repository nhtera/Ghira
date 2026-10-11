# SPDX-License-Identifier: Apache-2.0
"""NeMo-Speech.cpp's convert_model.py with torch.load forced to weights_only=True.

A .nemo holds a pickled checkpoint; this keeps a third-party one from unpickling arbitrary
objects. Needs the converter's requirements plus librosa (without it the mel filterbank
`preprocessor.fb` is silently dropped and the model reads 4-9 points worse).

    python safe_convert.py MODEL.nemo -o OUT.q8_0.gguf --outtype q8_0
"""

import runpy
import sys
from pathlib import Path

import torch

CONVERTER = Path(__file__).resolve().parents[2] / "third_party" / "NeMo-Speech.cpp"
_load = torch.load


def safe_load(*a, **k):
    k["weights_only"] = True
    return _load(*a, **k)


torch.load = safe_load
sys.path.insert(0, str(CONVERTER))
sys.argv = ["convert_model.py", *sys.argv[1:]]
runpy.run_path(str(CONVERTER / "convert_model.py"), run_name="__main__")
