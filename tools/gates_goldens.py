"""The gates' goldens on tools/gates.py's side (crates/ooxml-oracle/python/goldens.py has the
format): a differential run's record turned into goldens' records, and ``bless``.

A record (``golden-record.jsonl.gz`` in a ``diff --record`` shard's out folder, uploaded with
its ``inputs/``) has a line per case: its tree, document, mode, id and frozen case, then the
Rust oracle's answer (``digest`` and normalised ``output``) or the form it refused as not
supported yet (``not_supported``, with docx-agent's answer as ``python``), and its ``source``:

* recorded against the frozen reference (``diff --record`` with docx-agent): ``frozen Python
  <sha>, unmeasured`` where the answers were equal, ``known difference: <cause>`` where a known
  difference covered the case, none where nothing did (a failing gate: bless refuses it);
* recorded by a gate on goldens (golden_gate.py ``--record``): the cases whose answer differs
  from their golden, or that have none, with no source: ``bless --source`` gives one.

``bless DIR`` reads every record below DIR (a run's downloaded artifacts) and writes the
goldens; a record of the frozen reference also writes each recorded document's case list for
its mode.  It prints the changes by gate, mode and kind and refuses a changed or new answer
without a source.
"""

from __future__ import annotations

import json
import os
import shutil
import sys
import tomllib
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "crates" / "ooxml-oracle" / "python"))

import goldens  # noqa: E402

#: A shard's record and the harness's raw one, in its out folder.
RECORD = "golden-record.jsonl.gz"
RAW = "record-raw.jsonl.gz"
#: What a shard's record was made with (gate, mode, per_tool, every, whether it ran docx-agent).
META = "golden-record.json"
REFERENCE = ROOT / "crates" / "ooxml-oracle" / "reference.toml"


GATE = ROOT / "crates" / "ooxml-oracle" / "python" / "golden_gate.py"


def on_goldens(gate: str) -> bool:
    """Whether ``gate`` runs on goldens: its tree's goldens are in the checkout."""
    return bool(goldens.documents(goldens.TREE_OF_GATE[gate], goldens.GOLDENS))


def golden_command(meta: dict, binary: Path, tree: Path, results: Path, record: Path | None,
                   documents: list[str], made: bool) -> list[str]:
    """golden_gate.py's run of a shard: ``documents`` (names below ``tree``), and the cases on
    made-up documents where ``made`` (the first shard); ``record``: the out folder that gets
    the differing cases' answers (and the metadata bless reads)."""
    command = [sys.executable, str(GATE), "--gate", meta["gate"], "--mode", meta["mode"], "--rust", str(binary),
               "--tree", str(tree), "--results", str(results), "--per-tool", str(meta["per_tool"] or 0)]
    if made:
        command.append("--made")
    if record is not None:
        record.mkdir(parents=True, exist_ok=True)
        (record / META).write_text(json.dumps({**meta, "reference": False}))
        command += ["--record", str(record / RECORD)]
    return [*command, *documents]


def frozen_source() -> str:
    sha = tomllib.loads(REFERENCE.read_text())["sha"]
    return goldens.FROZEN_PYTHON.format(sha=sha[:8])


def _document(file: str | None, tree: Path) -> str:
    return goldens.MADE if file is None else os.path.relpath(file, tree)


def finish(out: Path, meta: dict, tree: Path, cover) -> int:
    """The shard's raw record (the harness's, against docx-agent) as goldens' records:
    ``cover(document, case)`` is the known difference that covers a case, if one does; the
    number of cases nothing covers (they differ: the gate fails)."""
    records, uncovered = [], 0
    source = frozen_source()
    tree_name = goldens.TREE_OF_GATE[meta["gate"]]
    for raw in goldens.read_lines(out / RAW):
        document = _document(raw["file"], tree)
        record = {"tree": tree_name, "document": document, "mode": meta["mode"],
                  "id": goldens.case_id(meta["mode"], raw["case"]), "case": raw["case"]}
        if raw.get("not_supported"):
            record.update(not_supported=raw["not_supported"], python=raw.get("python"), source=source)
        elif raw.get("missing"):
            record.update(source=None)
            uncovered += 1
        else:
            entry = cover(raw["file"], raw["case"]) if raw["differs"] else None
            if raw["differs"] and entry is None:
                uncovered += 1
            record.update(digest=raw["digest"], output=raw["output"],
                          source=None if raw["differs"] and entry is None else
                          goldens.KNOWN.format(cause=entry["cause"].splitlines()[0]) if entry else source)
        records.append(record)
    goldens.write_lines(out / RECORD, records)
    (out / META).write_text(json.dumps({**meta, "reference": True}))
    return uncovered


def _records(folder: Path) -> tuple[list[dict], list[dict]]:
    """Every record below ``folder`` and the metadata of the shards that wrote them."""
    records, metas = [], []
    for path in sorted(folder.rglob(RECORD)):
        records += goldens.read_lines(path)
        meta = path.parent / META
        metas.append(json.loads(meta.read_text()) if meta.is_file() else {})
    return records, metas


