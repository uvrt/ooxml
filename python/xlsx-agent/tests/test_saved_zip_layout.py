"""A saved workbook answers as a native Excel client would on a co-authoring host (the
design page "CyberAI as an Office Client", requests 1, 2, 3 and 6): every made-up workbook the
tests build, saved without an edit, is its bytes; an untouched entry is copied byte for byte,
a data descriptor too, and a save adds no layout problem MS-FSSHTTPD 2.4.1's chunking meets
(data descriptors, needless ZIP64) and keeps the input's entry order; one edit changes only
the parts its ``Result.changed_parts`` names.  The shared chunk writer
(``ooxml_edit.chunks.chunk_file``, request 4) chunks every save by ZIP analysis, as the tests' own
check (``zipchunks``) does, and a one-cell save sends only the changed parts' chunks and the
central directory."""

from __future__ import annotations

import sys
from pathlib import Path

from ooxml_edit.chunks import chunk_file
import pytest

from filter_book import pivot_book, stock_book
from notes_book import plain, threaded
from ooxml_edit.tools import Toolbox
from shelf_book import shelf_book
from test_catalogue import shop, warehouse
from xlsx_agent import Workbook
from xlsx_agent.tools import FORMAT, GROUPS, TOOLS

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "ooxml-common" / "tests" / "support"))
from zipchunks import (changed_entries, entry_records, has_directories,  # noqa: E402
                       input_order, new_problems, with_data_descriptors, zip_chunks)

BOOKS = {
    "shop": lambda: shop().to_bytes(),
    "warehouse": lambda: warehouse().to_bytes(),
    "shelf": lambda: shelf_book().to_bytes(),
    "stock": stock_book,
    "pivot": pivot_book,
    "notes": plain,
    "threaded": threaded,
}


@pytest.mark.parametrize("name", BOOKS)
def test_a_save_chunks_as_office_chunks(name):
    data = BOOKS[name]()
    saved = Workbook.open(data).to_bytes()
    assert new_problems(data, saved) == []
    assert input_order(saved) == input_order(data)
    if not has_directories(data):
        assert saved == data
    chunks = chunk_file(saved)
    assert chunks.zip_analysed
    assert [(c.offset, c.size, None if c.random else c.signature) for c in chunks.chunks] == [
        (c.start, c.size, c.signature) for c in zip_chunks(saved)]


@pytest.mark.parametrize("name", ["shop", "stock", "notes"])
def test_one_edit_changes_the_parts_it_names(name):
    data = BOOKS[name]()
    sheet = Workbook.open(data).sheets[0].name
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        session.open(data, f"{name}.xlsx")
        made = box.dispatch(session, "xlsx_set_cells", {"doc": "d1", "cells": [
            {"address": f"'{sheet}'!Z90", "number": 7}]})
        assert made.ok, made.to_json()
        assert any(p["part"].startswith("xl/worksheets/") for p in made.changed_parts)
        assert "changed_parts" not in made.to_json()
        saved = box.dispatch(session, "save_document", {"doc": "d1", "name": "out.xlsx",
                                                        "format": "xlsx"})
        assert saved.ok and saved.changed_parts == []
        edited = session.outputs[-1].data
    assert new_problems(data, edited) == []
    assert input_order(edited)[:len(input_order(data))] == input_order(data)
    assert changed_entries(data, edited) <= {p["part"] for p in made.changed_parts}
    chunks = chunk_file(edited)
    delta = chunks.delta(chunk_file(data).leaves())
    travel = {chunks.chunks[i].name or chunks.chunks[i].kind for i in delta.new}
    assert travel == changed_entries(data, edited) | {"final"}


def test_an_entry_with_a_data_descriptor_is_copied_as_it_is():
    """A made-up workbook whose every entry has a data descriptor: saved without an edit it
    is its bytes; after one cell every entry the edit did not change keeps its record,
    descriptor and all."""
    data = with_data_descriptors(BOOKS["shop"]())
    assert Workbook.open(data).to_bytes() == data
    sheet = Workbook.open(data).sheets[0].name
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        session.open(data, "shop.xlsx")
        made = box.dispatch(session, "xlsx_set_cells", {"doc": "d1", "cells": [
            {"address": f"'{sheet}'!Z90", "number": 7}]})
        assert made.ok, made.to_json()
        assert box.dispatch(session, "save_document", {"doc": "d1", "name": "out.xlsx",
                                                       "format": "xlsx"}).ok
        edited = session.outputs[-1].data
    before, after = entry_records(data), entry_records(edited)
    changed = {name for name in before if after.get(name) != before[name]}
    assert changed == {p["part"] for p in made.changed_parts}
