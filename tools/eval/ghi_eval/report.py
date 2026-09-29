# SPDX-License-Identifier: Apache-2.0
"""Aggregate scores into slices and gates, lint, and write the report (formats.md §6)."""

from __future__ import annotations

from datetime import datetime
from pathlib import Path

import yaml

from . import __version__
from .contract import validation_errors
from .errors import HarnessError, PrivacyLeak
from .llm import llm_summary, read_judgements
from .manifest import Manifest, load_manifest
from .metrics import asr_metric_for, percentile
from .privacy import build_index, lint_document, lint_text
from .run import load_run, utc_now, write_json
from .score import load_scores, score_run

DEFAULT_GATES = Path(__file__).parent / "gates.yaml"
LANG_ORDER = ["vi", "en", "mixed"]
SETTING_ORDER = ["room", "call", "other"]
BUCKET_ORDER = ["1-2", "3-5", "6+"]
METRIC_KEYS = [
    "der", "jer", "spk_count_err", "wer", "syl_wer", "mer",
    "partial_lag_p50", "partial_lag_p95", "final_lag_p50", "final_lag_p95",
    "caption_lag_p50", "caption_lag_p95", "rtf", "peak_rss_mb",
]  # fmt: skip
COVERAGE_KEYS = ["asr_files", "diar_files", "lag_files", "notes_files"]


def _ratio(num: float, den: float) -> float | None:
    return num / den if den else None


def aggregate(files: list[dict]) -> dict:
    """Pooled metrics over per-file score records (sums of components, never means of ratios)."""
    diar = [f["diar"] for f in files if "diar" in f]
    out: dict = {
        "files": len(files),
        "hours": round(sum((f.get("duration_s") or 0.0) for f in files) / 3600.0, 2),
        "der": _ratio(sum(d["missed"] + d["false_alarm"] + d["confusion"] for d in diar),
                      sum(d["ref_speech"] for d in diar)),
        "jer": _ratio(sum(d["jer_error"] for d in diar), sum(d["jer_speakers"] for d in diar)),
        "spk_count_err": sum(d["spk_count_err"] for d in diar) / len(diar) if diar else None,
        # files with both a reference and a valid hypothesis, per metric family
        "asr_files": sum("asr" in f for f in files),
        "diar_files": len(diar),
        "lag_files": sum(bool(f.get("lag", {}).get("caption")) for f in files),
        "notes_files": sum(
            bool(f.get("expects", {}).get("notes")) and any(f.get("notes", {}).values())
            for f in files
        ),
    }  # fmt: skip
    for metric in ("wer", "syl_wer", "mer"):
        asr = [f["asr"] for f in files if "asr" in f and f["asr"]["metric"] == metric]
        out[metric] = _ratio(sum(a["errors"] for a in asr), sum(a["ref_tokens"] for a in asr))
    for kind in ("partial", "final", "caption"):
        lags = [x for f in files for x in f.get("lag", {}).get(kind, [])]
        out[f"{kind}_lag_p50"] = percentile(lags, 50)
        out[f"{kind}_lag_p95"] = percentile(lags, 95)
    perf = [f["perf"] for f in files if f.get("perf") and f["perf"].get("audio_s")]
    out["rtf"] = _ratio(sum(p["wall_s"] for p in perf), sum(p["audio_s"] for p in perf))
    rss = [p["peak_rss_mb"] for p in perf if p.get("peak_rss_mb") is not None]
    out["peak_rss_mb"] = max(rss) if rss else None
    return out


def _round(metrics: dict) -> dict:
    return {
        k: (round(v, 1) if k == "peak_rss_mb" else round(v, 4)) if isinstance(v, float) else v
        for k, v in metrics.items()
    }


def load_gates(path: Path | None) -> list[dict]:
    doc = yaml.safe_load(Path(path or DEFAULT_GATES).read_text(encoding="utf-8")) or {}
    gates = doc.get("gates")
    if not isinstance(gates, list):
        raise HarnessError("gates file: expected a `gates:` list")
    for g in gates:
        missing = {"id", "metric", "slice", "max"} - set(g)
        if missing or g["metric"] not in METRIC_KEYS:
            raise HarnessError(f"gates file: bad gate {g.get('id', '?')}")
    return gates


def gate_status(value: float | None, gate: dict) -> str:
    if value is None:
        return "n/a"
    if value <= gate["max"]:
        return "pass"
    if gate.get("floor") is not None and value <= gate["floor"]:
        return "best_effort"
    return "fail"


WORD_METRICS = ("wer", "syl_wer", "mer")
LAG_METRICS = (
    "partial_lag_p50", "partial_lag_p95", "final_lag_p50", "final_lag_p95",
    "caption_lag_p50", "caption_lag_p95",
)  # fmt: skip


