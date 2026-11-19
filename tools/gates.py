#!/usr/bin/env python3
"""The heavy gates' runner for .github/workflows/gates.yml (on demand, never on push).

The differentials run docx-agent (pure Python: ``DOCX_AGENT_RUST=0``) against the Rust core's
oracle binaries on the same documents, through crates/ooxml-oracle/python's harnesses, one
shard of the documents at a time:

    python tools/gates.py corpus OUT                      # docx2svg's tools/make_*.py, each in OUT/<name>
    python tools/gates.py report OUT 6 61                 # the 3k document: make_report 6, grown to 61 chapters
    python tools/gates.py diff --gate fixtures --mode styled --shard 2/4 --bin DIR --out DIR
    python tools/gates.py matrix --gates fixtures,corpus  # the differential jobs (JSON)
    python tools/gates.py summary DIR                     # the job summary's tables (Markdown)
    python tools/gates.py cache-key                       # the reference cache's chain digest
    python tools/gates.py cache-merge STORE FILE...       # kept results added to a store

The reference cache (crates/ooxml-oracle/python/reference_cache.py): with ``--cache STORE``
a ``diff`` of an edit mode uses docx-agent's results kept in STORE where their key (the code
the reference runs, its environment, the document and the case) still holds, runs docx-agent
for the rest and writes those to ``--out``'s reference-new.jsonl.gz; ``cache-merge`` adds
them to the store.  Verdicts are the same as without it: a kept result whose comparison
finds a difference is computed again in full.

A ``diff`` writes the harness's log, its cases with differences (JSON lines) and a summary
(JSON) into ``--out``.  It fails when a case differs and no entry of the known differences
(crates/ooxml-oracle/known-differences.toml, each with its cause) covers it.

Sampling: ``--every N`` takes every N-th document; ``--per-tool N`` the first N cases of each
tool for the tool modes (structure, objects, style: the harnesses' *_PER_TOOL) and the first N
cases per document for the others (the harness's ``--limit``).  0 is all.
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import os
import subprocess
import sys
import time
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HARNESS = ROOT / "crates" / "ooxml-oracle" / "python"
KNOWN = ROOT / "crates" / "ooxml-oracle" / "known-differences.toml"
REPORT = ROOT / "python" / "ooxml-core" / "tests" / "harness"

#: The trees a gate reads by default (the corpus: the folder ``corpus`` made, given as --tree).
FIXTURES = ROOT / "python" / "docx-agent" / "tests" / "fixtures"
TREES = {"fixtures": FIXTURES, "reads": FIXTURES}

#: word_edits_diff.py's modes: each the flags it takes (``set_text`` none: set_text alone).
EDIT_MODES = {
    "set_text": [], "ops": ["--only-ops"], "paragraphs": ["--paragraphs"], "delete": ["--delete"],
    "cells": ["--cells"], "styled": ["--styled"], "markdown": ["--markdown"], "spacing": ["--spacing"],
    "text": ["--text"], "tools": ["--tools"], "review": ["--review"], "structure": ["--structure"],
    "objects": ["--objects"], "style": ["--style"],
}
#: word_reads_diff.py's checks (the ``reads`` gate) and the binary each runs.
READ_MODES = {"ids": "word-ids", "describe": "word-reads", "read": "word-reads", "ranges": "word-reads",
              "inspect": "word-reads", "find": "word-reads", "batch": "word-reads"}
#: The tool modes' sampling knobs (word_*_cases.py).
#: The corpus's cases per tool and document when the run gives none (a full corpus run of
#: every case is hours of docx-agent).
CORPUS_PER_TOOL = 10
PER_TOOL = {"structure": "STRUCTURE_PER_TOOL", "objects": "OBJECTS_PER_TOOL", "style": "STYLE_PER_TOOL"}


def documents(tree: Path, every: int, shard: tuple[int, int]) -> list[Path]:
    """Shard K of N of every ``every``-th document below ``tree`` (sorted, as the harnesses
    read a tree)."""
    files = [f for f in sorted(tree.rglob("*.docx")) if not f.name.startswith("~$")]
    files = files[::max(every, 1)]
    k, n = shard
    return [f for i, f in enumerate(files) if i % n == k - 1]


def known_entries(path: Path = KNOWN) -> list[dict]:
    entries = tomllib.loads(path.read_text()).get("difference", [])
    for entry in entries:
        if not entry.get("cause") or not entry.get("mode"):
            raise SystemExit(f"{path}: every known difference needs a mode and a cause: {entry}")
    return entries


def _contains(have, want) -> bool:
    """``want`` is ``have`` or, for objects, a subset of it (recursively)."""
    if isinstance(want, dict):
        return isinstance(have, dict) and all(k in have and _contains(have[k], v) for k, v in want.items())
    return have == want


def covering(entries: list[dict], gate: str, mode: str, relative: str, case) -> dict | None:
    """The known difference that covers a case, if one does."""
    for entry in entries:
        if entry["mode"] != mode or entry.get("gate", gate) != gate:
            continue
        if not fnmatch.fnmatch(relative, entry.get("file", "*")):
            continue
        if "input" in entry and not _contains(case.get("input"), entry["input"]):
            continue
        if "case" in entry and entry["case"] != case.get("case"):
            continue
        return entry
    return None


def run_diff(args) -> int:
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    tree = Path(args.tree) if args.tree else TREES[args.gate]
    k, n = (int(x) for x in args.shard.split("/"))
    files = documents(tree, args.every, (k, n))
    results = out / "results.jsonl"
    env = {**os.environ, "DOCX_AGENT_RUST": "0", "PYTHONWARNINGS": "ignore"}
    if args.mode in READ_MODES:
        command = [sys.executable, str(HARNESS / "word_reads_diff.py"), args.mode,
                   "--rust", str(Path(args.bin) / READ_MODES[args.mode])]
    else:
        command = [sys.executable, str(HARNESS / "word_edits_diff.py"), *EDIT_MODES[args.mode],
                   "--rust", str(Path(args.bin) / "word-edits")]
        if args.mode == "spacing":
            (out / "french").mkdir(exist_ok=True)    # --spacing's folder for the French copies
            command.insert(command.index("--spacing") + 1, str(out / "french"))
        if args.cache:
            command += ["--cache", args.cache, "--cache-new", str(out / "reference-new.jsonl.gz")]
        if args.per_tool and args.mode in PER_TOOL:
            env[PER_TOOL[args.mode]] = str(args.per_tool)
        elif args.per_tool:
            command += ["--limit", str(args.per_tool)]
    command += ["--results", str(results), *map(str, files)]
    started = time.monotonic()
    with open(out / "log.txt", "w") as log:
        done = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, env=env, cwd=ROOT) if files else None
    wall = time.monotonic() - started
    lines = [json.loads(x) for x in results.read_text().splitlines()] if results.exists() else []
    totals = lines.pop()["totals"] if lines and "totals" in lines[-1] else {}
    entries = known_entries()
    unknown, known = [], []
    for case in lines:
        source = (case.get("input") or {}).get("source") or case["file"]
        relative = os.path.relpath(source, tree)
        entry = covering(entries, args.gate, args.mode, relative, case)
        (known if entry else unknown).append({"file": relative, "case": case["case"],
                                              "cause": entry and entry["cause"]})
    crashed = done is not None and done.returncode not in (0, 1) or (files and not totals)
    summary = {"gate": args.gate, "mode": args.mode, "shard": args.shard, "documents": len(files),
               "cases": totals.get("cases", 0), "differences": len(unknown), "known": len(known),
               "unported": totals.get("unported", 0), "wall": round(wall, 1), "crashed": bool(crashed),
               **({"kept": totals["kept"], "computed": totals["computed"]} if "kept" in totals else {}),
               "unknown": unknown[:50]}
    (out / "summary.json").write_text(json.dumps(summary, indent=1))
    print(json.dumps({k: v for k, v in summary.items() if k != "unknown"}))
    for case in unknown[:50]:
        print(f"::error::{args.gate}/{args.mode}: {case['file']} [{case['case']}] differs (log.txt has the paths)")
    if crashed:
        print(f"::error::{args.gate}/{args.mode} {args.shard}: the harness failed (log.txt)")
    return 1 if unknown or crashed else 0


def run_cache_key(args) -> int:
    """The reference cache's chain digest, as the edits harness computes it."""
    done = subprocess.run([sys.executable, str(HARNESS / "reference_cache.py")], capture_output=True, text=True,
                          check=True, cwd=ROOT, env={**os.environ, "DOCX_AGENT_RUST": "0", "PYTHONWARNINGS": "ignore"})
    print(done.stdout.split()[-1])
    return 0