def _golden(record: dict, source: str | None) -> dict:
    if record.get("not_supported"):
        return {"id": record["id"], "not_supported": record["not_supported"],
                "source": record.get("source") or source}
    return {"id": record["id"], "digest": record["digest"], "source": record.get("source") or source}


def bless(folder: Path, source: str | None = None, run: str | None = None, write: bool = True,
          default_per_tool=lambda gate: 0) -> tuple[int, list[str]]:
    """Writes the goldens (and, from a record of the frozen reference, the case lists) the
    records below ``folder`` hold; ``source`` for answers whose record has none.  The exit
    status and the summary's lines (refused: nothing written)."""
    records, metas = _records(folder)
    if not records:
        return 1, [f"no {RECORD} below {folder}"]
    freezes = {(m["gate"], m["mode"]) for m in metas
               if m.get("reference") and int(m.get("per_tool") or 0) == default_per_tool(m["gate"])}
    partial = {(m["gate"], m["mode"]) for m in metas if m.get("reference")} - freezes
    by_document: dict[tuple[str, str], list[dict]] = {}
    for record in records:
        by_document.setdefault((record["tree"], record["document"]), []).append(record)
    changes: Counter = Counter()
    refused: list[str] = []
    writes: list[tuple[Path, list[dict]]] = []
    for (tree, document), found in sorted(by_document.items()):
        old_cases = goldens.read_lines(goldens.document_file(goldens.CASES, tree, document))
        old = goldens.goldens_of(tree, document)
        frozen_modes = {r["mode"] for r in found if any((g, r["mode"]) in freezes
                                                        for g, t in goldens.TREE_OF_GATE.items() if t == tree)}
        cases = [c for c in old_cases if c["mode"] not in frozen_modes]
        known_ids = {c["id"] for c in cases}
        new = dict(old)
        for record in found:
            if record["mode"] in frozen_modes and record["id"] not in known_ids:
                cases.append({"id": record["id"], "mode": record["mode"], "case": record["case"]})
                known_ids.add(record["id"])
            golden = _golden(record, source)
            before = old.get(record["id"])
            kind = ("new" if before is None else "unchanged" if before == golden
                    else "served" if before.get("not_supported") and not golden.get("not_supported")
                    else "refused" if golden.get("not_supported") and not before.get("not_supported") else "changed")
            if golden["source"] is None:
                refused.append(f"{tree}/{document} {record['id']}: {kind} without a source (bless --source, or a "
                               "known difference)")
                continue
            new[record["id"]] = golden
            changes[(record["mode"], kind)] += 1
        removed = [i for i in new if i not in {c["id"] for c in cases}] if frozen_modes else []
        for i in removed:
            changes[(i.split(":")[0], "removed")] += 1
            new.pop(i)
        order = {c["id"]: n for n, c in enumerate(cases)}
        if frozen_modes:
            writes.append((goldens.document_file(goldens.CASES, tree, document), cases))
        writes.append((goldens.document_file(goldens.GOLDENS, tree, document),
                       sorted(new.values(), key=lambda g: order.get(g["id"], len(order)))))
        pending_file = goldens.document_file(goldens.PENDING, tree, document, ".jsonl.gz")
        recorded = {r["id"] for r in found}
        pending = [p for p in goldens.read_lines(pending_file)
                   if p["id"] not in recorded and p["id"].split(":")[0] not in frozen_modes]
        pending += [{"id": r["id"], "python": r["python"]} for r in found
                    if r.get("not_supported") and r.get("python") is not None]
        writes.append((pending_file, pending))
    lines = summary(changes, refused, partial, run)
    if refused or not write:
        return (1 if refused else 0), lines
    for path, content in writes:
        goldens.write_lines(path, content)
    for path in sorted(folder.rglob("inputs/*.docx")):
        target = goldens.INPUTS / path.name
        if not target.exists():
            goldens.INPUTS.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
    return 0, lines


def summary(changes: Counter, refused: list[str], partial: set, run: str | None) -> list[str]:
    """The bless's changes by mode and kind (Markdown lines)."""
    kinds = ("new", "changed", "served", "refused", "removed", "unchanged")
    modes = sorted({mode for mode, _ in changes})
    out = ["| mode | " + " | ".join(kinds) + " |", "|---|" + "---:|" * len(kinds)]
    out += [f"| {mode} | " + " | ".join(str(changes[(mode, k)]) for k in kinds) + " |" for mode in modes]
    if run:
        out += ["", f"Full outputs (the normalised answers): the record artifacts of {run}."]
    if partial:
        out += ["", "Sampled reference records (per_tool below the freeze's): goldens only, case lists kept: "
                + ", ".join(f"{g}/{m}" for g, m in sorted(partial))]
    if refused:
        out += ["", f"Refused: {len(refused)} answers without a source; nothing written.", *refused[:40]]
    return out
