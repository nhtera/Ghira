# SPDX-License-Identifier: Apache-2.0
"""The product acceptance suite (doc 02 section 1, doc 05 section 9) in one command.

`ghi-eval acceptance --dataset D [--dataset D2 ...] --system ghi` runs, per dataset, a
`--pass final` run and a `--pass live --realtime` run, pools the per-file scores of all
datasets, applies gates.yaml (word-error and DER gates from the final runs, lag gates from the
live runs) and adds the RT-7 timing checks. One aggregate report (JSON + Markdown) states
pass / best_effort / fail per gate. Only aggregates go into it; the privacy lint of the
eval kit runs over both files before they are written (formats.md section 6).

RT-7 timing (from the harness-measured per-task wall times in `hyp/<id>.perf.json`):
  - `notes_after_stop`: the notes task of the live run (notes made from the live transcript,
    which is complete at stop) must take <= 180 s on every file.
  - `refined_notes`: asr + diar + notes wall time of the final run must be <= 600 s per hour of
    audio on every file (scaled by the file's duration, with a 60 s floor for model loading).
"""

from __future__ import annotations

import json
import platform
import subprocess
from collections.abc import Callable
from datetime import datetime
from pathlib import Path

import psutil

from . import __version__
from .errors import HarnessError, PrivacyLeak
from .manifest import Manifest, load_manifest
from .metrics import percentile
from .privacy import LintIndex, build_index, lint_document, lint_text
from .report import (
    BUCKET_ORDER,
    LAG_METRICS,
    LANG_ORDER,
    METRIC_KEYS,
    SETTING_ORDER,
    _round,
    _table,
    aggregate,
    evaluate_gates,
    load_gates,
    public_spec,
    write_report,
)
from .run import execute, load_run, utc_now, write_json
from .score import load_scores, score_run

RT7_NOTES_AFTER_STOP_S = 180.0
RT7_REFINED_S_PER_HOUR = 600.0
RT7_REFINED_FLOOR_S = 60.0

# Acceptance items that no dataset run can measure; the owner records their result with
# `--external ID=pass|fail` (the scripts that produce them are named in docs/release).
EXTERNAL_IDS = ("crash_safety", "strict_offline_audit", "vn_search")
EXTERNAL_STATUSES = ("pass", "fail")

FINAL_TASKS = ["asr", "diar", "notes"]
LIVE_TASKS = ["asr", "diar", "stream", "notes"]


def _hardware() -> dict:
    """What the numbers were measured on (never a hostname or user name)."""
    chip = platform.processor() or platform.machine()
    try:
        chip = (
            subprocess.run(
                ["sysctl", "-n", "machdep.cpu.brand_string"],
                capture_output=True,
                text=True,
                timeout=5,
            ).stdout.strip()
            or chip
        )
    except (OSError, subprocess.SubprocessError):
        pass
    return {
        "chip": chip.replace("/", "-"),
        "ram_gb": round(psutil.virtual_memory().total / 2**30),
        "os": f"{'macOS' if platform.mac_ver()[0] else platform.system()} {platform.mac_ver()[0] or platform.release()}".replace(
            "/", "-"
        ),
    }


def _perf_docs(run_dir: Path, manifest: Manifest) -> list[tuple[dict, float | None]]:
    """(perf.json document, audio seconds) for every file of the manifest that has one."""
    out = []
    for entry in manifest.files:
        path = Path(run_dir) / "hyp" / f"{entry.id}.perf.json"
        if not path.is_file():
            continue
        try:
            doc = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        out.append((doc, manifest.duration(entry)))
    return out