def _coverage(subset: list[dict], metric: str, run: dict) -> tuple[int, int] | None:
    """(covered, expected) files of `subset` for `metric`; None for metrics without references."""
    if metric in ("der", "jer", "spk_count_err"):
        return sum("diar" in f for f in subset), sum(f["expects"]["diar"] for f in subset)
    if metric in WORD_METRICS:
        mine = [f for f in subset if asr_metric_for(f["lang"]) == metric]
        return sum("asr" in f for f in mine), sum(f["expects"]["asr"] for f in mine)
    if metric in LAG_METRICS:
        ran = (
            run.get("realtime")
            and "stream" in run["tasks"]
            and "stream" not in run.get("unsupported", {})
        )
        expected = len(subset) if ran else 0
        key = metric.split("_", 1)[0]  # partial, final or caption
        return sum(bool(f.get("lag", {}).get(key)) for f in subset), expected
    return None


def evaluate_gates(files: list[dict], gates: list[dict], run: dict) -> list[dict]:
    """Gate values pooled over the matching files.

    Word-error gates are n/a on live-pass runs and lag gates on non-realtime runs. A gate
    is `incomplete` when some matching file has a reference but no valid hypothesis.
    """
    out = []
    for g in gates:
        sel = g.get("slice") or {}
        metric = g["metric"]
        subset = [f for f in files if all(f.get(k) == v for k, v in sel.items())]
        value = aggregate(subset)[metric] if subset else None
        value = None if value is None else round(value, 4)
        cov = _coverage(subset, metric, run)
        if (metric in WORD_METRICS and run["pass"] == "live") or (
            metric in LAG_METRICS and not run.get("realtime")
        ):
            value, status = None, "n/a"
        elif cov and cov[0] < cov[1] and (cov[0] > 0 or metric in LAG_METRICS):
            # some file has a reference (or, for lag, was streamed) but no valid hypothesis
            status = "incomplete"
        else:
            status = gate_status(value, g)
        out.append(
            {
                "id": g["id"], "metric": metric, "slice": dict(sel), "max": g["max"],
                "floor": g.get("floor"), "value": value, "status": status,
            }
        )  # fmt: skip
    return out


def public_spec(spec: str) -> str:
    """The system spec without paths (they can hold user or customer names)."""
    kind, _, arg = spec.partition(":")
    kind = kind.replace("_", "-")
    if kind == "files":
        return "files"
    if arg:
        return f"{kind}:{Path(arg.replace(chr(92), '/')).name}"
    return kind


def build_report(
    run_dir: Path, manifest: Manifest, gates_path: Path | None = None
) -> tuple[dict, list[str]]:
    """The report without its `privacy_lint` block, plus warnings for the console."""
    run = load_run(run_dir)
    scores = load_scores(run_dir)
    files = list(scores["files"].values())
    slices = []
    keys = sorted(
        {(f["lang"], f["setting"], f["speakers"]) for f in files},
        key=lambda k: (LANG_ORDER.index(k[0]), SETTING_ORDER.index(k[1]), BUCKET_ORDER.index(k[2])),
    )
    for lang, setting, bucket in keys:
        sel = [
            f for f in files if (f["lang"], f["setting"], f["speakers"]) == (lang, setting, bucket)
        ]
        slices.append(
            {"lang": lang, "setting": setting, "speakers": bucket, **_round(aggregate(sel))}
        )
    overall = aggregate(files)

    warnings: list[str] = []
    judgements = Path(run_dir) / "judgements.csv"
    rows = read_judgements(judgements) if judgements.is_file() else None
    notes_valid: dict[str, dict[str, bool]] = {}
    for fid, rec in scores["files"].items():
        for source, ok in rec.get("notes", {}).items():
            notes_valid.setdefault(source, {})[fid] = ok
    llm, llm_warnings = llm_summary(notes_valid, rows)
    warnings += llm_warnings

    sid = scores.get("speaker_id")
    report = {
        "schema": "ghi.eval-report/1",
        "generated": utc_now().isoformat().replace("+00:00", "Z"),
        "kit_version": __version__,
        "system": {"spec": public_spec(run["system"]["spec"]), "version": run["system"]["version"]},
        "dataset": {
            "name": manifest.name,
            "files": len(files),
            "hours": overall["hours"],
        },
        "run": {
            "pass": run["pass"],
            "realtime": bool(run["realtime"]),
            "lang": run["lang"],
            "tasks": run["tasks"],
            "errors": len(run.get("errors") or []),
        },  # fmt: skip
        "der_collar": scores.get("der_collar", 0.25),
        "slices": slices,
        "overall": _round(overall),
        "speaker_id": None
        if not sid
        else {
            "eer": None if sid["eer"] is None else round(sid["eer"], 4),
            "trials": sid["trials"],
        },  # fmt: skip
        "llm": [
            {k: (round(v, 4) if isinstance(v, float) else v) for k, v in e.items()} for e in llm
        ],
        "gates": evaluate_gates(files, load_gates(gates_path), run),
    }
    return report, warnings


