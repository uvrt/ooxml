"""A batch that derives a dataset from a workbook, then syncs a Word table from it, gives the
same answers, report, workbook and datasets whether the toolbox prepares batches on drafts
(``block_edits="always"``) or not (round 51; ooxml-edit 0.108.0).  Since Phase 3 E1b-2 the report
is the Rust host's and the workbook the Python host's: one batch changing both is refused with the
way out, two batches -- the derive, then the sync and the note -- which is what runs here
(decision D2).

One session holds a made-up report (docx-agent, a test-only dependency) with three tables and a
made-up workbook of sales and products.  Each batch derives with a join and a group
(``xlsx_derive``: the workbook only read), syncs one table from the dataset and sets one
paragraph.  The report is the Rust session's (docx-agent 0.2.0: made by
docx-agent's ``new_document``, read through the tools alone), which a draft does not hold: the
batch runs under the documents' locks in both modes, so no result names a draft.
"""

from __future__ import annotations

import copy
import hashlib
import re

import pytest
from ooxml_edit.tools import Toolbox

docx_tools = pytest.importorskip("docx_agent.tools")

from xlsx_agent import Workbook  # noqa: E402
from xlsx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402

from blank_word import blank_word  # noqa: E402

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
    blank = blank_word()
    with Toolbox(docx_tools.TOOLS, formats=[docx_tools.FORMAT], groups=docx_tools.GROUPS, workers=0) as box:
        session = box.session()
        session.open(blank, "report.docx")
        assert box.dispatch(session, "word_insert_markdown", {"doc": "d1", "at": "end", "markdown": markdown}).ok
        return saved(box, session, "d1")


def saved(box, session, doc: str) -> bytes:
    """The document's bytes as ``save_document`` writes them."""
    result = box.dispatch(session, "save_document", {"doc": doc, "name": "out.docx", "format": "docx"})
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


def two(given: dict) -> list[dict]:
    """A batch of :func:`batch` as decision D2's way out sends it: the derive (the workbook's),
    then the sync and the note (the report's)."""
    rest = {key: value for key, value in given.items() if key != "ops"}
    return [{**rest, "ops": given["ops"][:1]}, {**rest, "ops": given["ops"][1:]}]


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


def test_a_derive_then_sync_batch_is_the_same_with_drafts_or_without(made):
    out = {}
    for mode in ("off", "always"):
        with _box(mode) as box:
            session, tables, notes = _session(box, *made)
            refused = box.dispatch(session, "batch", batch(0, 0, tables, notes))
            assert refused.error.code == "refused" and "send them as two batches" in refused.error.message
            assert refused.error.details == {"batches": [[0], [1, 2]]}
            assert refused.error.try_call["arguments"]["ops"] == batch(0, 0, tables, notes)["ops"][:1]
            results = [*(box.dispatch(session, "batch", b) for b in two(batch(0, 0, tables, notes))),
                       box.dispatch(session, "xlsx_set_cells", {"doc": "d2", "cells": [
                           {"address": "Sales!D3", "number": 500}]}),
                       *(box.dispatch(session, "batch", b) for b in two(batch(1, 1, tables, notes))),
                       *(box.dispatch(session, "batch", b)
                         for b in two({**batch(0, 2, tables, notes), "atomic": True}))]
            assert all(r.ok for r in results), [r.to_json() for r in results if not r.ok]
            out[mode] = ([normal(r) for r in results], digest(box, session, "d1"), digest(box, session, "d2"),
                         datasets(session))
            blocks = [r.block for r in results]
    assert out["off"] == out["always"]
    assert blocks == [None] * 7, blocks                         # no draft holds the Rust session's report
    assert out["always"][0][0]["doc"] == "d2"                   # the derive's batch names the workbook