def timing_checks(
    live_runs: list[tuple[Path, Manifest]], final_runs: list[tuple[Path, Manifest]]
) -> list[dict]:
    """The RT-7 checks over all runs. Status `no_data` when the notes task did not run."""
    notes_s = [
        d["notes"]["wall_s"]
        for run_dir, manifest in live_runs
        for d, _ in _perf_docs(run_dir, manifest)
        if "notes" in d
    ]
    check_a = {
        "id": "notes_after_stop",
        "limit_s": RT7_NOTES_AFTER_STOP_S,
        "files": len(notes_s),
        "max_s": round(max(notes_s), 1) if notes_s else None,
        "p95_s": round(percentile(notes_s, 95), 1) if notes_s else None,
        "status": "no_data"
        if not notes_s
        else ("pass" if max(notes_s) <= RT7_NOTES_AFTER_STOP_S else "fail"),
    }
    ratios, longest = [], 0.0
    for run_dir, manifest in final_runs:
        for d, audio_s in _perf_docs(run_dir, manifest):
            if not all(k in d for k in ("asr", "diar", "notes")):
                continue
            audio_s = d["asr"].get("audio_s") or audio_s
            if not audio_s:
                continue
            total = sum(d[k]["wall_s"] for k in ("asr", "diar", "notes"))
            limit = max(RT7_REFINED_FLOOR_S, RT7_REFINED_S_PER_HOUR * audio_s / 3600.0)
            ratios.append(total / limit)
            longest = max(longest, audio_s)
    check_b = {
        "id": "refined_notes",
        "limit_s_per_hour": RT7_REFINED_S_PER_HOUR,
        "files": len(ratios),
        "worst_ratio": round(max(ratios), 3) if ratios else None,
        "longest_audio_min": round(longest / 60.0, 1) if ratios else None,
        "status": "no_data" if not ratios else ("pass" if max(ratios) <= 1.0 else "fail"),
    }
    return [check_a, check_b]


def _slices(files: list[dict]) -> list[dict]:
    keys = sorted(
        {(f["lang"], f["setting"], f["speakers"]) for f in files},
        key=lambda k: (LANG_ORDER.index(k[0]), SETTING_ORDER.index(k[1]), BUCKET_ORDER.index(k[2])),
    )
    return [
        {
            "lang": lang, "setting": setting, "speakers": bucket,
            **_round(aggregate([f for f in files if (f["lang"], f["setting"], f["speakers"]) == (lang, setting, bucket)])),
        }
        for lang, setting, bucket in keys
    ]  # fmt: skip


def _pooled_run(runs: list[dict], pass_: str, realtime: bool) -> dict:
    """One synthetic run document for gate evaluation over several runs."""
    unsupported: dict[str, str] = {}
    tasks: list[str] = []
    for r in runs:
        unsupported.update(r.get("unsupported") or {})
        tasks += [t for t in r["tasks"] if t not in tasks]
    return {"pass": pass_, "realtime": realtime, "tasks": tasks, "unsupported": unsupported}


def _merge_index(manifests: list[Manifest]) -> LintIndex:
    merged = LintIndex()
    names: set[tuple[str, ...]] = set()
    for m in manifests:
        idx = build_index(m)
        merged.ref_ngrams |= idx.ref_ngrams
        merged.note_ngrams |= idx.note_ngrams
        merged.ids += idx.ids
        names.update(idx.names)
    merged.names = sorted(names)
    return merged


def _verdict(
    counts: dict[str, int], failed: bool, errors: int, unsupported: bool, allow_partial: bool
) -> str:
    """fail > incomplete > pass_partial / pass_best_effort > pass.

    A gate without data (`n/a`) leaves the verdict `incomplete`, unless `allow_partial` says the
    datasets are not meant to cover every slice (public sets): then a clean run is `pass_partial`.
    """
    if failed:
        return "fail"
    if counts["incomplete"] or errors or unsupported:
        return "incomplete"
    if counts["n/a"] == sum(counts.values()):  # nothing was evaluated at all
        return "incomplete"
    if counts["n/a"]:
        return "pass_partial" if allow_partial else "incomplete"
    return "pass_best_effort" if counts["best_effort"] else "pass"


