"""An edit's cost proportional to its change (lane G) says what the whole path says.

The same made-up sequences -- Markdown inserts with lists and tables, text set, a table's
rows added and filled, the fields updated, undo and redo, tracked edits, several writers'
turns, batches, checks -- run with the per-change path on and off (``DOCX_AGENT_INCREMENTAL``),
and after every step the two are compared: the call's result (its timing aside), the
document's bytes, its validation (and, at every step, the problems found per change against
the whole :func:`docx_agent.validate.check`), and the version each call reports.
"""

from __future__ import annotations

import copy
import datetime as dt
import random
import threading

import pytest

from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.limits import Limits

from docx_agent import Document, revalidate, validate
from docx_agent.tools import FORMAT, GROUPS, TOOLS

CLOCK = lambda: dt.datetime(2026, 11, 8, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
WORDS = ("lantern meadow river stone harbour quiet orchard copper valley signal morning ledger "
         "window garden timber bridge market evening canvas thread pebble summit").split()
INCLUDE = ["validate", "layout", "numbers", "reflow", "fields"]


def _sentence(rng: random.Random, n: int) -> str:
    return " ".join(rng.choice(WORDS) for _ in range(n)).capitalize() + "."


def _chapter(rng: random.Random, n: int, sections: int) -> str:
    parts = [f"# Chapter {n} {_sentence(rng, 2)[:-1]}"]
    for sub in range(sections):
        parts.append(f"## Section {n}.{sub + 1} {_sentence(rng, 2)[:-1]}")
        parts += [" ".join(_sentence(rng, rng.randint(10, 22)) for _ in range(3)) for _ in range(2)]
        if sub % 3 == 1:
            parts.append(f"- {_sentence(rng, 6)}\n- {_sentence(rng, 8)}\n  - {_sentence(rng, 5)}")
        if sub % 3 == 2:
            parts.append(f"1. {_sentence(rng, 6)}\n2. {_sentence(rng, 8)}")
        if sub % 3 == 0:
            parts.append("| Item | Value | Comment |\n|---|---|---|\n" + "\n".join(
                f"| {_sentence(rng, 2)[:-1]} | {rng.randint(1, 999)} | {_sentence(rng, 5)} |" for _ in range(5)))
    parts.append(f"End of chapter {n}.")
    return "\n\n".join(parts)


@pytest.fixture(scope="module")
def report() -> bytes:
    return make_report()


def make_report() -> bytes:
    """Two chapters with a contents, lists, tables and a bookmark."""
    rng = random.Random(71)
    document = Document.new()
    document.paragraphs()[0].set_text("Made-up report")
    first = document.paragraphs()[0].id
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(document.to_bytes(), "report.docx")

        def call(tool, **arguments):
            out = box.dispatch(session, tool, {"doc": doc, **arguments}).to_json()
            assert out.get("ok", True), out
            return out

        call("word_fields", action="insert_toc", after=first)
        for n in (1, 2):
            call("word_insert_markdown", at="end", markdown=_chapter(rng, n, 5))
        document = session.entry(doc).document
        heads = [p for p in document.paragraphs() if p.text.startswith("Section ")]
        call("word_links", action="add_bookmark", target=heads[-2].id, bookmark="later")
        return session.entry(doc).document.to_bytes()


def _normal(result: dict) -> dict:
    out = copy.deepcopy(result)
    out.pop("duration", None)
    out.pop("block", None)
    data = out.get("data")
    if isinstance(data, dict):
        data.pop("timing", None)
        for op in data.get("ops") or []:
            if isinstance(op, dict):
                op.pop("duration", None)
                if isinstance(op.get("data"), dict):
                    op["data"].pop("timing", None)
    return out


def _sequence(session, doc):
    document = session.entry(doc).document
    paragraphs = document.paragraphs()
    at = max(k for k, p in enumerate(paragraphs) if p.text.startswith("Chapter 1"))
    body = paragraphs[at:]
    chapters = [p for p in body if p.text.startswith("Chapter ")]
    heads = [p for p in body if p.text.startswith("Section ")]
    ends = [p for p in body if p.text.startswith("End of chapter ")]
    longs = [p for p in body if len(p.text) > 150]
    tables = [t for t in document.tables() if t.column_count == 3]
    one = f"{chapters[0].id}..{ends[0].id}"
    rng = random.Random(5)
    text = lambda n=12: _sentence(rng, n)  # noqa: E731
    return [
        ("a", "word_insert_markdown", {"at": f"before:{heads[2].id}",
                                       "markdown": f"### Note\n\n{text(30)}\n\n- one\n- two\n\n1. first"}),
        ("b", "word_set_text", {"target": longs[1].id, "text": text(40)}),
        ("a", "check", {"range": [one], "include": INCLUDE}),
        ("b", "word_edit_table", {"table": tables[1].id, "action": "insert_row", "index": 2}),
        ("b", "word_edit_table", {"table": tables[1].id, "action": "set_cells",
                                  "cells": [{"row": 2, "col": 0, "text": "Added"}, {"row": 2, "col": 2, "text": text()}]}),
        ("a", "undo", {}),
        ("b", "undo", {}),
        ("b", "undo", {"redo": True}),
        ("a", "word_fields", {"action": "update"}),
        ("c", "batch", {"ops": [
            {"tool": "word_insert_markdown", "arguments": {"doc": doc, "at": f"before:{ends[1].id}",
                                                           "markdown": f"{text(20)}\n\n| A | B |\n|---|---|\n| 1 | 2 |"}},
            {"tool": "word_set_text", "arguments": {"doc": doc, "target": longs[-1].id, "text": text(25)}}]}),
        ("c", "word_set_tracking", {"on": True, "author": "Writer C"}),
        ("c", "word_set_text", {"target": longs[2].id, "text": longs[2].text + " " + text(8)}),
        ("c", "word_insert_markdown", {"at": f"after:{heads[3].id}", "markdown": text(15)}),
        ("c", "word_delete", {"target": longs[3].id}),
        ("c", "word_set_tracking", {"on": False}),
        ("a", "word_links", {"action": "add_bookmark", "target": heads[4].id, "bookmark": "second"}),
        ("a", "word_fields", {"action": "insert_cross_reference", "at": f"{longs[0].id}@0",
                              "bookmark": "second", "kind": "text"}),
        ("b", "word_format", {"targets": [f"{heads[1].id}..{heads[2].id}"], "keep_together": True}),
        ("a", "check", {"include": ["validate"]}),
        ("b", "undo", {}),
        ("a", "undo", {"steps": 2}),
        ("a", "check", {"range": [one], "include": INCLUDE}),
        ("c", "word_changes", {"action": "accept", "all": True}),
        ("a", "check", {"include": ["validate"]}),
    ]


def _run(data: bytes, incremental: bool, monkeypatch) -> list:
    monkeypatch.setenv("DOCX_AGENT_INCREMENTAL", "1" if incremental else "0")
    monkeypatch.setenv("DOCX_AGENT_FULL_VALIDATE", "1")
    revalidate.STATS.clear()
    out = []
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0,
                 limits=Limits(render_timeout=600, layout_timeout=600, batch_timeout=900)) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(data, "report.docx")
        for writer, tool, arguments in _sequence(session, doc):
            view = session.view(context={"agent": writer})
            arguments = dict(arguments)
            if tool != "batch":
                arguments["doc"] = doc
            result = box.dispatch(view, tool, arguments).to_json()
            entry = session.entry(doc)
            with entry.lock:
                document = entry.document
                found = revalidate.find(document)
                assert found == validate.check(document.package), (tool, arguments)
                out.append((tool, result, document.to_bytes(), [str(p) for p in found], entry.version))
    return out, dict(revalidate.STATS)


