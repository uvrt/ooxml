"""Lane H7d: the docx format's provenance hook for a document the Rust session owns reads the
session: the records of placed values, from the session's reading of the version.  (Since the
cut, P4e, the session's tools keep their own records: the format stores none.)
"""

from __future__ import annotations

import pytest

ooxml_core = pytest.importorskip("ooxml_core")

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from ooxml_edit.tools import OwnedDocument, Toolbox  # noqa: E402

import test_tables_from_data as data  # noqa: E402
from test_tables_from_data import call, ds_put, ok  # noqa: E402


@pytest.fixture(scope="module")
def box():
    with Toolbox([*TOOLS, ds_put], formats=[FORMAT, data.GRID], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _placed(box):
    session = data._session(box)
    ok(call(box, session, "ds_put", id="scores"))
    table = ok(call(box, session, "word_insert_markdown", doc="d1", at="end",
                    **{"from": "d:scores", "from_columns": ["check", "rating"]})).data["id"]
    return session, table


def test_the_records_of_an_owned_document_are_the_session_s(box):
    session, table = _placed(box)
    subject = session.entry("d1").subject
    assert isinstance(subject, OwnedDocument)
    kept = session.provenance("d1")
    assert list(kept) == [table] and kept[table]["source"] == "d:scores"
    assert FORMAT.read_provenance(subject) == kept
    ok(call(box, session, "word_edit_table", doc="d1", table=table, action="set_cells",
            **{"from": "d:scores", "from_columns": ["check", "rating"]}))
    assert list(session.provenance("d1")) == [table]