def run_cache_merge(args) -> int:
    sys.path.insert(0, str(HARNESS))
    from reference_cache import merge

    print(merge(args.store, args.files), "results added")
    return 0


def run_corpus(args) -> int:
    """docx2svg's made-up documents, as the word-model job makes them: each generator run in
    its own folder (they write there by default)."""
    out = Path(args.out)
    for script in sorted((ROOT / "python" / "docx2svg" / "tools").glob("make_*.py")):
        folder = out / script.stem
        folder.mkdir(parents=True, exist_ok=True)
        with open(folder / "log.txt", "w") as log:
            done = subprocess.run([sys.executable, str(script)], cwd=folder, stdout=log, stderr=subprocess.STDOUT)
        if done.returncode:
            print((folder / "log.txt").read_text())
            return 1
    print(len(list(out.rglob("*.docx"))), "documents")
    return 0


def run_report(args) -> int:
    """The long report: make_report.py's ``base`` chapters, grown to ``chapters``."""
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    base = out.with_name(out.stem + "-base.docx")
    for command in ([str(REPORT / "make_report.py"), str(base), str(args.base)],
                    [str(REPORT / "grow_report.py"), str(base), str(out), str(args.chapters)]):
        subprocess.run([sys.executable, *command], check=True, env={**os.environ, "DOCX_AGENT_RUST": "0"})
    return 0


