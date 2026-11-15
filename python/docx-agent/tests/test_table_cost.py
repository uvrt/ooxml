"""``word_edit_table`` at its table's cost (lane H1) says what the whole-document reading says.

The Markdown of the table an edit returns reads the list items before it only when the table
numbers one of its own (``markdown.read.Reader.block``), and its paragraphs' formatting from
docx2svg's model of that table alone (``edit.effective.table_model``) where nothing else of
the body reaches it.  Made-up documents -- lists before and inside tables, a restarted list,
nested tables, a styled table, a table in a content control, deleted paragraph marks in and
before a table -- are read and edited with the per-change path on and off
(``DOCX_AGENT_INCREMENTAL``), and the results and saved bytes compared.
"""

from __future__ import annotations

import copy
import datetime as dt

import pytest
from lxml import etree

from ooxml_edit.tools import Toolbox

from docx_agent import Document
from docx_agent.tools import FORMAT, GROUPS, TOOLS

CLOCK = lambda: dt.datetime(2026, 11, 8, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"

MARKDOWN = """# Lists and tables

1. First counted item
2. Second counted item

| Name | Value |
|---|---|
| alpha | 1 |
| beta | 2 |
| gamma | 3 |

- A bullet
- Another bullet

3. A third item, counted on

| Key | Note |
|---|---|
| one | **bold** text |
| two | *italic* text |

Some text between.

1. A new list
2. Its second item

| Plain | Table |
|---|---|
| x | y |
| z | w |

Closing paragraph.
"""


def _numbered_into_cells(document: Document) -> None:
    """Copies of the numbered paragraphs into the first cell of the second table and the
    last cell of the third: list items a table numbers itself."""
    body = document.package.tree(document.package.document_part()).find(_W + "body")
    from docx_agent.edit.numbering import membership

    numbered = [p for p in body if p.tag == _W + "p" and membership(document, p) is not None]
    assert len(numbered) >= 5
    tables = body.findall(_W + "tbl")
    first = tables[1].find(f"{_W}tr/{_W}tc")
    first.append(copy.deepcopy(numbered[0]))
    first.append(copy.deepcopy(numbered[-1]))
    last = tables[2].findall(f"{_W}tr")[-1].findall(_W + "tc")[-1]
    last.append(copy.deepcopy(numbered[2]))
    # a nested table in the first table's last cell, with a list item in it
    nested = copy.deepcopy(tables[2])
    cell = nested.find(f"{_W}tr/{_W}tc")
    cell.append(copy.deepcopy(numbered[1]))
    host = tables[0].findall(f"{_W}tr")[-1].findall(_W + "tc")[-1]
    host.append(nested)
    host.append(etree.SubElement(host, _W + "p"))
    document.package.mark_dirty(document.package.document_part())
    document._invalidate()


def _deleted_marks(document: Document) -> None:
    """A paragraph mark deleted (tracked) just before the third table, and one inside the
    second table: docx2svg's final view joins them across blocks."""
    body = document.package.tree(document.package.document_part()).find(_W + "body")
    tables = body.findall(_W + "tbl")

    def delete_mark(paragraph, number: int) -> None:
        properties = paragraph.find(_W + "pPr")
        if properties is None:
            properties = etree.Element(_W + "pPr")
            paragraph.insert(0, properties)
        marks = properties.find(_W + "rPr")
        if marks is None:
            marks = etree.SubElement(properties, _W + "rPr")
        etree.SubElement(marks, _W + "del", {_W + "id": str(number), _W + "author": "Writer",
                                              _W + "date": "2026-11-08T09:00:00Z"})

    before = tables[2].getprevious()
    while before is not None and before.tag != _W + "p":
        before = before.getprevious()
    delete_mark(before, 9001)
    delete_mark(tables[1].find(f"{_W}tr/{_W}tc/{_W}p"), 9002)
    document.package.mark_dirty(document.package.document_part())
    document._invalidate()


def _in_control(document: Document) -> None:
    """The first table wrapped in a block-level content control."""
    body = document.package.tree(document.package.document_part()).find(_W + "body")
    table = body.find(_W + "tbl")
    sdt = etree.Element(_W + "sdt")
    properties = etree.SubElement(sdt, _W + "sdtPr")
    etree.SubElement(properties, _W + "id", {_W + "val": "4242"})
    content = etree.SubElement(sdt, _W + "sdtContent")
    table.addprevious(sdt)
    content.append(table)
    document.package.mark_dirty(document.package.document_part())
    document._invalidate()


def _document(variant: str) -> bytes:
    document = Document.new()
    document.paragraphs()[0].set_text("Made-up tables")
    document.insert_markdown(MARKDOWN)
    if variant in ("cells", "tracked", "control"):
        _numbered_into_cells(document)
    if variant == "tracked":
        _deleted_marks(document)
    if variant == "control":
        _in_control(document)
    return document.to_bytes()


VARIANTS = ["plain", "cells", "tracked", "control"]


def _run(data: bytes, monkeypatch, incremental: bool) -> tuple[list, bytes]:
    monkeypatch.setenv("DOCX_AGENT_INCREMENTAL", "1" if incremental else "0")
    out = []
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(data, "tables.docx")
        document = session.entry(doc).document
        tables = [t.id for t in document.tables()]
        for k, table in enumerate(tables):
            result = box.dispatch(session, "word_edit_table", {
                "doc": doc, "table": table, "action": "set_cells",
                "cells": [{"row": 1, "col": 1, "text": f"value {k}"}]}).to_json()
            result.get("data", {}).pop("timing", None)
            out.append(result)
            out.append(box.dispatch(session, "word_read", {"doc": doc, "range": table}).to_json())
        out.append(box.dispatch(session, "undo", {"doc": doc}).to_json())
        document = session.entry(doc).document
        out.append([document.to_markdown(t.id, ids=False) for t in document.tables()])
        out.append([document.to_markdown(t.id) for t in document.tables()])
        saved = document.to_bytes()
    return out, saved


@pytest.mark.parametrize("variant", VARIANTS)
def test_table_edits_same_either_way(variant, monkeypatch):
    data = _document(variant)
    fast, fast_bytes = _run(data, monkeypatch, True)
    whole, whole_bytes = _run(data, monkeypatch, False)
    assert fast == whole
    assert fast_bytes == whole_bytes


@pytest.mark.parametrize("variant", VARIANTS)
def test_table_markdown_same_either_way(variant, monkeypatch):
    """Every table read each way in one state, every view, with and without ids."""
    document = Document.open(_document(variant))
    document.paragraphs()[0].set_text("Made-up tables, edited")   # a state no parse has read
    tables = [t.id for t in document.tables()]
    for view in ("final", "original", "markup"):
        for ids in (False, True):
            for table in tables:
                monkeypatch.setenv("DOCX_AGENT_INCREMENTAL", "1")
                document._invalidate()
                fast = document.to_markdown(table, view=view, ids=ids)
                monkeypatch.setenv("DOCX_AGENT_INCREMENTAL", "0")
                document._invalidate()
                whole = document.to_markdown(table, view=view, ids=ids)
                assert fast == whole, (variant, view, ids, table)


def test_a_table_numbering_nothing_counts_nothing_before(monkeypatch):
    """The list items before a table are not counted where it numbers none of its own; a
    table that does numbers on from them."""
    from docx_agent.markdown import read

    calls = []
    original = read.Reader._count_before

    def counted(self, part, groups):
        calls.append(len(groups))
        return original(self, part, groups)

    monkeypatch.setattr(read.Reader, "_count_before", counted)
    monkeypatch.setenv("DOCX_AGENT_INCREMENTAL", "1")
    monkeypatch.delenv("DOCX_AGENT_FULL_VALIDATE", raising=False)   # (which reads both ways)
    document = Document.open(_document("plain"))
    document.to_markdown(document.tables()[-1].id, ids=False)        # no list item in it
    assert calls == []
    document = Document.open(_document("cells"))
    text = document.to_markdown(document.tables()[2].id, ids=False)  # list items in its first cell
    assert calls and 'start="4"' in text


def test_table_model_used_and_refused(monkeypatch):
    """The table's own model is made for a plain table of the body, not for one after a
    deleted paragraph mark or inside a content control."""
    from docx_agent.edit import effective

    monkeypatch.setenv("DOCX_AGENT_INCREMENTAL", "1")
    document = Document.open(_document("plain"))
    body = document.package.tree(document.package.document_part()).find(_W + "body")
    assert all(effective.table_model(document, t) for t in body.findall(_W + "tbl"))
    document = Document.open(_document("tracked"))
    body = document.package.tree(document.package.document_part()).find(_W + "body")
    made = [effective.table_model(document, t) for t in body.findall(_W + "tbl")]
    assert made[1] is False and made[2] is False
    document = Document.open(_document("control"))
    body = document.package.tree(document.package.document_part()).find(_W + "body")
    nested = body.find(f"{_W}sdt/{_W}sdtContent/{_W}tbl")
    assert effective.table_model(document, nested) is False
