"""A check of some ranges served by the Rust core (lane F, :mod:`docx_agent.rust.check`) says
what docx-agent's own check says, field for field (the timing aside).

The same recorded sequences -- Markdown inserts with lists and tables, text set, a table's
rows added and filled, the fields updated, undo, several writers' turns, checks of a
chapter, of a few sections, of a heading's range, of two ranges -- run with the Rust path on
and off (``DOCX_AGENT_RUST``); every check's result is compared.  Made-up documents only.
Without ``ooxml-core`` these tests are skipped (docx-agent then always checks its own way).
"""

from __future__ import annotations

import copy
import datetime as dt
import random

import pytest

pytest.importorskip("ooxml_edit.tools.numbers", reason="needs ooxml-edit (numbers)")
ooxml_core = pytest.importorskip("ooxml_core", reason="needs ooxml-core (the Rust path)")
if not hasattr(ooxml_core, "check"):
    pytest.skip("needs ooxml-core 0.2.0 (the Rust path's check)", allow_module_level=True)

from ooxml_edit.tools import Toolbox  # noqa: E402
from ooxml_edit.tools.limits import Limits  # noqa: E402

from docx_agent import Document  # noqa: E402
from docx_agent.rust import check as rust_check  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS as WORD_TOOLS  # noqa: E402

from test_tables_from_data import ds_put  # noqa: E402

TOOLS = [*WORD_TOOLS, ds_put]
INCLUDE = ["validate", "layout", "numbers", "reflow", "fields"]
CLOCK = lambda: dt.datetime(2026, 11, 8, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
WORDS = ("lantern meadow river stone harbour quiet orchard copper valley signal morning ledger "
         "window garden timber bridge market evening canvas thread pebble summit").split()

pytestmark = pytest.mark.any_faces


def _sentence(rng: random.Random, n: int) -> str:
    return " ".join(rng.choice(WORDS) for _ in range(n)).capitalize() + "."


def _chapter(rng: random.Random, n: int, sections: int) -> str:
    parts = [f"# Chapter {n} {_sentence(rng, 2)[:-1]}"]
    for sub in range(sections):
        parts.append(f"## Section {n}.{sub + 1} {_sentence(rng, 2)[:-1]}")
        parts += [" ".join(_sentence(rng, rng.randint(10, 22)) for _ in range(4)) for _ in range(2)]
        if sub % 3 == 1:
            parts.append(f"- {_sentence(rng, 6)}\n- {_sentence(rng, 8)}\n  - {_sentence(rng, 5)}")
        if sub % 3 == 2:
            parts.append(f"1. {_sentence(rng, 6)}\n2. {_sentence(rng, 8)}")
        if sub % 3 == 0:
            parts.append("Values:")
            parts.append("| Item | Value | Comment |\n|---|---|---|\n" + "\n".join(
                f"| {_sentence(rng, 2)[:-1]} | {rng.randint(1, 999)} | {_sentence(rng, 5)} |" for _ in range(8)))
    parts.append(f"End of chapter {n}.")
    return "\n\n".join(parts)


@pytest.fixture(scope="module")
def report() -> bytes:
    """Two chapters with a contents, lists, tables, a bookmark, a page cross reference and
    "page X of Y" (about 12 pages)."""
    rng = random.Random(61)
    limits = Limits(render_timeout=600, layout_timeout=600, batch_timeout=900)
    document = Document.new()
    document.paragraphs()[0].set_text("Made-up report")
    first = document.paragraphs()[0].id
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0, limits=limits) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(document.to_bytes(), "report.docx")

        def call(tool, **arguments):
            out = box.dispatch(session, tool, {"doc": doc, **arguments}).to_json()
            assert out.get("ok", True), out
            return out

        call("word_fields", action="insert_toc", after=first)
        for n in (1, 2):
            call("word_insert_markdown", at="end", markdown=_chapter(rng, n, 7))
        call("word_headers_footers", section="s:body", which="footer", action="set", text="Made-up report",
             page_number="page_x_of_y")
        document = session.entry(doc).document
        heads = [p for p in document.paragraphs() if p.text.startswith("Section ")]
        call("word_links", action="add_bookmark", target=heads[-2].id, bookmark="later")
        ends = [p for p in document.paragraphs() if p.text.startswith("End of chapter ")]
        before = document.paragraphs()[[p.id for p in document.paragraphs()].index(ends[0].id) - 1]
        call("word_fields", action="insert_cross_reference", at=f"{before.id}@{len(before.text)}",
             bookmark="later", kind="page")
        call("word_fields", action="update")
        return session.entry(doc).document.to_bytes()