def test_per_change_says_what_the_whole_path_says(report, monkeypatch):
    mine, stats = _run(report, True, monkeypatch)
    theirs, _ = _run(report, False, monkeypatch)
    assert stats.get("incremental", 0) >= len(mine), stats
    assert not [key for key in stats if key.startswith("fallback")], stats
    assert len(mine) == len(theirs)
    assert sum(1 for _, a, *_ in mine if a.get("ok", True)) >= len(mine) - 3
    for (tool, a, data_a, found_a, version_a), (_, b, data_b, found_b, version_b) in zip(mine, theirs):
        assert _normal(a) == _normal(b), tool
        assert data_a == data_b, tool
        assert found_a == found_b and version_a == version_b, tool


def test_off_by_environment(report, monkeypatch):
    """``DOCX_AGENT_INCREMENTAL=0``: the whole check, every call."""
    monkeypatch.setenv("DOCX_AGENT_INCREMENTAL", "0")
    revalidate.STATS.clear()
    document = Document.open(report)
    assert revalidate.problems(document) == validate.check(document.package)
    assert "incremental" not in revalidate.STATS


def _problem_documents():
    """Made-up documents with problems the rules find, each made by hand in the XML."""
    w = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
    w14 = "{http://schemas.microsoft.com/office/word/2010/wordml}"
    from lxml import etree

    def body(document):
        return document.package.tree(document.package.document_part()).find(w + "body")

    def dirty(document):
        document.package.mark_dirty(document.package.document_part())
        document._invalidate()

    out = []
    document = Document.new()
    document.insert_markdown("# Title\n\nFirst paragraph.\n\nSecond paragraph.\n\n| A | B |\n|---|---|\n| 1 | 2 |")

    def repeat_id(d):
        ps = list(body(d).iter(w + "p"))
        ps[2].set(w14 + "paraId", ps[1].get(w14 + "paraId"))
        dirty(d)

    def bookmark_unpaired(d):
        p = list(body(d).iter(w + "p"))[1]
        etree.SubElement(p, w + "bookmarkStart", {w + "id": "77", w + "name": "orphan"})
        dirty(d)

    def field_open(d):
        r = etree.SubElement(list(body(d).iter(w + "p"))[2], w + "r")
        etree.SubElement(r, w + "fldChar", {w + "fldCharType": "begin"})
        dirty(d)

    def field_close(d):
        r = etree.SubElement(list(body(d).iter(w + "p"))[1], w + "r")
        etree.SubElement(r, w + "fldChar", {w + "fldCharType": "end"})
        dirty(d)

    def style_missing(d):
        p = list(body(d).iter(w + "p"))[1]
        ppr = p.find(w + "pPr")
        if ppr is None:
            ppr = etree.SubElement(p, w + "pPr")
            p.insert(0, ppr)
        etree.SubElement(ppr, w + "pStyle", {w + "val": "NoSuchStyle"})
        dirty(d)

    def order_wrong(d):
        p = list(body(d).iter(w + "p"))[2]
        etree.SubElement(p, w + "pPr")
        dirty(d)

    def cell_ends_badly(d):
        tc = next(body(d).iter(w + "tc"))
        for child in list(tc):
            if child.tag == w + "p":
                tc.remove(child)
        dirty(d)

    def relationship_unknown(d):
        p = list(body(d).iter(w + "p"))[1]
        link = etree.SubElement(p, w + "hyperlink")
        link.set("{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id", "rId999")
        dirty(d)

    def num_missing(d):
        p = list(body(d).iter(w + "p"))[2]
        ppr = p.find(w + "pPr")
        if ppr is None:
            ppr = etree.Element(w + "pPr")
            p.insert(0, ppr)
        numpr = etree.SubElement(ppr, w + "numPr")
        etree.SubElement(numpr, w + "ilvl", {w + "val": "12"})
        etree.SubElement(numpr, w + "numId", {w + "val": "404"})
        dirty(d)

    for make in (repeat_id, bookmark_unpaired, field_open, field_close, style_missing, order_wrong,
                 cell_ends_badly, relationship_unknown, num_missing):
        out.append((make.__name__, document.to_bytes(), make))
    return out


