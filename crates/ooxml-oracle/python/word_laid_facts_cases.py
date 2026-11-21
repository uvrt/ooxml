"""``check``'s layout facts read from the pages, docx-agent against Rust (test support;
lane P5f): ``word_reads_diff.py laid``.

For each document, docx-agent's ``layout_facts`` of its layout -- the facts the Rust tools
give so far (:data:`KEYS`, ``settings``' :data:`SETTINGS`), capped as ``check`` lists them
-- once without a minimum text size and once with each of :data:`MINIMUMS`, against the
``word-reads`` binary's laid fact cases (``crates/ooxml-oracle/src/laid_facts.rs``: a page
engine laying the document out with docx-agent's fonts, ``fonts_of``).

Python's behaviour is not changed: this only calls docx-agent's documented functions.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

TESTS = Path(__file__).resolve().parents[3] / "python" / "docx-agent" / "tests"
if str(TESTS) not in sys.path:
    sys.path.insert(0, str(TESTS))

from docx_agent import Document
from docx_agent.layout_facts import layout_facts
from docx_agent.scope import Scope

#: The facts compared, with their ``_left_out`` counts.
KEYS = ("smallest_text", "small_text", "spacers", "keep_with_next_at_foot", "table_parted", "split_rows",
        "header_not_repeated", "column_fit", "past_margins", "uncertain", "unknown_from")
#: ``settings``' facts compared.
SETTINGS = ("keep_together",)
#: The minimum text sizes tried besides none: one number, and one per kind with a caption's.
MINIMUMS = {"min 9": 9, "min by kind": {"body": 10.5, "table": 9, "caption": 11, "header": 8}}


def _picked(found: dict) -> dict:
    out = {k: v for k, v in found.items() if k.removesuffix("_left_out") in KEYS}
    settings = found.get("settings") or {}
    out["settings"] = {k: v for k, v in settings.items() if k.removesuffix("_left_out") in SETTINGS}
    return out


def _ranges(document) -> list[str]:
    """The range tried: the body's first table, else its middle top-level block."""
    from docx_agent.layout_facts import _W

    main = document.package.document_part()
    body = document.package.tree(main).find(_W + "body")
    if body is None:
        return []
    index = document._index(main)
    blocks = [index.entry_for(child) for child in body if child.tag in (_W + "p", _W + "tbl")]
    blocks = [entry for entry in blocks if entry is not None]
    tables = [entry.id for entry in blocks if entry.id.startswith("t")]
    if tables:
        return [tables[0]]
    return [blocks[len(blocks) // 2].id] if blocks else []


def check_laid(args, run_rust, report, files_of) -> int:
    from word_structure_cases import fonts_of

    expected, manifest = {}, []
    for path in files_of(args):
        try:
            document = Document.open(path.read_bytes())
            layout = document.layout()
            fonts = fonts_of(document)
        except Exception as error:  # noqa: BLE001 -- a test instrument: say what failed
            print(f"{path}: python failed to lay out: {error}")
            continue
        for case, minimum in {"none": None, **MINIMUMS}.items():
            try:
                found = json.loads(json.dumps(_picked(layout_facts(layout, min_size=minimum))))
            except Exception as error:  # noqa: BLE001
                print(f"{path} [{case}]: python failed: {error}")
                continue
            expected[(str(path), case)] = {"facts": found}
            item = {"file": str(path), "case": case, "laid": True, "fonts": fonts}
            if minimum is not None:
                item["min_size"] = minimum
            manifest.append(item)
        # One range: the body's first table, else its middle top-level block (check's
        # range: the facts about its blocks, on its pages).
        ranges = _ranges(document)
        if ranges:
            try:
                scope = Scope(document, ranges)
                pages = sorted({p for found in scope.pages(layout) for p in found})
                found = json.loads(json.dumps(_picked(layout_facts(layout, pages=pages, scope=scope))))
            except Exception as error:  # noqa: BLE001
                print(f"{path} [range]: python failed: {error}")
                continue
            expected[(str(path), "range")] = {"facts": found}
            manifest.append({"file": str(path), "case": "range", "laid": True, "fonts": fonts, "range": ranges})
    rust = {key: line["reply"] for key, line in run_rust(args.rust, manifest).items()}
    return report(expected, rust)
