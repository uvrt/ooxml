"""Lane H7c: a Word document opened in a toolbox is opened by the Rust session alone.

``Session.open`` hands the bytes, and the id state saved beside them, to the format's owner
(``RustOwner.open_data``): the session reads the story parts once, its ids against the state,
the ids the state retired and who retired them, and the versions it was saved at; no Python
``Document`` is parsed and the entry holds none.  Refusals are ``Document.open``'s: a state
that is not these bytes' (``StateRefused``, the way out in docx-agent's words), a package that
is not a Word document; a template warns ``TemplateOpened``.
"""

from __future__ import annotations

from pathlib import Path

import pytest

ooxml_core = pytest.importorskip("ooxml_core")

from docx_agent import Document  # noqa: E402
from docx_agent.edit.authoring import TemplateOpened  # noqa: E402
from docx_agent.oxml.package import WordPackage  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from ooxml_edit.tools import StateRefused, Toolbox  # noqa: E402

TEMPLATE = Path(__file__).parent / "fixtures" / "generated" / "templates" / "brand.dotx"


@pytest.fixture
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


@pytest.fixture
def no_python_parse(monkeypatch):
    """Opening a document never parses it in Python (``Document.open``, ``WordPackage.open``)."""

    def refused(*_args, **_kwargs):
        raise AssertionError("an open parsed the document in Python")

    monkeypatch.setattr(Document, "open", classmethod(refused))
    monkeypatch.setattr(WordPackage, "open", classmethod(refused))


def _saved_with_deletions(later: bool = False) -> tuple[bytes, str, list[str], tuple[int, int]]:
    """A document whose latest blocks were deleted (the file alone cannot tell; one undone and
    redone), its state, the deleted ids and its versions; ``later``: a paragraph appended
    after the state was taken (the bytes are no longer the state's)."""
    document = Document.new()
    document.insert_markdown("\n\n".join(f"Line {k}." for k in range(8)))
    gone = [p.id for p in document.paragraphs()[-4:]]
    for identifier in gone:
        document.delete_block(identifier)
    document.undo()
    document.redo()
    state = document.id_state()
    if later:
        document.append_paragraph("Later.")
    history = document.history
    return document.to_bytes(), state, gone, (history.version, history.last_version)


@pytest.fixture
def saved():
    return _saved_with_deletions()


@pytest.fixture
def later():
    return _saved_with_deletions(later=True)


def _owned(session, doc):
    entry = session.entry(doc)
    if not entry.owned:
        pytest.skip("no Rust session owns the document here")
    return entry


def test_an_open_holds_no_python_document(box, saved, no_python_parse):
    data = saved[0]
    session = box.session()
    doc = session.open(data, "report.docx")
    entry = _owned(session, doc)
    assert entry.held is None
    assert FORMAT.summary(entry.subject)["paragraphs"] == 4
    assert entry.baseline_problems == list(FORMAT.problems(entry.subject))
    session.close(doc)


def test_the_state_s_retired_ids_and_versions_are_the_session_s(box, saved, no_python_parse):
    data, state, gone, (version, last) = saved
    session = box.session()
    doc = session.open(data, "report.docx", state=state)
    entry = _owned(session, doc)
    assert entry.held is None
    assert (entry.history.version, entry.history.last_version) == (version, last)
    mirror = entry.handle
    retired = {f"{value:08X}" for kind, value, *_ in mirror.session.retired(mirror.doc) if kind == "block"}
    assert retired >= {identifier.split(":")[-1] for identifier in gone}
    # the state the session gives back opens the bytes again, the same record
    again = FORMAT.state(entry.subject)
    reopened = box.session()
    assert reopened.entry(reopened.open(data, "report.docx", state=again)).version == version


def test_a_state_of_other_bytes_or_none_is_refused_with_the_way_out(box, saved, later, no_python_parse):
    data, state = saved[:2]
    session = box.session()
    with pytest.raises(StateRefused, match="of other bytes") as caught:
        session.open(later[0], "other.docx", state=state)
    assert "without ids=" in str(caught.value) and "Session.save" in str(caught.value)
    for wrong in ("nonsense", "docx-ids.1.AAAA", "docx-ids.9.xyz", 42):
        with pytest.raises(StateRefused, match="without ids="):
            session.open(data, "report.docx", state=wrong)
    assert session.entry(session.open(data, "report.docx", state="")).version == 0


def test_a_template_warns_and_a_package_without_a_document_is_refused(box, no_python_parse):
    session = box.session()
    with pytest.warns(TemplateOpened, match=r"this file is a template \(\.dotx\)"):
        session.open(TEMPLATE.read_bytes(), "brand.dotx", kind="docx")


def test_a_new_blank_document_is_the_session_s_alone(box, no_python_parse):
    session = box.session()
    made = box.dispatch(session, "new_document", {"kind": "docx", "title": "Plan"})
    assert made.ok, made.to_json()
    entry = _owned(session, made.created[0])
    assert entry.held is None
    assert made.data["summary"]["template"] is False
