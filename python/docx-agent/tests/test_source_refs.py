"""Values by reference and refs as callers name them, on the Rust session (round 50; round 47.
P4e, lane CUT-D: from ``test_block_sources.py`` and ``test_ref_owners_serial.py``, whose
drafts are gone).

* A reference without its prefix (``{{rows/#1/name}}``) is refused naming the one meant; a
  document's own braces that name no source, and escaped ones, stay text.
* A batch's ``replace_text`` without ``expect`` replaces one, as alone, and says so.
* A ref name another caller gave is refused before anything is made, with the free name.

Made-up documents and values only.
"""

from __future__ import annotations

import pytest
from ooxml_edit.tools import Dataset, DocumentFormat, Result, Toolbox, string, tool

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from word_docs import blank, element, paragraphs, text_of


@tool("ds_rows", "Test only: keep a made-up dataset of rows with a name.",
      {"id": string("Its id."), "rows": string("How many rows.")}, documents=())
def ds_rows(call, id, rows):
    names = ("alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel")
    call.define_dataset(id, Dataset(("name", "count"), [(names[k % len(names)], k) for k in range(int(rows))]),
                        source=None)
    return Result(summary=f"Kept d:{id}")


class Grid:
    """A made-up workbook's stand-in: ``t:stock`` is a table of three rows."""

    def __init__(self) -> None:
        self.tables = {"stock": (("item", "count", "note"),
                                 [("bolts", 12, "small"), ("nuts", 7, "round"), ("pins", 3, "thin")])}


def _table(grid: Grid, address: str) -> Dataset:
    header, rows = grid.tables[address[len("t:"):].partition("/")[0]]
    return Dataset(header, rows, keys=1)


GRID = DocumentFormat(kind="xlsx", open=lambda data: Grid(), detect=lambda data, name: False,
                      datasets={"t:": _table})


@pytest.fixture(scope="module")
def box():
    with Toolbox([*TOOLS, ds_rows], formats=[FORMAT, GRID], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


@pytest.fixture
def opened(box):
    """A session on a made-up note of three paragraphs, the dataset ``d:rows`` kept."""
    session = box.session()
    doc = session.open(blank(), "notes.docx")
    made = box.dispatch(session, "word_insert_markdown", {
        "doc": doc, "at": "end",
        "markdown": "Opening line of the note.\n\nMiddle line of the note.\n\nClosing line of the note."})
    assert made.ok, made.to_json()
    assert box.dispatch(session, "ds_rows", {"id": "rows", "rows": "6"}).ok
    session.adopt(Grid(), "xlsx", "stock.xlsx")                           # d2
    return session, doc, [block for block in made.created if block.startswith("p:")]


def _text(session, doc, block) -> str:
    return text_of(element(session, doc, block))


def test_a_reference_without_its_prefix_is_refused_naming_the_one_meant(box, opened):
    session, doc, blocks = opened
    result = box.dispatch(session, "word_insert_markdown", {"doc": doc, "at": "end",
                                                            "markdown": "Name: {{rows/#1/name}}"})
    assert not result.ok and result.error.code == "invalid_arguments"
    assert "did you mean {{d:rows/#1/name}}?" in result.error.message
    assert "\\{{rows/#1/name}}" in result.error.message and result.error.field == "markdown"
    handle = box.dispatch(session, "word_set_text", {"doc": doc, "target": blocks[0], "text": "{{d2/#1/count}}"})
    assert not handle.ok and "{{d2/t:<table>/#1/count}}" in handle.error.message


def test_a_documents_own_braces_and_escaped_ones_stay_text(box, opened):
    session, doc, blocks = opened
    target = blocks[0]
    written = box.dispatch(session, "word_set_text", {"doc": doc, "target": target,
                                                      "text": "Dear {{name}}, see {{TODO: a/b}}."})
    assert written.ok and written.warnings == []
    assert _text(session, doc, target) == "Dear {{name}}, see {{TODO: a/b}}."
    replaced = box.dispatch(session, "replace_text", {"doc": doc, "find": "{{name}}", "replace": "Ann",
                                                      "expect": "one"})
    assert replaced.ok and _text(session, doc, target) == "Dear Ann, see {{TODO: a/b}}."
    escaped = box.dispatch(session, "word_insert_markdown", {"doc": doc, "at": "end",
                                                             "markdown": "Literal \\{{rows/#1/name}} here."})
    assert escaped.ok, escaped.to_json()
    assert "Literal {{rows/#1/name}} here." in [text for _, text in paragraphs(session, doc)]


def test_replace_text_in_a_batch_without_expect_replaces_one_as_alone(box, opened):
    session, doc, blocks = opened
    target = blocks[0]
    assert box.dispatch(session, "word_set_text", {"doc": doc, "target": target, "text": "One fox, two foxes."}).ok
    batched = box.dispatch(session, "batch", {"ops": [
        {"tool": "replace_text", "arguments": {"doc": doc, "find": "One", "replace": "A"}}]})
    assert batched.ok and "ops[0]: expect not given: read as 'one'" in batched.warnings
    assert _text(session, doc, target) == "A fox, two foxes."
    several = box.dispatch(session, "batch", {"ops": [
        {"tool": "replace_text", "arguments": {"doc": doc, "find": "fox", "replace": "cat"}}]})
    assert "2" in str(several.to_json()) and _text(session, doc, target) == "A fox, two foxes."


def test_a_name_another_agent_gave_is_refused_before_anything_is_made(box):
    session = box.session()
    doc = session.open(blank(), "notes.docx")
    first = paragraphs(session, doc)[0][0]
    assert box.dispatch(session, "word_set_text", {"doc": doc, "target": first, "text": "Made-up notes"}).ok
    one, two = session.view(context={"agent": "one"}), session.view(context={"agent": "two"})
    made = box.dispatch(one, "word_insert_markdown", {"doc": doc, "at": "end", "markdown": "First", "ref": "table"})
    assert made.ok, made.to_json()
    version = session.save(doc).version
    refused = box.dispatch(two, "word_insert_markdown", {"doc": doc, "at": "end", "markdown": "Second",
                                                         "ref": "table"})
    assert not refused.ok and refused.error.field == "ref" and refused.error.valid_options == ["table_2"]
    assert "another caller" in refused.error.message and "agent" not in str(refused.to_json())
    assert session.save(doc).version == version         # nothing made
    used = box.dispatch(two, "word_insert_markdown", {"doc": doc, "at": "after:$table", "markdown": "Second",
                                                      "ref": "table_2"})
    assert used.ok, used.to_json()
    assert session.read_refs(doc) == {"table": made.refs["table"], "table_2": used.refs["table_2"]}
    assert session.read_resolve_ref(doc, "$table_2") == used.refs["table_2"]
    texts = [text for _, text in paragraphs(session, doc)]
    assert texts == ["Made-up notes", "First", "Second"]
