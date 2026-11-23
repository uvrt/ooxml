"""A batch that derives a dataset from a workbook, then syncs a Word table from it, runs on a
draft of the Word document (round 51; ooxml-edit 0.108.0).

One session holds a made-up report (docx-agent, a test-only dependency) with three tables and a
made-up workbook of sales and products.  Each batch derives with a join and a group
(``xlsx_derive``: the workbook only read), syncs one table from the dataset and sets one
paragraph.  On a draft, the workbook is read at its version as the derive runs; the Word change
is applied, and the dataset kept, only while the workbook is at that version -- else the batch
is prepared again.  So the answers, the report, the workbook and the datasets are those of the
same calls one at a time, in the order they were applied.

The report is the Rust session's (docx-agent 0.2.0): made by ``ooxml_core.new_document``,
read through the tools alone.
"""

from __future__ import annotations

import copy
import hashlib
import re

import pytest
from ooxml_edit.tools import Toolbox

ooxml_core = pytest.importorskip("ooxml_core")
docx_tools = pytest.importorskip("docx_agent.tools")

from xlsx_agent import Workbook  # noqa: E402
from xlsx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402

REGIONS = ["North", "South", "East", "West"]


def book() -> bytes:
    sales = [[f"S-{i:03}", REGIONS[i % 4], f"P-{i % 5}", (i * 37) % 101 + 1] for i in range(40)]
    products = [[f"P-{k}", f"Part {k}"] for k in range(5)]
    return Workbook.from_tables({
        "Sales": {"columns": ["Ref", "Region", "Product", "Amount"], "table": "Sales", "rows": sales},
        "Products": {"columns": ["Product", "Name"], "table": "Products", "rows": products},
    }).to_bytes()


def report() -> bytes:
    markdown = "\n\n".join(f"# Part {k}\n\nFigures for part {k}.\n\nRead with care.\n\n"
                           f"| Product | Name | Total |\n|---|---|---|\n| x | y | 0 |\n\nSource: the book.\n\n"
                           f"More on part {k}." for k in range(3))
    blank = bytes(ooxml_core.new_document(page=("A4", False), language="en-US", separators=(".", ","),
                                          properties=(None, None, "2026-11-23T09:00:00Z")))
    with Toolbox(docx_tools.TOOLS, formats=[docx_tools.FORMAT], groups=docx_tools.GROUPS, workers=0) as box:
        session = box.session()
        session.open(blank, "report.docx")
        assert box.dispatch(session, "word_insert_markdown", {"doc": "d1", "at": "end", "markdown": markdown}).ok
        return saved(box, session, "d1")


def saved(box, session, doc: str) -> bytes:
    """The document's bytes as ``save_document`` writes them."""
    result = box.dispatch(session, "save_document", {"doc": doc, "name": "out.docx"})
    assert result.ok, result.to_json()
    return session.take_outputs()[-1].data


def _box(mode: str) -> Toolbox:
    return Toolbox([*docx_tools.TOOLS, *TOOLS], formats=[docx_tools.FORMAT, FORMAT],
                   groups=[*docx_tools.GROUPS, *GROUPS], workers=0, block_edits=mode)


def _session(box, word: bytes, workbook: bytes):
    session = box.session()
    session.open(word, "report.docx")        # d1
    session.open(workbook, "sales.xlsx")     # d2
    markdown = box.dispatch(session, "word_read", {"doc": "d1"}).data
    first = re.search(r"<!-- ([pt]:[0-9A-F]+) -->", markdown).group(1)
    blocks = box.dispatch(session, "word_inspect", {"doc": "d1", "target": f"{first}.."}).data["blocks"]
    tables = [block["id"] for block in blocks if block["id"].startswith("t:")]
    notes = [block["id"] for block in blocks if block["id"].startswith("p:") and block.get("t", "").startswith("Source:")]
    assert len(tables) == len(notes) == 3, blocks
    return session, tables, notes


def batch(k: int, c: int, tables: list[str], notes: list[str]) -> dict:
    """Derive (join and group: never a filter alone), sync table k, set its note."""
    id_ = f"part{k}"
    return {"ops": [
        {"tool": "xlsx_derive", "arguments": {
            "doc": "d2", "from": "t:Sales", "id": id_,
            "where": [{"column": "Region", "op": "in", "values": REGIONS[k:k + 2]}],
            "join": {"from": "t:Products", "on": ["Product", "Product"], "columns": ["Name"]},
            "group_by": ["Product", "Name"], "measures": [{"fn": "sum", "column": "Amount", "as": "Total"}],
            "sort": [{"column": "Product"}], "top": 2 + (c % 3)}},
        {"tool": "word_edit_table", "arguments": {"doc": "d1", "table": tables[k], "action": "sync",
                                                  "from": f"d:{id_}", "key": "Product", "remove": True}},
        {"tool": "word_set_text", "arguments": {"doc": "d1", "target": notes[k],
                                                "text": f"Source: the book, run {c}."}},
    ]}


def normal(result) -> dict:
    out = copy.deepcopy(result.to_json())
    for op in (out.get("data") or {}).get("ops") or []:
        if isinstance(op.get("data"), dict):
            op["data"].pop("timing", None)
    if isinstance(out.get("data"), dict):
        out["data"].pop("timing", None)
    out["warnings"] = [w for w in out.get("warnings") or [] if "changed while this call read it" not in w]
    return out


def datasets(session) -> dict:
    return {id_: (entry.doc, entry.version, entry.stale, [list(map(str, row)) for row in entry.dataset.rows])
            for id_, entry in session.datasets.items()}


def digest(box, session, doc: str) -> str:
    data = saved(box, session, doc) if doc == "d1" else session.entry(doc).document.to_bytes()
    return hashlib.sha256(data).hexdigest()


@pytest.fixture(scope="module")
def made():
    return report(), book()


def test_a_derive_then_sync_batch_runs_on_a_draft_as_under_both_locks(made):
    out = {}
    for mode in ("off", "always"):
        with _box(mode) as box:
            session, tables, notes = _session(box, *made)
            results = [box.dispatch(session, "batch", batch(0, 0, tables, notes)),
                       box.dispatch(session, "xlsx_set_cells", {"doc": "d2", "cells": [
                           {"address": "Sales!D3", "number": 500}]}),
                       box.dispatch(session, "batch", batch(1, 1, tables, notes)),
                       box.dispatch(session, "batch", {**batch(0, 2, tables, notes), "atomic": True})]
            assert all(r.ok for r in results), [r.to_json() for r in results if not r.ok]
            out[mode] = ([normal(r) for r in results], digest(box, session, "d1"), digest(box, session, "d2"),
                         datasets(session))
            blocks = [r.block for r in results]
    assert out["off"] == out["always"]
    assert blocks[1] is None                                    # the workbook's own change
    assert all(b and b.get("lock") is None and b["tries"] == 1 for k, b in enumerate(blocks) if k != 1), blocks
    assert out["always"][0][0]["doc"] == "d2"                   # names its first document, as before

