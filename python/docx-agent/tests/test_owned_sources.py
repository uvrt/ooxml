"""Lane H7d: the docx format's source and provenance hooks for a document the Rust session owns
read the session, never the Python view.

ooxml-edit gives ``own_source``, ``source_hint`` and ``read_provenance`` the entry's subject (an
``OwnedDocument`` for an owned document): what a source value names there -- a table, a
paragraph, a content control or part of one; whether a ``d:N`` drawing is there -- and the
records of placed values are answered from the session's reading of the version, equal to
docx-agent's Python for the same version.  ``store_provenance`` for an owned document is refused
as ``internal``: the session's tools keep their own records, and a handler of docx-agent's that
places values (a table from data, a chart) hands the hook the document it edits.
"""

from __future__ import annotations

import dataclasses
import json
from pathlib import Path

import pytest

ooxml_core = pytest.importorskip("ooxml_core")

from docx_agent import Document  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from docx_agent.tools import format as docx_format  # noqa: E402
from ooxml_edit.tools import OwnedDocument, ToolError, Toolbox  # noqa: E402

import test_tables_from_data as data  # noqa: E402
from test_tables_from_data import call, ds_put, ok  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]
#: The repository\'s Word fixtures, with the oracle (as conftest.FIXTURE_DIR).
FIXTURES = ROOT / "crates" / "ooxml-oracle" / "fixtures"
LISTED = sorted(json.loads((ROOT / "tools" / "gates_fixtures.json").read_text()))


@pytest.fixture(scope="module")
def box():
    with Toolbox([*TOOLS, ds_put], formats=[FORMAT, data.GRID], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _owned(session, doc):
    entry = session.entry(doc)
    if not entry.owned:
        pytest.skip("no Rust session owns the document here")
    subject = entry.subject
    assert isinstance(subject, OwnedDocument)
    return subject


def _addresses(document: Document) -> list[str]:
    """Source values a refusal may meet in ``document``: its blocks' ids, parts of them, its
    content controls, positions and ids that name nothing."""
    out: list[str] = []
    for paragraph in document.paragraphs()[:6]:
        out += [paragraph.id, f"{paragraph.id}/#1"]
    for table in document.tables()[:3]:
        out += [table.id, f"{table.id}/#2", f"{table.id}/c1,1"]
    out += [control.id for control in document.content_controls()[:4]]
    out += ["p@body/0", "p@body/0/x", "t:0A0000FF", "p:0A0000FF", "cc:999999", "cc@body/0", "s:1"]
    return out


def _placed(box):
    session = data._session(box)
    ok(call(box, session, "ds_put", id="scores"))
    table = ok(call(box, session, "word_insert_markdown", doc="d1", at="end",
                    **{"from": "d:scores", "from_columns": ["check", "rating"]})).data["id"]
    return session, table


def test_the_records_of_an_owned_document_are_the_session_s(box, monkeypatch):
    session, table = _placed(box)
    subject = _owned(session, "d1")
    kept = session.provenance("d1")
    assert list(kept) == [table]
    python = Document.open(session.save("d1").data)
    assert kept == dict(python.provenance()) == docx_format.read_provenance(python)
    # the hook reads the session, never the view
    monkeypatch.setattr(Document, "provenance", lambda self: pytest.fail("read the Python view"))
    assert FORMAT.read_provenance(subject) == kept


def test_store_provenance_on_an_owned_document_is_an_internal_refusal(box):
    session, table = _placed(box)
    subject = _owned(session, "d1")
    with pytest.raises(ToolError) as refused:
        FORMAT.store_provenance(subject, table, None)
    assert refused.value.code == "internal"
    assert list(session.provenance("d1")) == [table]


def test_store_provenance_is_never_handed_the_owner_s_document():
    """Tables from data and charts are placed by docx-agent's handlers still (the session
    leaves rows as a table to the host): their records go into the document they edit, in
    their step, and the session reads them back.  The owner's handle never reaches the hook."""
    given: list[object] = []

    def spy(document, address, record):
        given.append(document)
        return docx_format.store_provenance(document, address, record)

    spied = dataclasses.replace(FORMAT, store_provenance=spy)
    with Toolbox([*TOOLS, ds_put], formats=[spied, data.GRID], groups=GROUPS, workers=0) as box:
        session, table = _placed(box)
        _owned(session, "d1")
        ok(call(box, session, "word_edit_table", doc="d1", table=table, action="set_cells",
                **{"from": "d:scores", "from_columns": ["check", "rating"]}))
        assert given and not any(isinstance(document, OwnedDocument) for document in given)
        assert list(session.provenance("d1")) == [table]