@pytest.mark.parametrize("name, data, make", _problem_documents(), ids=lambda v: v if isinstance(v, str) else "")
def test_problems_are_found_per_change_as_whole(name, data, make, monkeypatch):
    """A problem made in a block, then the block edited again and the change undone: the
    problems found per change are the whole check's at every step."""
    monkeypatch.setenv("DOCX_AGENT_FULL_VALIDATE", "0")
    document = Document.open(data)
    assert revalidate.find(document) == validate.check(document.package)
    with document.batch():
        make(document)
    found = revalidate.find(document)
    assert found == validate.check(document.package)
    assert found, name                                       # the problem is there
    document.paragraphs()[-1].set_text("Edited after")
    assert revalidate.find(document) == validate.check(document.package)
    document.undo()
    document.undo()
    assert revalidate.find(document) == validate.check(document.package) == []


def test_several_writers_at_once_see_the_whole_checks_problems(report, monkeypatch):
    """Writers in threads: every edit's validation (the problems kept at its commit) is the
    whole check's of the same version."""
    monkeypatch.setenv("DOCX_AGENT_FULL_VALIDATE", "1")
    errors = []
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(report, "report.docx")
        paragraphs = [p.id for p in session.entry(doc).document.paragraphs() if len(p.text) > 150]

        def writer(w):
            rng = random.Random(w)
            view = session.view(context={"agent": f"w{w}"})
            for turn in range(3):
                for tool, arguments in (
                        ("word_set_text", {"target": paragraphs[(w * 3 + turn) % len(paragraphs)], "text": _sentence(rng, 20)}),
                        ("word_insert_markdown", {"at": "end", "markdown": _sentence(rng, 12)}),
                        ("check", {"include": ["validate"]})):
                    result = box.dispatch(view, tool, {"doc": doc, **arguments})
                    if not result.ok:
                        errors.append((tool, result.to_json()))

        threads = [threading.Thread(target=writer, args=(w,)) for w in range(4)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
    assert not errors, errors[:3]
