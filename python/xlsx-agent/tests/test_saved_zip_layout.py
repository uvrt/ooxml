"""A saved workbook answers as a native Excel client would on a co-authoring host (the
design page "CyberAI as an Office Client", requests 1, 2, 3 and 6): every made-up workbook the
tests build, saved without an edit, is its bytes; every save chunks per MS-FSSHTTPD 2.4.1 (no
data descriptors, no needless ZIP64, the input's entry order); and one edit changes only the
parts its ``Result.changed_parts`` names."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

from filter_book import pivot_book, stock_book
from notes_book import plain, threaded
from ooxml_edit.tools import Toolbox
from shelf_book import shelf_book
from test_catalogue import shop, warehouse
from xlsx_agent import Workbook
from xlsx_agent.tools import FORMAT, GROUPS, TOOLS

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "ooxml-common" / "tests" / "support"))
from zipchunks import (changed_entries, entry_names, input_order, layout_problems,  # noqa: E402
                       office_chunked)

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
    assert layout_problems(saved) == []
    assert entry_names(saved) == input_order(data)
    if office_chunked(data):
        assert saved == data


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
    assert layout_problems(edited) == []
    names = entry_names(edited)
    assert names[:len(input_order(data))] == input_order(data)
    assert changed_entries(data, edited) <= {p["part"] for p in made.changed_parts}
