"""Lane P2d3: values written by reference into ``word_insert_markdown`` -- references in
``markdown`` (``{{d:notes/#1/body}}``) and ``from`` naming one field of one row -- are kept as
docx-agent's handler keeps them when the Rust session serves the call: the same call,
answered by the session and by the Python handler, leaves the same custom XML item (the
blocks a value standing alone made, the values found in a sentence) and lists the same
``data.facts`` and notes.  A value whose source holds Markdown of its own is written as it.

Made-up data only: the toy datasets of :mod:`test_tables_from_data` and
:mod:`test_dataset_markdown`."""

from __future__ import annotations

import pytest

from docx_agent.rust import edits
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry

from test_dataset_markdown import ds_rich
from test_served_records import FIXTURE, _serve_none
from test_served_written import _kept
from test_tables_from_data import ds_put


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox([*TOOLS, ds_put, ds_rich], formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


@pytest.fixture(scope="module")
def plain() -> bytes:
    return FIXTURE.read_bytes()


def _run(toolbox, data: bytes, calls: list[dict], served: bool) -> list[tuple[dict, object, list]]:
    """Each call's records, reported facts and notes after it, on a document the session owns."""
    session = toolbox.session()
    doc = session.open(data, name="fields.docx")
    entry = session.entry(doc)
    if not isinstance(entry, OwnedEntry):
        pytest.skip("no Rust session to own the document")
    assert toolbox.dispatch(session, "ds_put", {"id": "scores"}).ok
    assert toolbox.dispatch(session, "ds_rich", {}).ok
    out = []
    for arguments in calls:
        before = sum(edits.served().values())
        result = toolbox.dispatch(session, "word_insert_markdown", {"doc": doc, "at": "end", **arguments})
        assert result.ok, result.error
        assert (sum(edits.served().values()) > before) == served, edits.fallbacks()
        out.append((_kept(entry.document.to_bytes()), (result.data or {}).get("facts"), result.warnings))
    session.close(doc)
    return out


CASES = {
    "from a field with Markdown": [{"from": "d:notes/#1/body"}],
    "from a plain field": [{"from": "d:notes/#3/body"}],
    "from a field of two lines": [{"from": "d:scores/Gamma/note"}],
    "values standing alone": [{"markdown": "## Doors\n\n{{d:notes/#1/body}}\n\n## Roof\n\n{{d:notes/#2/body}}"}],
    "a value inside a sentence": [{"markdown": "Rated {{d:scores/Gamma/rating}} today\n\nand {{d:notes/#2/body}}"
                                               " here"}],
    "a value with Markdown at a line's start": [{"markdown": "{{d:notes/#2/body}} was typed"}],
    "a number alone": [{"markdown": "{{d:scores/Beta/rating}}"}],
    "between two tables": [{"markdown": "| a |\n|---|\n| 1 |\n\n{{d:scores/Alpha/check}}\n\n| b |\n|---|\n| 2 |"}],
    "two tables apart": [{"markdown": "| a |\n|---|\n| 1 |"},
                         {"markdown": "| b |\n|---|\n| {{d:scores/Alpha/check}} |\n\n{{d:notes/#3/body}}"}],
    "then a plain call": [{"markdown": "{{d:notes/#3/body}}"}, {"markdown": "Plain text"}],
}


@pytest.mark.parametrize("case", list(CASES))
def test_served_fields_are_kept_as_docx_agent_keeps_them(toolbox, plain, case, monkeypatch):
    calls = CASES[case]
    rust = _run(toolbox, plain, calls, served=True)
    _serve_none(monkeypatch)
    python = _run(toolbox, plain, calls, served=False)
    for k, ((ours, our_facts, our_notes), (theirs, their_facts, their_notes)) in enumerate(zip(rust, python)):
        assert our_facts == their_facts, (case, k)
        assert our_notes == their_notes, (case, k)
        for key in theirs:
            assert ours[key] == theirs[key], (case, k, key, ours[key], theirs[key])
    if case == "from a field with Markdown":
        assert rust[0][0]["item"] is not None and rust[0][1][0]["blocks"]


def test_rows_from_a_dataset_stay_the_hosts(toolbox, plain):
    """A table of rows (``from: d:scores``) is still answered by the Python handler."""
    session = toolbox.session()
    doc = session.open(plain, name="fields.docx")
    if not isinstance(session.entry(doc), OwnedEntry):
        pytest.skip("no Rust session to own the document")
    assert toolbox.dispatch(session, "ds_put", {"id": "scores"}).ok
    before = sum(edits.served().values())
    assert toolbox.dispatch(session, "word_insert_markdown", {"doc": doc, "at": "end", "from": "d:scores"}).ok
    assert sum(edits.served().values()) == before
    session.close(doc)
