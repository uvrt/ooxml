"""The main part written out from its last bytes (lane J, 0.1.120): only the body's children
that changed are written again, in place; the rest are the last bytes'.  The bytes are the
tree's, written out whole, in every case (the suite's ``OOXML_EDIT_VERIFY_SERIALIZED=1``
compares each write too); where the part cannot be written so, it is written whole."""

from __future__ import annotations

import pytest
from lxml import etree

from ooxml_edit.xml import serialize

from docx_agent import Document, incremental
from docx_agent.oxml import package as word_package

pytest.importorskip("ooxml_core")
if word_package._core() is None:  # pragma: no cover -- an older core
    pytest.skip("ooxml-core without tree_digests", allow_module_level=True)

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"


def _document() -> Document:
    document = Document.new()
    document.paragraphs()[0].set_text("A made-up report")
    document.insert_markdown("# One\n\nSome text here.\n\n- a list\n- of two\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n"
                             "# Two\n\nMore text & signs: 1 &lt; 2, \"quoted\".\n\nThe end.")
    return document


def _written(document: Document) -> bytes:
    """The main part's bytes as a read writes them, checked against the tree written whole."""
    package = document.package
    part = package.document_part()
    package.mark_dirty(part)
    data = package.read(part)
    assert data == serialize(package.tree(part))
    return data


def _counts() -> tuple[int, int]:
    return incremental.STATS["write: per block"], incremental.STATS["write: whole"]


def test_edits_undo_and_redo_are_written_per_block():
    document = _document()
    _written(document)                              # the first write is whole
    per_block, whole = _counts()
    paragraphs = document.paragraphs()
    document.paragraph(paragraphs[2].id).set_text("Changed text, with & and <.")
    _written(document)
    document.insert_markdown("Inserted after.\n\n1. one\n2. two", at=f"after:{paragraphs[3].id}")
    _written(document)
    document.paragraph(paragraphs[1].id).delete()
    _written(document)
    document.undo()
    _written(document)
    document.redo()
    _written(document)
    assert _counts()[0] >= per_block + 5
    assert _counts()[1] == whole


def test_a_moved_block_is_taken_from_the_last_bytes():
    document = _document()
    _written(document)
    body = document.package.tree(document.package.document_part()).find(W + "body")
    moved = body[1]
    body.remove(moved)
    body.insert(4, moved)
    per_block, whole = _counts()
    _written(document)
    assert _counts() == (per_block + 1, whole)


def test_a_block_declaring_a_namespace_of_its_own_is_written_whole():
    document = _document()
    _written(document)
    body = document.package.tree(document.package.document_part()).find(W + "body")
    own = etree.SubElement(body, W + "p", nsmap={"x": "urn:made-up"})
    own.set("{urn:made-up}mark", "1")
    body.remove(own)
    body.insert(1, own)
    whole = _counts()[1]
    _written(document)
    assert _counts()[1] == whole + 1


def test_a_comment_in_the_body_and_a_changed_root_are_written_whole():
    document = _document()
    _written(document)
    root = document.package.tree(document.package.document_part())
    body = root.find(W + "body")
    whole = _counts()[1]
    root.set(W + "conformance", "transitional")
    _written(document)
    assert _counts()[1] == whole + 1
    body.insert(0, etree.Comment(" a remark "))
    _written(document)
    body[1].set(W + "rsidR", "00AB12CD")
    _written(document)
    assert _counts()[1] == whole + 3


def test_whole_when_switched_off(monkeypatch):
    document = _document()
    _written(document)
    monkeypatch.setenv("DOCX_AGENT_INCREMENTAL", "0")
    document.paragraph(document.paragraphs()[2].id).set_text("Off.")
    per_block, whole = _counts()
    _written(document)
    assert _counts() == (per_block, whole)


def test_whole_without_the_core(monkeypatch):
    document = _document()
    _written(document)
    monkeypatch.setattr(word_package, "_core", lambda: None)
    document.paragraph(document.paragraphs()[2].id).set_text("No core.")
    per_block = _counts()[0]
    _written(document)
    assert _counts()[0] == per_block