def _fmt(v) -> str:
    return "-" if v is None else str(v)


def _table(headers: list[str], rows: list[list]) -> list[str]:
    lines = ["| " + " | ".join(headers) + " |", "|" + "|".join("---" for _ in headers) + "|"]
    lines += ["| " + " | ".join(_fmt(c) for c in row) + " |" for row in rows]
    return lines


def render_markdown(report: dict) -> str:
    """The same numbers as the JSON, as tables with minimal prose."""
    metric_cols = ["files", "hours", *METRIC_KEYS, *COVERAGE_KEYS]
    md = ["# Ghira eval report", ""]
    md.append(f"- system: {report['system']['spec']} {report['system']['version']}")
    d = report["dataset"]
    md.append(f"- dataset: {d['name']}, {d['files']} files, {d['hours']} h")
    r = report["run"]
    md.append(
        f"- run: pass {r['pass']}, realtime {str(r['realtime']).lower()}, lang {r['lang']}, "
        f"errors {r['errors']}"
    )
    md.append(f"- der_collar: {report['der_collar']}")
    md.append(f"- generated: {report['generated']}, kit {report['kit_version']}")
    md += ["", "## Slices", ""]
    rows = [
        [s["lang"], s["setting"], s["speakers"], *[s[c] for c in metric_cols]]
        for s in report["slices"]
    ]
    rows.append(["all", "all", "all", *[report["overall"][c] for c in metric_cols]])
    md += _table(["lang", "setting", "spk", *metric_cols], rows)
    md += ["", "## Gates", ""]
    md += _table(
        ["id", "metric", "slice", "max", "floor", "value", "status"],
        [
            [g["id"], g["metric"], ",".join(f"{k}={v}" for k, v in g["slice"].items()) or "all",
             g["max"], g["floor"], g["value"], g["status"]]
            for g in report["gates"]
        ],
    )  # fmt: skip
    if report["speaker_id"]:
        md += ["", "## Speaker ID", ""]
        md += _table(
            ["eer", "trials"], [[report["speaker_id"]["eer"], report["speaker_id"]["trials"]]]
        )
    if report["llm"]:
        md += ["", "## Notes", ""]
        cols = ["source", "files", "schema_valid", "precision", "recall", "owner_acc",
                "citation_valid", "hallucinations"]  # fmt: skip
        md += _table(cols, [[e[c] for c in cols] for e in report["llm"]])
    p = report["privacy_lint"]
    md += ["", "## Privacy", ""]
    md += _table(
        ["passed", "ngram", "names_checked"],
        [[str(p["passed"]).lower(), p["ngram"], p["names_checked"]]],
    )
    return "\n".join(md) + "\n"


def write_report(
    run_dir: Path, gates_path: Path | None = None, dataset: Path | None = None
) -> tuple[Path, Path, list[str]]:
    """Build, lint and write report-<system>-<pass>-<date>.{json,md}. Nothing is written if the lint fails."""
    run_dir = Path(run_dir)
    run = load_run(run_dir)
    manifest = load_manifest(Path(dataset or run["dataset"]))
    if not (run_dir / "scores.json").is_file():
        score_run(run_dir, dataset)
    report, warnings = build_report(run_dir, manifest, gates_path)

    index = build_index(manifest)
    lint = lint_document(report, index)
    report["privacy_lint"] = {"passed": True, "ngram": 3, "names_checked": lint.names_checked}
    md = render_markdown(report)
    md_lint = lint_text(md, index)
    findings = lint.findings + md_lint.findings
    if findings:
        raise PrivacyLeak(findings)
    problems = validation_errors(report, "eval-report")
    if problems:
        raise HarnessError(f"internal error: report does not match its schema ({problems[0]})")

    date = datetime.strptime(report["generated"], "%Y-%m-%dT%H:%M:%SZ")
    stem = f"report-{run['system']['name']}-{run['pass']}-{date:%Y%m%d}"
    json_path, md_path = run_dir / f"{stem}.json", run_dir / f"{stem}.md"
    write_json(json_path, report)
    md_path.write_text(md, encoding="utf-8", newline="\n")
    return json_path, md_path, warnings
