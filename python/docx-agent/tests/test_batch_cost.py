"""What a batch of Word ops costs does not grow with the document (round 28, W8), and its
results are the ones each op gives alone.

The Rust session serves every op of a batch: ``keep_together``'s warning (a table taller than
a page) is read from one layout after the batch's last op, not one per op, and
``word_edit_table``'s table as Markdown counts the list items before it.

The golden-style test runs one mixed batch -- formatting, cells set, rows, keeps that warn,
text, Markdown, a failing op -- and each of its ops alone, and compares the results."""

from __future__ import annotations

import contextlib
import datetime as dt
import functools

import pytest

from ooxml_edit.tools import Limits, Toolbox

from docx_agent.rust import edits as _edits
from docx_agent.rust import reads as _reads
from docx_agent.rust.engines import session_pages
from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

CLOCK = lambda: dt.datetime(2026, 11, 3, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731


@functools.lru_cache(maxsize=1)
def report() -> bytes:
    """Three tables: the first with a list item in a cell that counts on from the body's
    list, followed by a paragraph and a table (a keep with the next warns); the third taller
    than a page."""
    rows = "\n".join(f"| Line {k} | {k * 3} |" for k in range(70))
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(word_docs.blank(), "report.docx")
        word_docs.call(box, session, "word_insert_markdown", doc=doc, at="end", markdown="\n\n".join([
            "# Report title", "An opening paragraph.", "1. First step\n2. Second step",
            "| Name | Count |\n|---|---|\n| North | 4 |\n| South | 5 |",
            "A paragraph between two tables.",
            "| Kind | Share |\n|---|---|\n| Paper | 40 |\n| Glass | 60 |",
            "# Chapter title", "Some words before the long table.",
            "| Line | Value |\n|---|---|\n" + rows,
            "A closing paragraph.",
        ]))
        first = word_docs.tables(session, doc)[0]
        cell = word_docs.cells_of(word_docs.rows_of(first)[1])[0].find("{*}p")
        target = f"p:{cell.get(word_docs.W14_PARA)}"
        step = next(block for block, text in word_docs.paragraphs(session, doc) if text == "First step")
        # the body list's own definition and level, as the cell's paragraph takes it on
        word_docs.call(box, session, "word_format", doc=doc, targets=[target], like=step)
        return word_docs.saved(session, doc)


def mixed(box, session, doc: str) -> list[dict]:
    first, second, long = [block for block, _ in word_docs.blocks(box, session, doc) if block.startswith("t:")]
    texts = [block for block, text in word_docs.paragraphs(session, doc)
             if text.startswith(("An opening", "A closing", "Some words"))]

    def op(tool, **arguments):
        return {"tool": tool, "arguments": {"doc": doc, **arguments}}

    return [
        op("word_format_table", table=first, scope="table", borders="single"),
        op("word_edit_table", table=first, action="set_cells", cells=[{"row": 2, "col": 1, "text": "6"}]),
        op("word_format_table", table=first, scope="table", keep_together=True, keep_with_next=True),
        op("word_format_table", table=long, scope="table", keep_together=True),
        op("word_set_text", target=texts[0], text="A new opening paragraph."),
        op("word_format", targets=[texts[1]], bold=True),
        op("word_edit_table", table=second, action="insert_row"),
        op("word_edit_table", table="t:7FFF0000", action="insert_row"),
        op("word_insert_markdown", at=f"after:{texts[2]}", markdown="New words.\n\n- one\n- two"),
        op("word_format_table", table=second, scope="row", row=0, shading="D9E2F3"),
        op("word_edit_table", table=long, action="set_cells",
           cells=[{"row_label": "Line 5", "col_label": "Value", "text": "fifteen"}]),
        op("word_format_table", table=long, scope="table", header_rows=1),
        op("word_edit_table", table=first, action="set_cells", cells=[{"row": 1, "col": 1, "text": "7"}]),
    ]


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def _plain(result) -> dict:
    data = result.to_json()
    data.pop("duration", None)
    return {"result": data, "touched": result.touched,
            "touched_by_op": {str(k): v for k, v in result.touched_by_op.items()}}


def _laid(session, doc) -> int:
    """The versions the document's page engine laid out so far, whole or again in part."""
    stats = dict(session_pages(_reads.owned(session.entry(doc))).stats())
    return int(stats["full"] + stats["relaid"])


def _run(box, data: bytes, during=contextlib.nullcontext) -> dict:
    """The mixed batch's results and each op's alone; ``during``: a context the batch itself
    runs in (what it costs, counted)."""
    session = box.session(clock=CLOCK)
    doc = session.open(data, "report.docx")
    # laid out before: the checks' reflow compares with it
    assert box.dispatch(session, "check", {"doc": doc, "include": ["reflow"]}).ok
    ops = mixed(box, session, doc)
    with during(session, doc):
        batch = _plain(box.dispatch(session, "batch", {"ops": ops}))
    out = {"batch": batch}
    alone = []
    for op in ops:
        single = box.session(clock=CLOCK)
        single.open(data, "report.docx")
        assert box.dispatch(single, "check", {"doc": doc, "include": ["reflow"]}).ok
        alone.append(_plain(box.dispatch(single, op["tool"], op["arguments"])))
    out["alone"] = alone
    return out


def test_a_mixed_batch_gives_the_results_each_op_gives_and_lays_out_once(box):
    data = report()
    cost: dict = {}

    @contextlib.contextmanager
    def counting(session, doc):
        """What the batch costs: the editing calls the session answered and the versions the
        page engine laid out."""
        served, laid = _edits.served(), _laid(session, doc)
        yield
        cost["served"] = {k: v - served.get(k, 0) for k, v in _edits.served().items()}
        cost["laid"] = _laid(session, doc) - laid

    fast = _run(box, data, counting)
    probe = box.session(clock=CLOCK)
    tables = sum(op["tool"] == "word_edit_table" for op in mixed(box, probe, probe.open(data, "report.docx")))
    # Every word_edit_table op is the session's, which reads its table in Rust; the document
    # is laid out once, after the last op.
    assert cost["served"]["word_edit_table"] == tables, cost
    assert cost["laid"] == 1, cost
    batch = fast["batch"]["result"]
    warnings = batch["warnings"]
    taller = [w for w in warnings if "taller than a page" in w]
    loose = [w for w in warnings if "does not keep with the next" in w]
    assert taller and taller[0].startswith("ops[3]:") and loose and loose[0].startswith("ops[2]:")
    assert batch["data"]["failed"] == [7] and batch["checks"]["reflow"] is not None
    first_table = batch["data"]["ops"][1]["data"]["markdown"]
    assert '<ol start="3">' in first_table         # counted on from the body's list
    assert fast["alone"][3]["result"]["warnings"] == [taller[0].removeprefix("ops[3]: ")]


def test_keep_together_lays_the_document_out_once_per_batch(box):
    session = box.session(clock=CLOCK)
    doc = session.open(report(), "report.docx")
    tables = [block for block, _ in word_docs.blocks(box, session, doc) if block.startswith("t:")]
    ops = [{"tool": "word_format_table", "arguments": {"doc": doc, "table": t, "scope": "table",
                                                       "keep_together": True}} for t in tables * 2]
    before = _laid(session, doc)
    result = box.dispatch(session, "batch", {"ops": ops})
    assert result.ok and sum("taller than a page" in w for w in result.warnings) == 2
    assert _laid(session, doc) - before == 1       # after the last op; each op's own would be six


def test_a_batch_past_its_deadline_says_what_was_not_run(box):
    session = box.session(clock=CLOCK, limits=Limits(batch_timeout=0.0))
    doc = session.open(report(), "report.docx")
    before = word_docs.saved(session, doc)
    tables = [block for block, _ in word_docs.blocks(box, session, doc) if block.startswith("t:")]
    ops = [{"tool": "word_format_table", "arguments": {"doc": doc, "table": t, "scope": "table",
                                                       "borders": "single"}} for t in tables]
    result = box.dispatch(session, "batch", {"ops": ops})
    assert not result.ok and result.error.code == "timeout"
    assert result.error.details["not_run"] == {"count": 3, "first": 0, "last": 2, "ran": 0,
                                               "seconds": result.error.details["not_run"]["seconds"]}
    assert "Not run: 3 op(s), ops[0]..ops[2]" in result.error.message
    assert "resend them as smaller batches" in result.error.message
    assert word_docs.saved(session, doc) == before
