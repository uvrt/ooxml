"""Lane H7a: the docx format's hooks for a document the Rust session owns read the session.

ooxml-edit gives the hooks an ``OwnedDocument`` (the owner, its handle, the version) for such a
document -- never the entry or its Python view -- and each answers from the session's reading
of the version: ``summary``, ``problems``, ``state``, ``closed`` and ``restored``.  Each answer
equals what docx-agent's Python gives for the same version: the summary and the problems as
they are, the id state as the same decoded record (the deflate bytes may differ between
Rust's zlib and C's; neither reader minds, and each side opens the other's state).
"""

from __future__ import annotations

import base64
import json
import zlib
from pathlib import Path

import pytest

ooxml_core = pytest.importorskip("ooxml_core")

from docx_agent import Document  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from docx_agent.tools import format as docx_format  # noqa: E402
from ooxml_edit.tools import OwnedDocument, Toolbox  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]
#: The repository\'s Word fixtures, with the oracle (as conftest.FIXTURE_DIR).
FIXTURES = ROOT / "crates" / "ooxml-oracle" / "fixtures"
LISTED = sorted(json.loads((ROOT / "tools" / "gates_fixtures.json").read_text()))


@pytest.fixture
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def _owned(session, doc):
    entry = session.entry(doc)
    if not entry.owned:
        pytest.skip("no Rust session owns the document here")
    subject = entry.subject
    assert isinstance(subject, OwnedDocument)
    return entry, subject


def _record(state: str) -> dict:
    """An id state's record: what it says, whatever bytes deflate wrote."""
    prefix = "docx-ids.1."
    assert state.startswith(prefix)
    body = state[len(prefix):]
    return json.loads(zlib.decompress(base64.urlsafe_b64decode(body + "=" * (-len(body) % 4))))


def _report() -> bytes:
    document = Document.new()
    document.insert_markdown("# Part one\n\n" + "\n\n".join(f"Text {k}." for k in range(6))
                             + "\n\n| a | b |\n|---|---|\n| 1 | 2 |")
    return document.to_bytes()


@pytest.mark.parametrize("name", LISTED)
def test_summary_and_problems_of_an_owned_document_are_the_python_ones(box, name):
    data = (FIXTURES / name).read_bytes()
    session = box.session()
    doc = session.open(data, Path(name).name)
    _entry, subject = _owned(session, doc)
    python = Document.open(data)
    assert FORMAT.summary(subject) == docx_format.summary(python)
    key = FORMAT.problem_key
    assert sorted(map(key, FORMAT.problems(subject))) == sorted(map(key, docx_format.problems(python)))


def _ledger(data: bytes, state: str) -> dict:
    """What a state gives a document opened with it: every id it had, where each seed's
    draws got to, what is gone and by which change, the renames and the versions."""
    opened = Document.open(data, ids=state)
    ledger = opened._ledger
    return {"pool": set(ledger.pool), "cursors": {s: k for s, k in ledger.pool.cursors.items() if k},
            "gone": dict(ledger.gone), "drawings_gone": dict(ledger.drawings_gone),
            "replaced": dict(ledger.replaced), "earlier": dict(ledger.earlier),
            "aliases": dict(opened._aliases), "highest": ledger.highest_drawing,
            "versions": (opened.history.version, opened.history.last_version)}


def test_the_state_of_an_owned_document_says_what_the_view_s_says_and_each_opens_the_other(box):
    session = box.session()
    doc = session.open(_report(), "report.docx")
    writer = session.view(context={"agent": "writer-2"})
    entry, subject = _owned(session, doc)
    blocks = [p.id for p in entry.document.paragraphs()]
    assert box.dispatch(writer, "word_delete", {"doc": doc, "target": blocks[3]}).ok
    assert box.dispatch(session, "word_set_text", {"doc": doc, "target": blocks[2], "text": "Changed."}).ok
    made = box.dispatch(session, "word_insert_markdown", {"doc": doc, "markdown": "More.", "at": "end"})
    assert made.ok, made.to_json()
    rust = FORMAT.state(entry.subject)
    python = entry.document.id_state()
    data = session.save(doc).data
    ours, theirs = _record(rust), _record(python)
    # The same record but for where the seeds' draws got to: the session drew the new ids,
    # so it knows its cursors (and places a deleted id by its seed); the view, which drew
    # none, lists every id as a value.  Opened, the two give the same document.
    assert {k: v for k, v in ours.items() if k not in ("seeds", "gone")} == \
        {k: v for k, v in theirs.items() if k not in ("seeds", "gone")}
    assert theirs["seeds"] == [] and ours["seeds"]
    rust_ledger, python_ledger = _ledger(data, rust), _ledger(data, python)
    assert rust_ledger.pop("cursors") and not python_ledger.pop("cursors")
    # The session's pool also holds ids it drew that no part holds now, which the view
    # (told the ids written) never learned: retired too, so never given again.
    ours_pool, theirs_pool = rust_ledger.pop("pool"), python_ledger.pop("pool")
    assert ours_pool >= theirs_pool
    assert rust_ledger == python_ledger
    assert rust_ledger["versions"] == (entry.version, entry.version)
    # either side reads the other's
    later = box.session()
    assert later.open(data, "report.docx", doc_id=doc, state=python) == doc
    other = box.session()
    assert other.open(data, "report.docx", doc_id=doc, state=rust) == doc
    assert _record(FORMAT.state(other.entry(doc).subject)) == ours


def test_save_close_and_open_document_s_summary_never_read_the_view(box, monkeypatch):
    from ooxml_edit.tools.owned import OwnedEntry

    session = box.session()
    doc = session.open(_report(), "report.docx")
    entry, _subject = _owned(session, doc)

    def no_view(self):
        raise AssertionError("a format hook read the Python view")

    monkeypatch.setattr(OwnedEntry, "document", property(no_view))
    saved = session.save(doc)
    assert saved.state
    summary = FORMAT.summary(entry.subject)
    assert summary["tables"] == 1 and summary["headings"] == 1 and summary["template"] is False
    session.close(doc)


def test_a_served_check_never_reaches_the_python_layout_paths(box, monkeypatch):
    """Q3 of the design: read_version's and checks' Python-layout parts (deleted with the
    view) are never reached by a check the session serves."""
    from docx_agent.tools import _base, owner

    def unreachable(*_args, **_kwargs):
        raise AssertionError("a served check reached a Python layout path")

    monkeypatch.setattr(owner, "_engine", unreachable)
    monkeypatch.setattr(_base, "_reflow", unreachable)
    session = box.session()
    doc = session.open(_report(), "report.docx")
    _owned(session, doc)
    checked = box.dispatch(session, "check", {"doc": doc, "include": ["validate"]})
    assert checked.ok, checked.to_json()