def build_acceptance(
    pairs: list[tuple[Path, Path, Manifest]],
    gates_path: Path | None = None,
    external: dict[str, str] | None = None,
    allow_partial: bool = False,
) -> dict:
    """The aggregate report (without its `privacy_lint` block) from finished run directories.

    `pairs`: (final run dir, live run dir, manifest) per dataset; both runs scored.
    """
    external = external or {}
    final_docs = [load_run(f) for f, _, _ in pairs]
    live_docs = [load_run(lv) for _, lv, _ in pairs]
    final_files = [f for fd, _, _ in pairs for f in load_scores(fd)["files"].values()]
    live_files = [f for _, lv, _ in pairs for f in load_scores(lv)["files"].values()]
    final_run = _pooled_run(final_docs, "final", False)
    live_run = _pooled_run(live_docs, "live", True)

    gates = load_gates(gates_path)
    from_final = {g["id"]: g for g in evaluate_gates(final_files, gates, final_run)}
    from_live = {g["id"]: g for g in evaluate_gates(live_files, gates, live_run)}
    gate_rows = []
    for g in gates:
        lag = g["metric"] in LAG_METRICS
        row = dict((from_live if lag else from_final)[g["id"]])
        row["run"] = "live" if lag else "final"
        gate_rows.append(row)

    timing = timing_checks([(lv, m) for _, lv, m in pairs], [(f, m) for f, _, m in pairs])
    ext = [{"id": i, "status": external.get(i, "not_run")} for i in EXTERNAL_IDS]

    counts = {s: sum(g["status"] == s for g in gate_rows) for s in
              ("pass", "best_effort", "fail", "incomplete", "n/a")}  # fmt: skip
    errors = sum(len(d.get("errors") or []) for d in final_docs + live_docs)
    unsupported = sorted(set(final_run["unsupported"]) | set(live_run["unsupported"]))
    failed = (
        counts["fail"]
        or any(t["status"] == "fail" for t in timing)
        or any(e["status"] == "fail" for e in ext)
    )
    verdict = _verdict(counts, bool(failed), errors, bool(unsupported), allow_partial)

    first = final_docs[0]
    return {
        "schema": "ghi.acceptance-report/1",
        "generated": utc_now().isoformat().replace("+00:00", "Z"),
        "kit_version": __version__,
        "system": {
            "spec": public_spec(first["system"]["spec"]),
            "version": first["system"]["version"],
        },
        "hardware": _hardware(),
        "datasets": [
            {
                "name": m.name,
                "files": len(m.files),
                "hours": round(sum((m.duration(e) or 0.0) for e in m.files) / 3600.0, 2),
            }
            for _, _, m in pairs
        ],  # fmt: skip
        "runs": {
            "final": {
                "tasks": final_run["tasks"],
                "errors": sum(len(d.get("errors") or []) for d in final_docs),
            },
            "live": {
                "tasks": live_run["tasks"],
                "errors": sum(len(d.get("errors") or []) for d in live_docs),
            },
            "unsupported_tasks": unsupported,
        },  # fmt: skip
        "gates": gate_rows,
        "timing": timing,
        "external": ext,
        "overall_final": _round(aggregate(final_files)),
        "overall_live": _round(aggregate(live_files)),
        "slices_final": _slices(final_files),
        "summary": {
            "pass": counts["pass"],
            "best_effort": counts["best_effort"],
            "fail": counts["fail"],
            "incomplete": counts["incomplete"],
            "not_evaluated": counts["n/a"],
            "run_errors": errors,
        },
        "verdict": verdict,
    }


def render_markdown(report: dict) -> str:
    md = ["# Ghira acceptance report", ""]
    md.append(f"- system: {report['system']['spec']} {report['system']['version']}")
    ds = report["datasets"]
    md.append(
        "- dataset: " + "; ".join(f"{d['name']} {d['files']} files {d['hours']} h" for d in ds)
    )
    hw = report["hardware"]
    md.append(f"- hardware: {hw['chip']}, {hw['ram_gb']} GB, {hw['os']}")
    md.append(f"- generated: {report['generated']}, kit {report['kit_version']}")
    md.append(f"- verdict: {report['verdict']}")
    s = report["summary"]
    md.append(
        f"- gates: pass {s['pass']}, best_effort {s['best_effort']}, fail {s['fail']}, "
        f"incomplete {s['incomplete']}, not_evaluated {s['not_evaluated']}; "
        f"run errors {s['run_errors']}"
    )
    if report["runs"]["unsupported_tasks"]:
        md.append("- unsupported tasks: " + ", ".join(report["runs"]["unsupported_tasks"]))
    md += ["", "## Gates", ""]
    md += _table(
        ["id", "metric", "slice", "max", "floor", "value", "status", "run"],
        [
            [g["id"], g["metric"], ",".join(f"{k}={v}" for k, v in g["slice"].items()) or "all",
             g["max"], g["floor"], g["value"], g["status"], g["run"]]
            for g in report["gates"]
        ],
    )  # fmt: skip
    md += ["", "## Timing (RT-7)", ""]
    md += _table(
        ["id", "files", "limit", "worst", "status"],
        [
            [t["id"], t["files"],
             f"{t['limit_s']} s" if "limit_s" in t else f"{t['limit_s_per_hour']} s per h",
             f"max {t['max_s']} s" if "max_s" in t else f"{t['worst_ratio']} x limit", t["status"]]
            for t in report["timing"]
        ],
    )  # fmt: skip
    md += ["", "## Checks outside the datasets", ""]
    md += _table(["id", "status"], [[e["id"], e["status"]] for e in report["external"]])
    cols = ["files", "hours", *METRIC_KEYS]
    md += ["", "## Overall", ""]
    md += _table(
        ["run", *cols],
        [["final", *[report["overall_final"][c] for c in cols]],
         ["live", *[report["overall_live"][c] for c in cols]]],
    )  # fmt: skip
    return "\n".join(md) + "\n"