def _normal(result: dict) -> dict:
    out = copy.deepcopy(result)
    for key in ("duration", "block", "version"):
        out.pop(key, None)
    out["warnings"] = [w for w in out.get("warnings") or [] if "newer than at the call" not in w
                       and "changed while this call read it" not in w]
    (out.get("data") or {}).pop("timing", None)
    return out


def _first_difference(a, b, path=""):
    if type(a) is not type(b):
        return f"{path}: {str(a)[:300]} != {str(b)[:300]}"
    if isinstance(a, dict):
        for key in list(a) + [k for k in b if k not in a]:
            if key not in a or key not in b:
                return f"{path}/{key}: only in {'rust' if key in a else 'python'}: {str(a.get(key, b.get(key)))[:300]}"
            found = _first_difference(a[key], b[key], f"{path}/{key}")
            if found:
                return found
        return None
    if isinstance(a, list):
        if len(a) != len(b):
            return f"{path}: {len(a)} items != {len(b)}: {str(a)[:300]} || {str(b)[:300]}"
        for k, (x, y) in enumerate(zip(a, b)):
            found = _first_difference(x, y, f"{path}[{k}]")
            if found:
                return found
        return None
    return None if a == b else f"{path}: {str(a)[:300]} != {str(b)[:300]}"


def _ids(session, doc):
    """The report's blocks the sequences name, read from the document as it is now (after
    the contents, which repeats the headings' texts)."""
    paragraphs = session.entry(doc).document.paragraphs()
    at = max(k for k, p in enumerate(paragraphs) if p.text.startswith("Chapter 1"))
    body = paragraphs[at:]
    chapters = [p for p in body if p.text.startswith("Chapter ")]
    heads = [p for p in body if p.text.startswith("Section ")]
    ends = [p for p in body if p.text.startswith("End of chapter ")]
    longs = [p for p in body if len(p.text) > 200]
    return chapters, heads, ends, longs


def _sequence(session, doc):
    chapters, heads, ends, longs = _ids(session, doc)
    tables = [t for t in session.entry(doc).document.tables() if t.column_count == 3]
    one = f"{chapters[0].id}..{ends[0].id}"
    two = f"{chapters[1].id}..{ends[1].id}"
    few = f"{heads[2].id}..{heads[4].id}"
    long_text = " ".join(WORDS * 3) + "."
    return [
        ("a", "check", {"range": [one], "include": INCLUDE}),
        ("b", "check", {"range": [few], "include": INCLUDE}),
        ("a", "word_insert_markdown", {"at": f"before:{heads[3].id}",
                                       "markdown": f"### Note\n\n{long_text}\n\n- one\n- two\n\n1. first"}),
        ("b", "check", {"range": [few], "include": INCLUDE}),
        ("a", "check", {"range": [one]}),
        ("b", "set_text", {"target": longs[3].id, "text": longs[3].text + " " + long_text}),
        ("a", "check", {"range": [one, two], "include": INCLUDE}),
        ("b", "word_edit_table", {"table": tables[1].id, "action": "insert_row", "index": 2}),
        ("b", "word_edit_table", {"table": tables[1].id, "action": "set_cells",
                                  "cells": [{"row": 2, "col": 0, "text": "Added"}, {"row": 2, "col": 2,
                                                                                  "text": long_text}]}),
        ("a", "check", {"range": [two], "include": INCLUDE}),
        ("a", "undo", {}),
        ("b", "check", {"range": [few], "include": INCLUDE}),
        ("a", "word_fields", {"action": "update"}),
        ("b", "word_format", {"targets": [f"{heads[5].id}..{heads[6].id}"], "keep_together": True}),
        ("b", "check", {"range": [f"{heads[5].id}..{heads[6].id}"], "include": INCLUDE}),
        ("b", "check", {"range": [f"h:{heads[1].id}"], "include": INCLUDE}),
        ("a", "set_text", {"target": longs[-2].id, "text": longs[-2].text[::-1]}),
        ("b", "check", {"range": [two], "include": ["layout", "reflow"]}),
        ("a", "check", {"range": [one], "include": INCLUDE}),
    ]


