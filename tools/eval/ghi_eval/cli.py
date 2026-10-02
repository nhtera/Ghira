# SPDX-License-Identifier: Apache-2.0
"""`ghi-eval` command line."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from . import __version__
from .errors import HarnessError, PrivacyLeak
from .manifest import format_validation, load_manifest, validate

EPILOG = "Privacy: reports hold aggregates only; run directories hold hypothesis text and stay on the dataset volume."


def _eprint(*args) -> None:
    print(*args, file=sys.stderr)


def cmd_validate(args) -> int:
    manifest = load_manifest(args.dataset)
    errors, warnings, stats = validate(manifest)
    print(format_validation(manifest, errors, warnings, stats))
    return 1 if errors else 0


def _finish(run_dir: Path, gates: Path | None, dataset: Path | None) -> int:
    """score + report (+ lint). Shared by `run`, `score` and `report`."""
    from .report import write_report

    try:
        json_path, md_path, warnings = write_report(run_dir, gates, dataset)
    except PrivacyLeak as exc:
        _eprint(f"error: {exc}")
        for f in exc.findings:  # the leaked text goes to stderr only, never into a file
            _eprint(f"  {f['kind']}: {f['detail']!r}")
        return 1
    for w in warnings:
        _eprint(f"warning: {w}")
    print(f"report: {json_path}")
    print(f"report: {md_path}")
    return 0


def cmd_run(args) -> int:
    from .run import NOTES_SOURCES, TASKS, execute, parse_list
    from .score import score_run

    tasks = (
        parse_list(args.tasks, TASKS, "--tasks")
        if args.tasks
        else (["asr", "diar", "notes"] + (["stream"] if args.realtime else []))
    )
    notes_input = parse_list(args.notes_input, NOTES_SOURCES, "--notes-input")
    run_dir = execute(
        args.dataset, args.system, tasks=tasks, pass_=args.pass_, realtime=args.realtime,
        lang=args.lang, notes_input=notes_input, run_id=args.run_id, out=args.out,
        collar=args.collar,
        timeout=args.timeout,
    )  # fmt: skip
    print(f"run: {run_dir}")
    score_run(run_dir)
    code = _finish(run_dir, args.gates, None)
    from .run import load_run

    return code or (1 if load_run(run_dir)["errors"] else 0)


def cmd_acceptance(args) -> int:
    from .acceptance import parse_external, run_acceptance

    try:
        report, json_path, md_path = run_acceptance(
            args.dataset, args.system, gates_path=args.gates, out=args.out, stamp=args.stamp,
            lang=args.lang, timeout=args.timeout, collar=args.collar,
            external=parse_external(args.external), allow_partial=args.partial,
            execute_runs=not args.aggregate_only,
        )  # fmt: skip
    except PrivacyLeak as exc:
        _eprint(f"error: {exc}")
        return 1
    print(f"acceptance: {json_path}")
    print(f"acceptance: {md_path}")
    print(f"verdict: {report['verdict']}")
    return 0 if report["verdict"] in ("pass", "pass_best_effort", "pass_partial") else 1


def cmd_score(args) -> int:
    from .score import score_run

    score_run(args.run, args.dataset, args.collar)
    print(f"scores: {Path(args.run) / 'scores.json'}")
    return 0


def cmd_report(args) -> int:
    from .score import score_run

    score_run(args.run, args.dataset, args.collar)
    return _finish(args.run, args.gates, args.dataset)


def cmd_judge(args) -> int:
    from .llm import build_judgements, write_judgements
    from .run import load_run

    run_dir = Path(args.run)
    run = load_run(run_dir)
    manifest = load_manifest(Path(args.dataset or run["dataset"]))
    path = run_dir / "judgements.csv"
    if path.exists() and not args.force:
        raise HarnessError(f"{path} exists; pass --force to overwrite (this loses the judgements)")
    rows = build_judgements(run_dir, manifest)
    write_judgements(path, rows)
    print(f"judgements: {path} ({len(rows)} rows)")
    if not rows:
        _eprint("warning: no notes outputs with reference notes; nothing to judge")
    return 0


def cmd_lint_report(args) -> int:
    from .privacy import lint_file

    result = lint_file(args.report, load_manifest(args.dataset))
    if result.passed:
        print(f"privacy lint: ok ({result.names_checked} names checked)")
        return 0
    _eprint(
        f"error: privacy lint failed ({', '.join(sorted({f['kind'] for f in result.findings}))})"
    )
    for f in result.findings:
        _eprint(f"  {f['kind']}: {f['detail']!r}")
    return 1


def cmd_trials(args) -> int:
    from .speakerid import make_trials, write_trials

    manifest = load_manifest(args.dataset)
    trials = make_trials(manifest)
    out = args.out or Path(args.dataset) / "trials.tsv"
    write_trials(out, trials)
    targets = sum(t[4] == "target" for t in trials)
    print(f"trials: {out} ({len(trials)} trials, {targets} target)")
    if not targets:
        _eprint("warning: no target trials; label the same person with the same id in `persons`")
    return 0


def _check_id(value: str) -> str:
    from .manifest import ID_RE

    if not ID_RE.match(value):
        raise HarnessError(f"id '{value}' must match [A-Za-z0-9_-]+ (pass --id)")
    return value


def cmd_convert(args) -> int:
    from . import convert

    if args.what == "eaf":
        ds = Path(args.dataset) if args.dataset else None
        fid = _check_id(args.id or args.eaf.stem)
        rttm = args.rttm or (ds / "labels" / f"{fid}.rttm" if ds else None)
        ref = args.ref or (ds / "refs" / f"{fid}.txt" if ds else None)
        if rttm is None or ref is None:
            raise HarnessError("convert eaf: give --dataset, or both --rttm and --ref")
        turns, speakers = convert.eaf_to_files(args.eaf, fid, rttm, ref)
        print(f"eaf: {turns} turns, {speakers} speakers -> {rttm}, {ref}")
    elif args.what == "audacity":
        ds = Path(args.dataset) if args.dataset else None
        fid = _check_id(args.id or args.labels.stem)
        rttm = args.rttm or (ds / "labels" / f"{fid}.rttm" if ds else None)
        if rttm is None:
            raise HarnessError("convert audacity: give --dataset or --rttm")
        from .rttm import write_rttm

        turns = convert.audacity_to_turns(args.labels)
        write_rttm(rttm, fid, turns)
        print(f"audacity: {len(turns)} turns -> {rttm}")
    elif args.what == "rttm-audacity":
        out = args.out or args.rttm.with_suffix(".labels.txt")
        n = convert.rttm_to_audacity(args.rttm, out)
        print(f"rttm-audacity: {n} labels -> {out}")
    elif args.what == "draft-eaf":
        from .run import load_run

        run = load_run(args.run)
        manifest = load_manifest(Path(args.dataset or run["dataset"]))
        out_dir = args.out or Path(args.run) / "draft-eaf"
        written = convert.draft_eaf(Path(args.run), manifest, out_dir)
        print(f"draft-eaf: {len(written)} files -> {out_dir}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="ghi-eval", description="Ghira eval kit.", epilog=EPILOG)
    p.add_argument("--version", action="version", version=f"ghi-eval {__version__}")
    sub = p.add_subparsers(dest="command", required=True)

    def add(name: str, fn, help_: str) -> argparse.ArgumentParser:
        sp = sub.add_parser(name, help=help_, description=help_)
        sp.set_defaults(fn=fn)
        return sp

    sp = add("validate", cmd_validate, "check manifest and files, print hours per slice")
    sp.add_argument("--dataset", type=Path, required=True)

    sp = add("run", cmd_run, "run a system on every file, then score, report and lint")
    sp.add_argument("--dataset", type=Path, required=True)
    sp.add_argument(
        "--system",
        required=True,
        help="ghi[:path] | files:<dir> | nemo-ref[:cfg] | whisper-ref[:model]",
    )
    sp.add_argument(
        "--tasks",
        help="comma list of asr,diar,stream,notes (default asr,diar,notes; +stream with --realtime)",
    )
    sp.add_argument("--pass", dest="pass_", choices=["live", "final"], default="final")
    sp.add_argument(
        "--realtime", action="store_true", help="feed audio at 1x and measure caption lag"
    )
    sp.add_argument("--lang", choices=["auto", "vi", "en"], default="auto")
    sp.add_argument("--notes-input", default="pipeline", help="pipeline, gold or pipeline,gold")
    sp.add_argument("--run-id")
    sp.add_argument(
        "--out", type=Path, help="parent directory of the run dir (default <dataset>/runs)"
    )
    sp.add_argument("--gates", type=Path)
    sp.add_argument(
        "--timeout", type=float, help="seconds per task and file (default max(600, 3 x audio))"
    )
    sp.add_argument(
        "--collar", type=float, default=0.25, help="DER collar in seconds (+- width); default 0.25"
    )

    sp = add(
        "acceptance",
        cmd_acceptance,
        "the doc 02 section 1 acceptance suite: final + live runs, gates, RT-7 timing, one report",
    )
    sp.add_argument("--dataset", type=Path, action="append", required=True,
                    help="repeat for several datasets (their files are pooled)")  # fmt: skip
    sp.add_argument("--system", required=True, help="ghi[:path] | files:<dir> | ...")
    sp.add_argument(
        "--out", type=Path, help="where the aggregate report goes (default <dataset>/runs)"
    )
    sp.add_argument("--gates", type=Path)
    sp.add_argument("--lang", choices=["auto", "vi", "en"], default="auto")
    sp.add_argument("--timeout", type=float, help="seconds per task and file")
    sp.add_argument("--collar", type=float, default=0.25)
    sp.add_argument("--stamp", help="run id suffix (default: the current time)")
    sp.add_argument(
        "--partial", action="store_true",
        help="datasets need not cover every gate (public sets): gates without data give pass_partial, not incomplete",
    )  # fmt: skip
    sp.add_argument(
        "--aggregate-only", action="store_true",
        help="do not run; aggregate the runs of --stamp that already exist",
    )  # fmt: skip
    sp.add_argument(
        "--external", action="append", default=[], metavar="ID=pass|fail",
        help="record a result measured elsewhere: crash_safety, strict_offline_audit, vn_search",
    )  # fmt: skip

    sp = add("score", cmd_score, "recompute scores.json for a run")
    sp.add_argument("--run", type=Path, required=True)
    sp.add_argument("--dataset", type=Path, help="override the dataset recorded in run.yaml")
    sp.add_argument(
        "--collar",
        type=float,
        default=None,
        help="DER collar in seconds (+- width); default: the run's value",
    )

    sp = add("report", cmd_report, "rebuild the report (e.g. after judgements.csv is filled in)")
    sp.add_argument("--run", type=Path, required=True)
    sp.add_argument("--dataset", type=Path)
    sp.add_argument(
        "--collar",
        type=float,
        default=None,
        help="DER collar in seconds (+- width); default: the run's value",
    )
    sp.add_argument("--gates", type=Path)

    sp = add("judge", cmd_judge, "write judgements.csv with suggested matches")
    sp.add_argument("--run", type=Path, required=True)
    sp.add_argument("--dataset", type=Path)
    sp.add_argument("--force", action="store_true", help="overwrite an existing judgements.csv")

    sp = add("lint-report", cmd_lint_report, "privacy lint of a report file")
    sp.add_argument("--report", type=Path, required=True)
    sp.add_argument("--dataset", type=Path, required=True)

    sp = add("trials", cmd_trials, "write speaker-ID trials from manifest persons")
    sp.add_argument("--dataset", type=Path, required=True)
    sp.add_argument("--out", type=Path)

    conv = sub.add_parser("convert", help="label converters")
    csub = conv.add_subparsers(dest="what", required=True)

    def cadd(name: str, help_: str) -> argparse.ArgumentParser:
        sp = csub.add_parser(name, help=help_, description=help_)
        sp.set_defaults(fn=cmd_convert)
        return sp

    sp = cadd("eaf", "ELAN .eaf -> labels/<id>.rttm + refs/<id>.txt")
    sp.add_argument("eaf", type=Path)
    sp.add_argument("--id", help="manifest id (default: file stem)")
    sp.add_argument("--dataset", type=Path)
    sp.add_argument("--rttm", type=Path)
    sp.add_argument("--ref", type=Path)

    sp = cadd("audacity", "Audacity label track (start, end, speaker) -> RTTM")
    sp.add_argument("labels", type=Path)
    sp.add_argument("--id")
    sp.add_argument("--dataset", type=Path)
    sp.add_argument("--rttm", type=Path)

    sp = cadd("rttm-audacity", "RTTM -> Audacity label track")
    sp.add_argument("rttm", type=Path)
    sp.add_argument("--out", type=Path)

    sp = cadd("draft-eaf", "make <id>.eaf per file from a run, for correction in ELAN")
    sp.add_argument("--run", type=Path, required=True)
    sp.add_argument("--dataset", type=Path)
    sp.add_argument("--out", type=Path, help="default <run>/draft-eaf")
    return p


def main(argv: list[str] | None = None) -> int:
    for stream in (sys.stdout, sys.stderr):  # legacy Windows consoles must not crash on Vietnamese
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(errors="backslashreplace")
    args = build_parser().parse_args(argv)
    try:
        return args.fn(args)
    except HarnessError as exc:
        _eprint(f"error: {exc}")
        return 1
    except KeyboardInterrupt:
        _eprint("interrupted")
        return 130


if __name__ == "__main__":
    sys.exit(main())