def write_acceptance(report: dict, manifests: list[Manifest], out: Path) -> tuple[Path, Path]:
    """Lint, then write `acceptance-<date>.json` and `.md` into `out`. Nothing is written on a leak."""
    index = _merge_index(manifests)
    lint = lint_document(report, index)
    report["privacy_lint"] = {"passed": True, "ngram": 3, "names_checked": lint.names_checked}
    md = render_markdown(report)
    findings = lint.findings + lint_text(md, index).findings
    if findings:
        raise PrivacyLeak(findings)
    date = datetime.strptime(report["generated"], "%Y-%m-%dT%H:%M:%SZ")
    out = Path(out)
    json_path, md_path = (
        out / f"acceptance-{date:%Y%m%d}.json",
        out / f"acceptance-{date:%Y%m%d}.md",
    )
    write_json(json_path, report)
    md_path.write_text(md, encoding="utf-8", newline="\n")
    return json_path, md_path


def parse_external(values: list[str]) -> dict[str, str]:
    out: dict[str, str] = {}
    for v in values:
        key, _, status = v.partition("=")
        if key not in EXTERNAL_IDS or status not in EXTERNAL_STATUSES:
            raise HarnessError(
                f"bad --external {v!r}: use ID=pass|fail with ID in {', '.join(EXTERNAL_IDS)}"
            )
        out[key] = status
    return out


def run_acceptance(
    datasets: list[Path],
    system: str,
    *,
    gates_path: Path | None = None,
    out: Path | None = None,
    stamp: str | None = None,
    lang: str = "auto",
    timeout: float | None = None,
    collar: float = 0.25,
    external: dict[str, str] | None = None,
    allow_partial: bool = False,
    execute_runs: bool = True,
    log: Callable[[str], None] | None = None,
) -> tuple[dict, Path, Path]:
    """Run (or re-aggregate) the suite. Returns the report, and the two files written."""
    import sys

    log = log or (lambda m: print(m, file=sys.stderr))
    stamp = stamp or f"{utc_now():%Y%m%d-%H%M%S}"
    pairs: list[tuple[Path, Path, Manifest]] = []
    for ds in datasets:
        ds = Path(ds).resolve()
        manifest = load_manifest(ds)
        runs = {}
        for kind, tasks, pass_, realtime in (
            ("final", FINAL_TASKS, "final", False),
            ("live", LIVE_TASKS, "live", True),
        ):
            run_id = f"acc-{stamp}-{kind}"
            if execute_runs:
                log(f"== {manifest.name}: {kind} pass{' (realtime)' if realtime else ''}")
                run_dir = execute(
                    ds, system, tasks=tasks, pass_=pass_, realtime=realtime, lang=lang,
                    run_id=run_id, collar=collar, timeout=timeout, log=log,
                )  # fmt: skip
            else:
                run_dir = ds / "runs" / run_id
            score_run(run_dir)
            try:
                write_report(run_dir, gates_path, ds)  # the per-run report, linted like any other
            except PrivacyLeak as exc:
                for f in exc.findings:
                    log(f"  {f['kind']}: {f['detail']!r}")
                raise
            runs[kind] = run_dir
        pairs.append((runs["final"], runs["live"], manifest))
    report = build_acceptance(pairs, gates_path, external, allow_partial)
    out = Path(out) if out else Path(datasets[0]).resolve() / "runs"
    out.mkdir(parents=True, exist_ok=True)
    json_path, md_path = write_acceptance(report, [m for _, _, m in pairs], out)
    return report, json_path, md_path