def run_matrix(args) -> int:
    """The differential jobs' matrix (JSON) for the gates asked for: each gate's modes, each
    in ``shards`` shards; the corpus's default sampling where none is given."""
    gates = [g.strip() for g in args.gates.split(",") if g.strip()]
    modes = list(EDIT_MODES) if args.modes in ("", "all") else [m.strip() for m in args.modes.split(",")]
    unknown = [m for m in modes if m not in EDIT_MODES]
    if unknown:
        raise SystemExit(f"unknown modes {unknown}; the modes: {', '.join(EDIT_MODES)}")
    include = []
    for gate in gates:
        if gate not in ("fixtures", "corpus", "reads"):
            continue
        per_tool = args.per_tool if args.per_tool != "" else str(CORPUS_PER_TOOL if gate == "corpus" else 0)
        for mode in (READ_MODES if gate == "reads" else modes):
            for k in range(1, args.shards + 1):
                include.append({"gate": gate, "mode": mode, "shard": f"{k}/{args.shards}", "per_tool": per_tool})
    if len(include) > 256:
        raise SystemExit(f"{len(include)} jobs: GitHub's matrix takes 256; fewer shards or modes")
    print(json.dumps({"include": include}) if include else "")
    return 0


def run_summary(args) -> int:
    """Per gate: each mode's shards added up, as Markdown tables."""
    rows: dict[tuple[str, str], dict] = {}
    for path in sorted(Path(args.dir).rglob("summary.json")):
        s = json.loads(path.read_text())
        row = rows.setdefault((s["gate"], s["mode"]), {"shards": 0, "documents": 0, "cases": 0, "differences": 0,
                                                       "known": 0, "unported": 0, "wall": 0.0, "crashed": 0,
                                                       "unknown": []})
        row["shards"] += 1
        for key in ("kept", "computed"):
            if key in s:
                row[key] = row.get(key, 0) + s[key]
        row["wall"] = max(row["wall"], s["wall"])
        row["crashed"] += s["crashed"]
        row["unknown"] += s["unknown"]
        for key in ("documents", "cases", "differences", "known", "unported"):
            row[key] += s[key]
    out = []
    for gate in dict.fromkeys(g for g, _ in rows):
        out += [f"### {gate}", "", "| mode | shards | documents | cases | differences | known | unported | "
                "slowest shard (s) |", "|---|---:|---:|---:|---:|---:|---:|---:|"]
        for (g, mode), r in rows.items():
            if g == gate:
                flag = " (harness failed)" if r["crashed"] else ""
                out.append(f"| {mode}{flag} | {r['shards']} | {r['documents']} | {r['cases']:,} | "
                           f"{r['differences']} | {r['known']} | {r['unported']} | {r['wall']:.0f} |")
        cached = [f"{mode} {r['kept']:,} of {r['kept'] + r['computed']:,}" for (g, mode), r in rows.items()
                  if g == gate and "kept" in r]
        if cached:
            out += ["", "docx-agent's results kept from earlier runs (reference cache): " + ", ".join(cached)]
        out.append("")
        for (g, mode), r in rows.items():
            for case in r["unknown"][:20] if g == gate else []:
                out.append(f"- {mode}: `{case['file']}` [{case['case']}]")
        out.append("")
    print("\n".join(out))
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    d = sub.add_parser("diff")
    d.add_argument("--gate", required=True, choices=["fixtures", "corpus", "reads"])
    d.add_argument("--mode", required=True, choices=[*EDIT_MODES, *READ_MODES])
    d.add_argument("--tree", help="the documents (fixtures: docx-agent's, from the checkout)")
    d.add_argument("--shard", default="1/1", help="K/N")
    d.add_argument("--every", type=int, default=1)
    d.add_argument("--per-tool", type=int, default=0)
    d.add_argument("--bin", required=True, help="the folder of the release oracle binaries")
    d.add_argument("--out", required=True)
    d.add_argument("--cache", help="the reference cache's store (edit modes; reference_cache.py)")
    c = sub.add_parser("corpus")
    c.add_argument("out")
    r = sub.add_parser("report")
    r.add_argument("out")
    r.add_argument("base", type=int)
    r.add_argument("chapters", type=int)
    m = sub.add_parser("matrix")
    m.add_argument("--gates", required=True)
    m.add_argument("--modes", default="all")
    m.add_argument("--shards", type=int, default=3)
    m.add_argument("--per-tool", default="")
    s = sub.add_parser("summary")
    s.add_argument("dir")
    sub.add_parser("cache-key")
    k = sub.add_parser("cache-merge")
    k.add_argument("store")
    k.add_argument("files", nargs="*")
    args = parser.parse_args(argv)
    return {"diff": run_diff, "corpus": run_corpus, "report": run_report, "matrix": run_matrix,
            "summary": run_summary, "cache-key": run_cache_key, "cache-merge": run_cache_merge}[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
