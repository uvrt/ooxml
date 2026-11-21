#!/usr/bin/env python3
"""The heavy gates' runner for .github/workflows/gates.yml (on demand, never on push).

The differentials run docx-agent (pure Python: ``DOCX_AGENT_RUST=0``) against the Rust core's
oracle binaries on the same documents, through crates/ooxml-oracle/python's harnesses, one
shard of the documents at a time:

    python tools/gates.py corpus OUT                      # docx2svg's tools/make_*.py, each in OUT/<name>
    python tools/gates.py report OUT 6 61                 # the 3k document: make_report 6, grown to 61 chapters
    python tools/gates.py diff --gate fixtures --mode styled --shard 2/4 --bin DIR --out DIR
    python tools/gates.py matrix --gates fixtures,corpus --warm  # the differential jobs (JSON; tools/gates_jobs.py)
    python tools/gates.py plan [--changed FILE]...        # the modes and fixtures a branch can affect (auto)
    python tools/gates.py fixtures --fixtures auto        # the fixtures a branch runs (all, or a comma list)
    python tools/gates.py summary DIR                     # the job summary's tables (Markdown)
    python tools/gates.py cache-key                       # the reference cache's chain digest
    python tools/gates.py cache-merge STORE FILE...       # kept results added to a store
    python tools/gates.py durations DIR [--write]         # a run's warm times per mode (tools/gates_durations.json)

The reference cache (crates/ooxml-oracle/python/reference_cache.py): with ``--cache STORE``
a ``diff`` of an edit mode uses docx-agent's results kept in STORE where their key (the code
the reference runs, its environment, the document and the case) still holds, runs docx-agent
for the rest and writes those to ``--out``'s reference-new.jsonl.gz; ``cache-merge`` adds
them to the store.  Verdicts are the same as without it: a kept result whose comparison
finds a difference is computed again in full.

A ``diff`` writes the harness's log, its cases with differences (JSON lines) and a summary
(JSON) into ``--out``.  It fails when a case differs and no entry of the known differences
(crates/ooxml-oracle/known-differences.toml, each with its cause) covers it.

Modes ``auto`` (the dispatch's default): the edit modes, and whether the reads' checks run,
picked from the files the branch changes against origin/main (``git diff origin/main...``):
a path counts for the modes it can change and no others (``gate_effects``), conservatively --
the Rust code and docx-agent's count for every mode, a harness's case maker for its own
modes, notes and the packages and crates the gates do not run for none, and anything not
named for every mode.  No change at all (the ref is main) is every mode too.

Fixtures ``auto`` (``--fixtures``; the dispatch's default for a branch): the fixtures and
reads gates run only the fixture documents holding the Word features the branch's changes
touch, plus a fixed set of canaries (tools/gates_fixtures.py); a change to a foundation (the
XML writer, the package, the store, the session, the layout, the harness) or to a path naming
no feature runs them all.  ``all`` runs every fixture (the nightly run on main).

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
import zipfile
from functools import lru_cache
from pathlib import Path
from xml.etree import ElementTree

import gates_fixtures
import gates_hunks
import gates_jobs

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
              "inspect": "word-reads", "find": "word-reads", "batch": "word-reads",
              "check": "word-reads", "facts": "word-reads", "laid": "word-reads"}
#: The tool modes' sampling knobs (word_*_cases.py).
#: The corpus's cases per tool and document when the run gives none (a full corpus run of
#: every case is hours of docx-agent).
CORPUS_PER_TOOL = 10
PER_TOOL = {"structure": "STRUCTURE_PER_TOOL", "objects": "OBJECTS_PER_TOOL", "style": "STYLE_PER_TOOL"}

#: What a changed path can change in the gates, for modes ``auto`` (``gate_effects``).  ALL is
#: every edit mode and the reads.
READS = "reads"
ALL = frozenset([*EDIT_MODES, READS])
#: The harness's case makers: each the modes whose cases it makes (word_object_cases.py and
#: word_style_cases.py take their helpers from word_structure_cases.py).
CASE_MAKERS = {"word_structure_cases.py": {"structure", "objects", "style"}, "word_object_cases.py": {"objects"},
               "word_style_cases.py": {"style"}, "word_text_cases.py": {"text"}}
#: The rest of the harness the gates run: the edits harness (it imports word_reads_diff.py).
GATE_HARNESS = ("word_edits_", "word_tool_reference", "word_reads_", "reference_cache")
#: The oracle binaries the gates run, and what each answers.
GATE_BINARIES = {"word-edits": set(EDIT_MODES), "word-reads": {READS}, "word-ids": {READS}}
#: The packages the gates install (gates.yml: the reference and what makes the corpus).
GATE_PACKAGES = ("docx-agent", "docx2svg", "ooxml-edit", "ooxml-common")
#: The files the gates themselves are: they change every mode.
GATE_FILES = ("tools/gates.py", ".github/workflows/gates.yml", ".github/actions/", "Cargo.toml", "Cargo.lock",
              ".cargo/", "rust-toolchain", "crates/ooxml-oracle/known-differences.toml")
#: Paths no gate reads: notes and the other workflows, tools and documentation.
NOT_GATES = (".github/", "tools/", "docs/")
#: A crate's folders not built into its library or binaries.
CRATE_TEST_FOLDERS = ("tests", "benches", "examples")


def documents(tree: Path, every: int, shard: tuple[int, int], only: list[str] | None = None) -> list[Path]:
    """Shard K of N of every ``every``-th document below ``tree`` (sorted, as the harnesses
    read a tree); ``only``: those of these names (relative to ``tree``) alone."""
    files = [f for f in sorted(tree.rglob("*.docx")) if not f.name.startswith("~$")]
    if only is not None:
        files = [f for f in files if str(f.relative_to(tree)) in set(only)]
    files = files[::max(every, 1)]
    k, n = shard
    return [f for i, f in enumerate(files) if i % n == k - 1]


def known_entries(path: Path = KNOWN) -> list[dict]:
    entries = tomllib.loads(path.read_text()).get("difference", [])
    for entry in entries:
        if not entry.get("cause") or not entry.get("mode"):
            raise SystemExit(f"{path}: every known difference needs a mode and a cause: {entry}")
        if "case" in entry:
            # a case's number changes with the run's sampling and with each case maker added
            raise SystemExit(f"{path}: a known difference names its case by input, not by number: {entry}")
        condition = entry.get("document")
        if condition is not None and (not isinstance(condition, dict) or "lacks_namespace" not in condition
                                      or set(condition) - DOCUMENT_KEYS):
            raise SystemExit(f"{path}: a document condition takes lacks_namespace (and part): {entry}")
    return entries


def _contains(have, want) -> bool:
    """``want`` is ``have`` or, for objects, a subset of it (recursively)."""
    if isinstance(want, dict):
        return isinstance(have, dict) and all(k in have and _contains(have[k], v) for k, v in want.items())
    return have == want


#: The keys a known difference's ``document`` condition takes.
DOCUMENT_KEYS = {"part", "lacks_namespace"}


@lru_cache(maxsize=256)
def root_namespaces(source: str, part: str) -> frozenset[str] | None:
    """The namespaces the root element of ``part`` in the package ``source`` declares
    (``None``: no such package or part)."""
    try:
        with zipfile.ZipFile(source) as package, package.open(part) as xml:
            declared = set()
            for event, item in ElementTree.iterparse(xml, events=("start-ns", "start")):
                if event == "start":
                    return frozenset(declared)
                declared.add(item[1])
    except (OSError, KeyError, zipfile.BadZipFile, ElementTree.ParseError):
        return None
    return None


def _document_holds(condition: dict, source: str | None) -> bool:
    """Whether the case's document meets an entry's ``document`` condition: the root of its
    ``part`` (word/document.xml when not given) does not declare ``lacks_namespace``."""
    if source is None:
        return False
    declared = root_namespaces(source, condition.get("part", "word/document.xml"))
    return declared is not None and condition["lacks_namespace"] not in declared


def covering(entries: list[dict], gate: str, mode: str, relative: str, case, source: str | None = None) -> dict | None:
    """The known difference that covers a case, if one does (``source``: the case's document,
    for an entry's ``document`` condition)."""
    for entry in entries:
        if entry["mode"] != mode or entry.get("gate", gate) != gate:
            continue
        if not fnmatch.fnmatch(relative, entry.get("file", "*")):
            continue
        if "input" in entry and not _contains(case.get("input"), entry["input"]):
            continue
        if "document" in entry and not _document_holds(entry["document"], source):
            continue
        return entry
    return None


def harness_command(args, out: Path, results: Path, files: list[Path]) -> tuple[list[str], dict[str, str]]:
    """The harness run for ``diff``'s mode, and the variables it adds to the environment.

    The reads harness takes the documents right after its check. It has two positionals
    (the check, then the files); an older Python's argparse (3.12.5, for one) fills both at
    the first positional and refuses files that come after the options.
    """
    if args.mode in READ_MODES:
        return [sys.executable, str(HARNESS / "word_reads_diff.py"), args.mode, *map(str, files),
                "--rust", str(Path(args.bin) / READ_MODES[args.mode]), "--results", str(results)], {}
    env: dict[str, str] = {}
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
    return [*command, "--results", str(results), *map(str, files)], env


def run_diff(args) -> int:
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    tree = Path(args.tree) if args.tree else TREES[args.gate]
    k, n = (int(x) for x in args.shard.split("/"))
    only = None
    if args.gate in TREES and not args.tree:
        chosen = fixture_selection(args.fixtures, args.base)
        only = None if chosen == gates_fixtures.ALL else chosen
    files = documents(tree, args.every, (k, n), only)
    results = out / "results.jsonl"
    command, extra = harness_command(args, out, results, files)
    env = {**os.environ, "DOCX_AGENT_RUST": "0", "PYTHONWARNINGS": "ignore", **extra}
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
        entry = covering(entries, args.gate, args.mode, relative, case, source)
        (known if entry else unknown).append({"file": relative, "case": case["case"],
                                              "cause": entry and entry["cause"],
                                              **({"errors": case["errors"]} if case.get("errors") else {})})
    crashed = done is not None and done.returncode not in (0, 1) or (files and not totals)
    summary = {"gate": args.gate, "mode": args.mode, "shard": args.shard, "every": args.every,
               "per_tool": args.per_tool, "documents": len(files),
               "cases": totals.get("cases", 0), "differences": len(unknown), "known": len(known),
               "unported": totals.get("unported", 0), "wall": round(wall, 1), "crashed": bool(crashed),
               **({"kept": totals["kept"], "computed": totals["computed"]} if "kept" in totals else {}),
               "unknown": unknown[:50]}
    (out / "summary.json").write_text(json.dumps(summary, indent=1))
    print(json.dumps({k: v for k, v in summary.items() if k != "unknown"}))
    for case in unknown[:50]:
        errors = f"; errors {json.dumps(case['errors'])[:300]}" if case.get("errors") else ""
        print(f"::error::{args.gate}/{args.mode}: {case['file']} [{case['case']}] differs (log.txt has the paths)"
              f"{errors}")
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


def oracle_crates(root: Path = ROOT) -> set[str]:
    """The folders below crates/ the oracle binaries are built from: ooxml-oracle and every
    workspace crate it depends on (dependencies and build-dependencies, read from Cargo.toml)."""
    folders = {}
    for manifest in sorted((root / "crates").glob("*/Cargo.toml")):
        folders[tomllib.loads(manifest.read_text())["package"]["name"]] = manifest.parent.name
    found: set[str] = set()
    todo = ["ooxml-oracle"]
    while todo:
        folder = todo.pop()
        if folder in found:
            continue
        found.add(folder)
        manifest = tomllib.loads((root / "crates" / folder / "Cargo.toml").read_text())
        tables = [manifest.get("dependencies", {}), manifest.get("build-dependencies", {})]
        tables += [t.get(k, {}) for t in manifest.get("target", {}).values() for k in ("dependencies",
                                                                                      "build-dependencies")]
        todo += [folders[name] for table in tables for name in table if name in folders]
    return found


def gate_effects(path: str, crates: set[str]) -> set[str]:
    """The edit modes (and READS) a change to ``path`` can change; ``crates``: oracle_crates()."""
    parts = path.split("/")
    if path.startswith(GATE_FILES):
        return set(ALL)
    if path.startswith("crates/ooxml-oracle/python/"):
        if parts[-1] in CASE_MAKERS:
            return set(CASE_MAKERS[parts[-1]])
        return set(ALL) if parts[-1].startswith(GATE_HARNESS) else set()
    if parts[0] == "crates" and len(parts) > 2:
        if parts[1] not in crates or parts[2] in CRATE_TEST_FOLDERS or path.endswith(".md") and parts[2] != "src":
            return set()
        if path.startswith("crates/ooxml-oracle/src/bin/"):
            return set(GATE_BINARIES.get(parts[-1].removesuffix(".rs"), ()))
        return set(ALL)
    if parts[0] == "python" and len(parts) > 2:
        if parts[1] not in GATE_PACKAGES:
            return set()
        inside = "/".join(parts[2:])
        if inside.startswith(("src/", "tests/fixtures/")) or inside == "pyproject.toml" \
                or parts[1] == "docx2svg" and inside.startswith("tools/make_"):
            return set(ALL)
        return set()
    if path.startswith(NOT_GATES) or len(parts) == 1 and path.endswith((".md", ".txt")) or path.startswith("."):
        return set()
    return set(ALL)


def changed_files(base: str = "origin/main") -> list[str]:
    """The files the checkout changes against ``base`` (since their merge base)."""
    done = subprocess.run(["git", "diff", "--name-only", f"{base}...HEAD"], cwd=ROOT, capture_output=True,
                          text=True, check=True)
    return [line for line in done.stdout.splitlines() if line]


def auto_modes(changed: list[str]) -> tuple[list[str], bool]:
    """The edit modes (in EDIT_MODES' order) and whether the reads run, for a branch that
    changes ``changed``; no change at all is every mode."""
    if not changed:
        return list(EDIT_MODES), True
    crates = oracle_crates()
    effects = set().union(*(gate_effects(path, crates) for path in changed))
    return [m for m in EDIT_MODES if m in effects], READS in effects


def auto_fixtures(changed: list[str], diff=None) -> list[str] | str:
    """The fixtures a branch changing ``changed`` runs (tools/gates_fixtures.py), or ALL; no
    change at all is every fixture.  ``diff``: the changed lines (gates_hunks.Diff)."""
    if not changed:
        return gates_fixtures.ALL
    crates = oracle_crates()
    return gates_fixtures.select([path for path in changed if gate_effects(path, crates)], diff=diff)


def fixture_selection(value: str, base: str = "origin/main") -> list[str] | str:
    """``--fixtures``: ``all``, ``auto`` (auto_fixtures of the changes since ``base``) or a
    comma list of names relative to the fixtures folder."""
    if value in ("", gates_fixtures.ALL):
        return gates_fixtures.ALL
    if value == "auto":
        return auto_fixtures(changed_files(base), gates_hunks.Diff(base))
    return [name.strip() for name in value.split(",") if name.strip()]


def run_plan(args) -> int:
    changed = args.changed if args.changed is not None else changed_files(args.base)
    modes, reads = auto_modes(changed)
    chosen = auto_fixtures(changed, None if args.changed is not None else gates_hunks.Diff(args.base))
    fixtures = "all" if chosen == gates_fixtures.ALL else f"{len(chosen)}: {','.join(chosen)}"
    print(f"{len(changed)} changed files; modes: {','.join(modes) or '(none)'}; reads: {'yes' if reads else 'no'}; "
          f"fixtures: {fixtures}")
    return 0


def run_fixtures(args) -> int:
    """The fixtures selection as ``diff --fixtures`` takes it: ``all`` or a comma list."""
    chosen = fixture_selection(args.fixtures, args.base)
    print(gates_fixtures.ALL if chosen == gates_fixtures.ALL else ",".join(chosen))
    return 0


def default_per_tool(gate: str) -> int:
    """A gate's cases per tool when the run gives none (0: all)."""
    return CORPUS_PER_TOOL if gate == "corpus" else 0


def planned_documents(args) -> int:
    """The documents one mode of the fixtures and reads gates runs: the fixtures picked
    (``--fixtures``), every ``--every``-th."""
    chosen = fixture_selection(args.fixtures, args.base)
    return len(documents(FIXTURES, args.every, (1, 1), None if chosen == gates_fixtures.ALL else chosen))


def run_matrix(args) -> int:
    """The differential jobs' matrix (JSON) for the gates asked for: each gate's modes in
    shards and bundles (tools/gates_jobs.py: ``--warm`` when the reference cache holds a
    store for this checkout, ``--shards N`` to fix the shards); the corpus's default
    sampling where none is given."""
    gates = [g.strip() for g in args.gates.split(",") if g.strip()]
    reads = True
    if args.modes == "auto":
        modes, reads = auto_modes(changed_files(args.base))
        print(f"modes auto: {','.join(modes) or '(none)'}; reads: {'yes' if reads else 'no'}", file=sys.stderr)
    elif args.modes in ("", "all"):
        modes = list(EDIT_MODES)
    else:
        modes = [m.strip() for m in args.modes.split(",")]
    unknown = [m for m in modes if m not in EDIT_MODES]
    if unknown:
        raise SystemExit(f"unknown modes {unknown}; the modes: {', '.join(EDIT_MODES)}")
    shards = int(args.shards) if args.shards not in ("", "auto") else None
    durations = gates_jobs.load_durations()
    fixtures = planned_documents(args) if shards is None and any(g in TREES for g in gates) else None
    include = []
    for gate in gates:
        if gate not in ("fixtures", "corpus", "reads") or gate == "reads" and not reads:
            continue
        per_tool = args.per_tool if args.per_tool != "" else str(default_per_tool(gate))
        gate_modes = list(READ_MODES if gate == "reads" else modes)
        count = fixtures if gate in TREES else None     # the corpus: as recorded
        estimates = {mode: gates_jobs.estimate(durations, gate, mode, count) for mode in gate_modes}
        jobs = gates_jobs.gate_jobs(gate, gate_modes, warm=args.warm, shards=shards, estimates=estimates)
        include += [{**job, "per_tool": per_tool} for job in jobs]
    if len(include) > 256:
        raise SystemExit(f"{len(include)} jobs: GitHub's matrix takes 256; fewer shards or modes")
    shape = f"shards {shards}" if shards else "warm" if args.warm else "cold"
    print(f"{len(include)} differential jobs ({shape})", file=sys.stderr)
    print(json.dumps({"include": include}) if include else "")
    return 0


def run_durations(args) -> int:
    """A run's warm times per mode (from its downloaded ``diff-*`` logs) as JSON; with
    ``--write`` merged into tools/gates_durations.json."""
    summaries = [json.loads(p.read_text()) for p in sorted(Path(args.dir).rglob("summary.json"))]
    found = gates_jobs.warm_durations(summaries, {g: default_per_tool(g) for g in ("fixtures", "corpus", "reads")})
    print(json.dumps(found, indent=1))
    if args.write:
        gates_jobs.write_durations(found)
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
                errors = f": {json.dumps(case['errors'])[:300]}" if case.get("errors") else ""
                out.append(f"- {mode}: `{case['file']}` [{case['case']}]{errors}")
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
    d.add_argument("--fixtures", default="all", help="fixtures and reads: all, auto or a comma list of fixtures")
    d.add_argument("--base", default="origin/main", help="--fixtures auto: the changes since this one's merge base")
    c = sub.add_parser("corpus")
    c.add_argument("out")
    r = sub.add_parser("report")
    r.add_argument("out")
    r.add_argument("base", type=int)
    r.add_argument("chapters", type=int)
    m = sub.add_parser("matrix")
    m.add_argument("--gates", required=True)
    m.add_argument("--modes", default="auto", help="all, auto or a comma list")
    m.add_argument("--base", default="origin/main", help="modes auto: the changes since this one's merge base")
    m.add_argument("--shards", default="auto", help="jobs per mode; auto: tools/gates_jobs.py")
    m.add_argument("--warm", action="store_true", help="the reference cache holds a store for this checkout")
    m.add_argument("--per-tool", default="")
    m.add_argument("--every", type=int, default=1)
    m.add_argument("--fixtures", default="all", help="fixtures and reads: all, auto or a comma list (job sizes)")
    pl = sub.add_parser("plan", help="the modes a branch can affect (modes auto)")
    pl.add_argument("--changed", action="append", help="a changed path (repeat); default: git diff BASE...HEAD")
    pl.add_argument("--base", default="origin/main")
    f = sub.add_parser("fixtures", help="the fixtures a branch runs (all or a comma list)")
    f.add_argument("--fixtures", default="auto")
    f.add_argument("--base", default="origin/main")
    s = sub.add_parser("summary")
    s.add_argument("dir")
    sub.add_parser("cache-key")
    du = sub.add_parser("durations", help="a run's warm times per mode (from its diff-* logs)")
    du.add_argument("dir")
    du.add_argument("--write", action="store_true", help="merge them into tools/gates_durations.json")
    k = sub.add_parser("cache-merge")
    k.add_argument("store")
    k.add_argument("files", nargs="*")
    args = parser.parse_args(argv)
    return {"diff": run_diff, "corpus": run_corpus, "report": run_report, "matrix": run_matrix,
            "summary": run_summary, "plan": run_plan, "fixtures": run_fixtures, "cache-key": run_cache_key, "cache-merge": run_cache_merge,
            "durations": run_durations}[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
