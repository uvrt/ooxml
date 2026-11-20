"""Lane P2c19: ``word_edit_table`` set_cells, insert_row and copy_row answered by the Rust
session with round 58's R126 step -- the rows written kept for check's
``row_text_format`` (one record, the session's), text in a cell that was empty given its
column's formatting, a row unlike the rows around it said -- as the Python handler answers
them (``test_rows_r58``'s made-up "Shelf" tables, the session's own clock so that the
session serves).
"""

from __future__ import annotations

import pytest
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry

from docx_agent.rust import edits
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from docx_agent.tools.objects import ROWS_WRITTEN
from test_rows_r58 import ROWS, _look, _rows, _shelf


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


def _run(toolbox, monkeypatch, data: bytes, served: bool, calls) -> tuple[list, list, list]:
    """Each call's warnings, the looks of the cells named, and check's row_text_format;
    ``served`` false: word_edit_table taken out of the session's tools."""
    with monkeypatch.context() as patched:
        if not served:
            patched.setattr(edits, "SERVED", {k: v for k, v in edits.SERVED.items() if k != "word_edit_table"})
        session = toolbox.session()
        doc = session.open(data, "shelf.docx")
        if not isinstance(session.entry(doc), OwnedEntry):
            pytest.skip("no Rust session to own the document")
        document = session.entry(doc).document
        warnings = []
        for arguments in calls:
            table = document.tables()[0].id
            before = edits.served().get("word_edit_table", 0)
            result = toolbox.dispatch(session, "word_edit_table", {"doc": doc, "table": table, **arguments})
            assert result.ok, result.to_json()
            assert edits.served().get("word_edit_table", 0) - before == (1 if served else 0), edits.fallbacks()
            now = session.entry(doc).document.tables()[0].id
            warnings.append([w.replace(now, "<t>").replace(table, "<t>") for w in result.warnings])
        document = session.entry(doc).document
        table = document.tables()[0].id
        count = len(_rows(document))
        looks = [_look(document, f"{table}/c{r},{c}") for r in range(1, count) for c in (0, 2)]
        check = toolbox.dispatch(session, "check", {"doc": doc, "include": ["layout"]})
        assert check.ok, check.to_json()
        found = check.data["layout"]["row_text_format"]
        written = sorted(document.__dict__.get(ROWS_WRITTEN) or ())
        return warnings, looks, [found, len(written)]


CASES = {
    "empty cells take their column": (
        [ROWS[0], ROWS[1], None, ROWS[2]],
        None,
        [{"action": "set_cells", "cells": [{"row": 3, "col": 0, "text": "Rug"}, {"row": 3, "col": 2, "text": "7"}]}]),
    "a red row is said with its call": (
        ROWS, ["1F4E79", "1F4E79", "C00000", "1F4E79"],
        [{"action": "set_cells", "cells": [{"row": 3, "col": 0, "text": "Rug"}]}]),
    "one after another": (
        [ROWS[0], None, None, ROWS[1]], None,
        [{"action": "set_cells", "cells": [{"row": 2, "col": 0, "text": "Rug"}]},
         {"action": "set_cells", "cells": [{"row": 3, "col": 0, "text": "Vase"}]}]),
    "rows added": (
        ROWS, None,
        [{"action": "insert_row", "index": 2}, {"action": "copy_row", "index": 1, "side": "before"},
         {"action": "set_cells", "cells": [{"row": 3, "col": 0, "text": "Rug"}]}]),
}


@pytest.mark.parametrize("name", list(CASES))
def test_rows_written_served_as_the_python_handler_answers(toolbox, monkeypatch, name):
    rows, colors, calls = CASES[name]
    data = _shelf(rows, colors=colors)
    assert _run(toolbox, monkeypatch, data, True, calls) == _run(toolbox, monkeypatch, data, False, calls)


def test_the_python_handlers_rows_go_into_the_sessions_record(toolbox):
    """A row the Python handler writes (a call it answers) is in the session's record, which
    check reads: one record for both."""
    session = toolbox.session()
    doc = session.open(_shelf([ROWS[0], None, ROWS[1]]), "shelf.docx")
    entry = session.entry(doc)
    if not isinstance(entry, OwnedEntry):
        pytest.skip("no Rust session to own the document")
    record = entry.document.__dict__[ROWS_WRITTEN]
    record.update(["0A0B0C0D"])
    assert "0A0B0C0D" in record and len(record) == 1
