#!/usr/bin/env python3
"""The differential gates on goldens (goldens.py): a mode's frozen cases on some documents run
through the Rust oracle binary, each answer's digest compared with the case's golden.  No
docx-agent, no reference cache: stdlib only.

    python crates/ooxml-oracle/python/golden_gate.py --gate fixtures --mode structure \\
        --rust target/release/word-edits --results out/results.jsonl generated/controls.docx ...

Each case is one of:

* equal: its answer's digest is its golden's;
* refused as not supported yet where its golden is too: tools/gates.py judges the form
  against the allow-list, as before goldens;
* a difference: another digest, an answer where the golden was refused (the form is served
  now: bless it, docx-agent's answer is in goldens/pending), a refusal where the golden had an
  answer, no answer, or no golden at all (a case not blessed yet).

The results file is the harnesses' (word_reads_diff.py's ``report``): a line per case with a
difference, then the totals; ``--record`` writes each such case's answer as goldens.py's
record, for ``tools/gates.py bless``.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import goldens  # noqa: E402
from oracle_run import RUST_TALLY, refused, run_lines  # noqa: E402

#: The modes whose cases are tool calls: --per-tool samples them per tool.
TOOL_MODES = ("structure", "objects", "style")
#: The read modes (word-reads, word-ids): a case is a manifest item.
READ_MODES = ("ids", "describe", "read", "ranges", "inspect", "find", "batch", "check", "facts", "laid")


def manifest_of(mode: str, tree: Path, frozen: list[tuple[str, dict]]) -> tuple[list[dict], dict[tuple, tuple]]:
    """The Rust oracle's manifest for ``frozen`` (each a document's name and a case line) and
    each answer's key to (document, case line).  A case's ``input`` is the file the oracle
    runs it on; an edit mode's cases are grouped per file in their order."""
    manifest: list[dict] = []
    keys: dict[tuple, tuple] = {}
    entries: dict[str, dict] = {}
    for document, line in frozen:
        case = goldens.local(line["case"])
        file = case.pop("input", None) or str(tree / document)
        if mode in READ_MODES:
            manifest.append({"file": file, **case})
            keys[(file, case.get("case"))] = (document, line)
        else:
            entry = entries.get(file)
            if entry is None:
                entry = entries[file] = {"file": file, "cases": []}
                manifest.append(entry)
            keys[(file, len(entry["cases"]))] = (document, line)
            entry["cases"].append(case)
    return manifest, keys


def judge(line: dict | None, golden: dict | None) -> tuple[str | None, str | None]:
    """A case's difference (None: none) and the form Rust refused it in, if it did."""
    form = refused(line)
    if line is None:
        return "no rust result", None
    if golden is None:
        return "no golden: a case not blessed yet (tools/gates.py bless)", form
    if form:
        return (None if golden.get("not_supported") else
                f"refused as not supported yet ({form}) where the golden has an answer ({golden['source']})"), form
    if golden.get("not_supported"):
        return (f"served now; the golden was refused ({golden['not_supported']}): bless it against docx-agent's "
                "answer in goldens/pending"), None
    found = goldens.digest(line)
    return (None if found == golden["digest"] else
            f"the answer's digest {found[:23]} is not the golden's {golden['digest'][:23]} ({golden['source']})"), None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--gate", required=True, choices=sorted(goldens.TREE_OF_GATE))
    parser.add_argument("--mode", required=True)
    parser.add_argument("--rust", required=True, help="the oracle binary (word-edits, word-reads or word-ids)")
    parser.add_argument("--tree", required=True, help="the folder of the gate's documents")
    parser.add_argument("--results", required=True)
    parser.add_argument("--record", help="the differing cases' answers, as goldens.py's record (for bless)")
    parser.add_argument("--per-tool", type=int, default=0)
    parser.add_argument("--made", action="store_true", help="also the cases on made-up documents (_made)")
    parser.add_argument("documents", nargs="*", help="names below the tree")
    args = parser.parse_args(argv)
    tree_name = goldens.TREE_OF_GATE[args.gate]
    names = [*args.documents, *([goldens.MADE] if args.made else [])]
    frozen: list[tuple[str, dict]] = []
    golden: dict[tuple[str, str], dict] = {}     # by document and case id
    for name in names:
        cases = goldens.sample(goldens.cases_of(tree_name, name, args.mode), args.per_tool, args.mode in TOOL_MODES)
        frozen += [(name, line) for line in cases]
        golden.update(((name, i), line) for i, line in goldens.goldens_of(tree_name, name).items())
    tree = Path(args.tree)
    manifest, keys = manifest_of(args.mode, tree, frozen)
    answers = run_lines(args.rust, manifest) if manifest else {}
    lines, records, known = [], [], 0
    for name in args.documents:
        if not goldens.document_file(goldens.CASES, tree_name, name).is_file():
            lines.append({"file": str(tree / name), "case": None, "input": None,
                          "differences": ["no frozen cases: a document not frozen yet (tools/gates.py bless)"]})
    for key, (name, case) in keys.items():
        line = answers.get(key)
        blessed = golden.get((name, case["id"]))
        known += bool(blessed and blessed.get("source", "").startswith("known difference"))
        difference, form = judge(line, blessed)
        if difference is None:
            continue
        lines.append({"file": str(tree / name), "case": case["id"], "input": case["case"], "differences": [difference]})
        if line is not None:
            records.append({"tree": tree_name, "document": name, "mode": args.mode, "id": case["id"],
                            "case": case["case"], **({"not_supported": form} if form else
                                                     {"digest": goldens.digest(line), "output": goldens.normalised(line)})})
    for found in lines[:50]:
        print(f"{found['file']} [{found['case']}]: {found['differences'][0]}")
    print(f"{len(keys)} cases, {len(lines)} with differences")
    totals = {"cases": len(keys), "failed": len(lines), "panics": RUST_TALLY["panics"],
              "not_supported_forms": dict(RUST_TALLY["not_supported_forms"].most_common()), "golden_known": known}
    Path(args.results).write_text("".join(json.dumps(x) + "\n" for x in [*lines, {"totals": totals}]))
    if args.record:
        goldens.write_lines(Path(args.record), records)
    return 1 if lines else 0


if __name__ == "__main__":
    sys.exit(main())
