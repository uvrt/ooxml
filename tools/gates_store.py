#!/usr/bin/env python3
"""What a store of the gates' reference cache holds, per mode: its coverage, and the merge
that keeps it (tools/gates.py ``cache-merge`` and ``matrix --store``).

A store (crates/ooxml-oracle/python/reference_cache.py) is a folder of ``*.jsonl.gz`` files
of docx-agent's kept results, each under a key that says nothing of the gate, mode or
document it was computed for.  So a store also carries a **coverage** file (COVERAGE, beside
the results): per gate and mode, per cases-per-tool (``per_tool``; it changes a document's
case list), each document's case keys as the mode's last complete run of it listed them, and
per tree the documents it held (the corpus: the plan job does not make it, so it plans from
the documents the last run found).

* A ``diff`` of an edit mode whose harness finished writes its shard's coverage
  (COVERAGE_NEW: the documents it ran, each with its case keys; ``coverage``); a shard that
  crashed writes none, so its documents stay uncovered.
* ``merge`` adds a run's results and coverage to the store it restored: a document's keys
  replace the ones recorded for it, and a kept result whose key some document listed and no
  document lists any more (its case left the case list) is dropped -- the store keeps every
  mode it held, whatever modes the run ran, and does not grow with cases that are gone.
* ``warm_modes``: the modes whose every planned document the store covers.  The plan
  (tools/gates_jobs.py) gives the others COLD_SHARDS shards: a store made by a run of a few
  modes leaves the rest cold, not warm in one shard.

A covered document's newer cases (a case maker that adds some) are computed when the run
meets them; they are few, and a warm shard has room for them.
"""

from __future__ import annotations

import gzip
import json
from pathlib import Path

#: The store's coverage, beside its results (not ``*.jsonl.gz``: reference_cache.Store reads
#: those as results).
COVERAGE = "coverage.json.gz"
#: A ``diff`` shard's coverage, in its ``--out`` beside reference-new.jsonl.gz.
COVERAGE_NEW = "reference-coverage.json"
#: The name the merged results' file has (reference_cache.MERGED).
MERGED = "reference.jsonl.gz"
VERSION = 1


def read_results(path: Path):
    with gzip.open(path, "rt", encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                entry = json.loads(line)
                yield entry["key"], entry["value"]


def write_results(path: Path, entries: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with gzip.open(path, "wt", encoding="utf-8", compresslevel=6) as handle:
        for key, value in entries.items():
            handle.write(json.dumps({"key": key, "value": value}) + "\n")


def empty() -> dict:
    return {"version": VERSION, "trees": {}, "modes": {}}


def load(folder: str | Path) -> dict | None:
    """The store's coverage, or None when it has none (an older store's, or no store)."""
    path = Path(folder) / COVERAGE
    if not path.is_file():
        return None
    with gzip.open(path, "rt", encoding="utf-8") as handle:
        found = json.load(handle)
    return found if found.get("version") == VERSION else None


def _mode(gate: str, mode: str) -> str:
    return f"{gate}/{mode}"


def _per_tool(per_tool) -> str:
    return f"per_tool={int(per_tool or 0)}"


def coverage(gate: str, mode: str, per_tool, documents: dict[str, list[str]], tree: list[str] | None) -> dict:
    """One shard's coverage (COVERAGE_NEW): ``documents`` its documents (relative to the
    tree), each with its case keys; ``tree`` every document of the tree, or None."""
    return {"gate": gate, "mode": mode, "per_tool": _per_tool(per_tool), "documents": documents,
            **({"tree": tree} if tree is not None else {})}


def _listed(covered: dict) -> set[str]:
    return {key for selections in covered["modes"].values() for docs in selections.values()
            for keys in docs.values() for key in keys}


def merge(store: str | Path, results: list[str | Path], coverages: list[str | Path]) -> dict[str, int]:
    """Adds ``results`` (reference-new.jsonl.gz files) and ``coverages`` (COVERAGE_NEW files)
    to the store in ``store``: one results file (MERGED) and its COVERAGE.  The counts of
    results added and dropped and of documents recorded."""
    folder = Path(store)
    entries: dict[str, dict] = {}
    for path in sorted(folder.glob("*.jsonl.gz")) if folder.is_dir() else ():
        entries.update(read_results(path))
    before = set(entries)
    for path in results:
        entries.update(read_results(Path(path)))
    covered = load(folder) or empty()
    listed_before = _listed(covered)
    recorded = 0
    for path in coverages:
        shard = json.loads(Path(path).read_text())
        docs = covered["modes"].setdefault(_mode(shard["gate"], shard["mode"]), {}).setdefault(shard["per_tool"], {})
        for name, keys in shard["documents"].items():
            if docs.get(name) != keys:
                docs[name] = keys
                recorded += 1
        if "tree" in shard and covered["trees"].get(shard["gate"]) != shard["tree"]:
            covered["trees"][shard["gate"]] = shard["tree"]
            recorded += 1
    # a result some document listed and none lists now: its case is no longer made
    gone = (listed_before - _listed(covered)) & set(entries)
    for key in gone:
        del entries[key]
    for path in folder.glob("*.jsonl.gz") if folder.is_dir() else ():
        path.unlink()
    write_results(folder / MERGED, entries)
    with gzip.open(folder / COVERAGE, "wt", encoding="utf-8", compresslevel=6) as handle:
        json.dump(covered, handle, sort_keys=True)
    return {"added": len(set(entries) - before), "dropped": len(gone), "recorded": recorded}


def tree(covered: dict | None, gate: str) -> list[str] | None:
    """The documents the last run found in ``gate``'s tree (the corpus), or None."""
    return (covered or {}).get("trees", {}).get(gate)


def warm_modes(covered: dict | None, gate: str, modes: list[str], per_tool, documents: list[str] | None) -> set[str]:
    """The ``modes`` of ``gate`` whose every one of ``documents`` (relative to the tree) the
    store covers at ``per_tool``; none when the documents are unknown or the store has no
    coverage."""
    if covered is None or documents is None:
        return set()
    warm = set()
    for mode in modes:
        docs = covered["modes"].get(_mode(gate, mode), {}).get(_per_tool(per_tool), {})
        if docs and all(name in docs for name in documents):
            warm.add(mode)
    return warm
