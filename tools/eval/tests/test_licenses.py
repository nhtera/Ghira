# SPDX-License-Identifier: Apache-2.0
"""Every runtime dependency of ghi-eval (default install, no extras) must be permissively licensed."""

import re
from importlib import metadata

from packaging.requirements import Requirement

ALLOWED_SPDX = {"MIT", "BSD-2-Clause", "BSD-3-Clause", "0BSD", "Apache-2.0", "ISC", "PSF-2.0",
                "MPL-2.0", "Zlib", "CC0-1.0", "Unlicense"}  # fmt: skip
ALLOWED_CLASSIFIERS = {
    "License :: OSI Approved :: MIT License",
    "License :: OSI Approved :: BSD License",
    "License :: OSI Approved :: Apache Software License",
    "License :: OSI Approved :: ISC License (ISCL)",
    "License :: OSI Approved :: Python Software Foundation License",
    "License :: OSI Approved :: Mozilla Public License 2.0 (MPL 2.0)",
}
# Distributions whose metadata carries no usable license field. Each entry names the
# real license, checked against the LICENSE file in the installed dist-info.
OVERRIDES = {
    "pyannote-core": "MIT",  # dist-info/licenses/LICENSE: MIT, (c) CNRS
    "pyannote-database": "MIT",  # dist-info/licenses/LICENSE: MIT, (c) CNRS
    "pyannote-metrics": "MIT",  # dist-info/licenses/LICENSE: MIT, (c) CNRS
}
FORBIDDEN = re.compile(r"\b(A|L)?GPL\b|SSPL|non-?commercial|Commons Clause", re.I)


def runtime_distributions(root="ghi-eval"):
    seen, todo = {}, [root]
    while todo:
        name = metadata.distribution(todo.pop()).metadata["Name"]
        key = name.lower().replace("_", "-")
        if key in seen:
            continue
        dist = metadata.distribution(name)
        seen[key] = dist
        for line in dist.requires or []:
            req = Requirement(line)
            if req.marker is None or req.marker.evaluate({"extra": ""}):
                todo.append(req.name)
    return seen


def license_problem(key, dist):
    md = dist.metadata
    # Only the structured fields are screened for copyleft. The free-text License field of
    # scipy and pandas is the whole LICENSE file including notices for bundled third-party
    # code (scipy wheels ship libgfortran under the GCC runtime library exception); their
    # own license is BSD-3-Clause, declared by the classifier below.
    text = " ".join([md.get("License-Expression") or ""] + md.get_all("Classifier", []))
    if FORBIDDEN.search(text):
        return f"forbidden license text: {text[:80]}"
    if key in OVERRIDES:
        return None
    expr = md.get("License-Expression")
    if expr:
        parts = [p for p in re.split(r"\s+(?:AND|OR|WITH)\s+|[()]", expr) if p.strip()]
        return None if all(p.strip() in ALLOWED_SPDX for p in parts) else f"expression {expr}"
    classifiers = {c for c in md.get_all("Classifier", []) if c.startswith("License ::")}
    if classifiers:
        return None if classifiers <= ALLOWED_CLASSIFIERS else f"classifiers {sorted(classifiers)}"
    lic = md.get("License") or ""
    if re.search(r"\b(MIT|BSD|Apache|ISC|PSF|Python Software Foundation|MPL)\b", lic):
        return None
    return "no license metadata; add an override with the real license"


def test_runtime_dependencies_are_permissive():
    dists = runtime_distributions()
    assert {"pyannote-metrics", "jiwer", "psutil", "pyyaml", "jsonschema", "numpy"} <= set(dists)
    assert "nemo-toolkit" not in dists and "faster-whisper" not in dists  # extras stay out
    problems = {k: p for k, d in dists.items() if k != "ghi-eval" and (p := license_problem(k, d))}
    assert problems == {}


def test_overrides_match_installed_license_files():
    for key in OVERRIDES:
        dist = metadata.distribution(key)
        files = [f for f in dist.files or [] if "LICENSE" in f.name.upper()]
        assert files, f"{key}: no license file to back the override"
        assert "MIT License" in files[0].read_text()[:200]