def _run(data: bytes, rust: bool, monkeypatch) -> list:
    monkeypatch.setenv("DOCX_AGENT_RUST", "1" if rust else "0")
    rust_check.STATS.clear()
    out = []
    limits = Limits(render_timeout=600, layout_timeout=600, batch_timeout=900)
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=1, limits=limits) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(data, "report.docx")
        assert box.dispatch(session, "ds_put", {"id": "values"}).ok
        for writer, tool, arguments in _sequence(session, doc):
            view = session.view(context={"agent": writer})
            result = box.dispatch(view, tool, {"doc": doc, **arguments}).to_json()
            out.append((tool, arguments, result))
    return out, dict(rust_check.STATS)


def test_checks_served_in_rust_say_what_docx_agent_says(report, monkeypatch):
    mine, stats = _run(report, True, monkeypatch)
    theirs, _ = _run(report, False, monkeypatch)
    assert stats.get("served", 0) >= 6, stats            # most of them served there
    compared = 0
    for (tool, arguments, a), (_, _, b) in zip(mine, theirs):
        assert a.get("ok", True) == b.get("ok", True), (tool, arguments, a, b)
        if tool != "check":
            continue
        assert _normal(a) == _normal(b), (arguments, _first_difference(_normal(a), _normal(b)))
        compared += 1
    assert compared == 11


def test_off_by_environment(report, monkeypatch):
    """``DOCX_AGENT_RUST=0``: nothing served there."""
    _, stats = _run(report, False, monkeypatch)
    assert "served" not in stats and "opened" not in stats


def test_unsupported_document_is_checked_docx_agent_s_way(monkeypatch):
    """A document with a footnote is not modelled there: checked as before, the same."""
    document = Document.new()
    document.insert_markdown("# Title\n\nSome text with a note.[^1]\n\n[^1]: The note.\n\nMore text.")
    data = document.to_bytes()
    results = []
    for rust in (True, False):
        monkeypatch.setenv("DOCX_AGENT_RUST", "1" if rust else "0")
        rust_check.STATS.clear()
        with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
            session = box.session(clock=CLOCK)
            doc = session.open(data, "notes.docx")
            first = session.entry(doc).document.paragraphs()[0].id
            results.append(_normal(box.dispatch(session, "check", {"doc": doc, "range": [first],
                                                                    "include": INCLUDE}).to_json()))
        if rust:
            assert "served" not in rust_check.STATS, dict(rust_check.STATS)
    assert results[0] == results[1]


def test_a_glyph_its_face_lacks_is_said_the_same(monkeypatch):
    """A character its face has no glyph for: measured with another, said as an
    approximation in ``reflow``'s coverage and warnings -- the same both ways."""
    document = Document.new()
    document.insert_markdown("# Title\n\n" + "\n\n".join(
        f"Paragraph {k} with a snowman ☃ and more text to fill a line or two of the page." for k in range(20)))
    data = document.to_bytes()
    results = []
    for rust in (True, False):
        monkeypatch.setenv("DOCX_AGENT_RUST", "1" if rust else "0")
        rust_check.STATS.clear()
        with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
            session = box.session(clock=CLOCK)
            doc = session.open(data, "glyph.docx")
            paragraphs = session.entry(doc).document.paragraphs()
            results.append(_normal(box.dispatch(session, "check", {
                "doc": doc, "range": [f"{paragraphs[0].id}..{paragraphs[-1].id}"], "include": INCLUDE}).to_json()))
    assert results[0] == results[1], _first_difference(results[0], results[1])
