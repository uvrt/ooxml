"""Lane P2c14: ``word_move``, ``word_sections``, ``word_notes`` and ``word_links`` served by
the Rust session; their replies as docx-agent's own handler gives them (a session with its
own clock is answered by the Python handler), refs they define and use included.  Made-up
documents only."""

from __future__ import annotations

import datetime as dt

import pytest

from docx_agent import Document
from docx_agent.rust import edits
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry

CLOCK = lambda: dt.datetime(2026, 11, 20, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


@pytest.fixture(scope="module")
def report() -> bytes:
    doc = Document.new()
    parts = ["# A made-up report"]
    for c in range(1, 4):
        parts += [f"# Part {c}", f"Lantern meadow {c} by the harbour.", f"River stone number {c}."]
    doc.insert_markdown("\n\n".join(parts))
    return doc.to_bytes()


def _open(toolbox, data: bytes, served: bool):
    """A session that serves (the system's clock) or one the Python handler answers (its own)."""
    session = toolbox.session() if served else toolbox.session(clock=CLOCK)
    doc = session.open(data, "report.docx")
    if served and not isinstance(session.entry(doc), OwnedEntry):
        pytest.skip("no Rust session to own the document")
    return session, doc


def _calls(toolbox, data: bytes, served: bool, calls) -> list:
    session, doc = _open(toolbox, data, served)
    out = []
    for tool, arguments in calls(session, doc):
        before = dict(edits.served())
        result = toolbox.dispatch(session, tool, {"doc": doc, **arguments})
        assert result.ok, result.error
        if served:
            assert edits.served().get(tool, 0) == before.get(tool, 0) + 1, edits.fallbacks()
        out.append((tool, result.summary, result.data, sorted(result.changed), result.warnings,
                    dict(result.refs)))
    return out


def _ids(session, doc) -> dict[str, str]:
    """Each paragraph's id by its text."""
    return {p.text: p.id for p in session.entry(doc).document.paragraphs()}


def _structure_calls(session, doc):
    ids = _ids(session, doc)
    # a heading's section moved before another's, the side said in after (and a ref there)
    yield "word_move", {"section_heading": ids["Part 3"], "after": f"before:{ids['Part 1']}"}
    yield "word_sections", {"action": "insert_break", "after": ids["River stone number 1."]}
    yield "word_sections", {"action": "list"}
    yield "word_sections", {"action": "set", "section": "s:body", "orientation": "landscape",
                            "margins": {"left": 54}}
    yield "word_notes", {"action": "insert", "find": "meadow 2", "text": "A made-up note.", "ref": "note"}
    yield "word_notes", {"action": "edit", "note": "$note", "text": "The note, edited."}
    yield "word_notes", {"action": "insert", "kind": "endnote", "at": ids["River stone number 3."],
                         "text": "An endnote."}
    yield "word_notes", {"action": "list"}
    yield "word_links", {"action": "add_bookmark", "target": ids["Part 2"], "bookmark": "part_two"}
    yield "word_links", {"action": "add_hyperlink", "find": "meadow 1 by", "bookmark": "part_two"}
    yield "word_links", {"action": "add_hyperlink", "find": "River stone number 2",
                         "url": "https://example.com/stones"}
    yield "word_links", {"action": "rename_bookmark", "bookmark": "part_two", "new_name": "second_part"}
    yield "word_links", {"action": "list"}


def test_structure_tools_served_as_the_python_handler_answers(toolbox, report):
    served = _calls(toolbox, report, True, _structure_calls)
    assert served == _calls(toolbox, report, False, _structure_calls)


def test_a_ref_a_served_call_defines_is_the_sessions(toolbox, report):
    session, doc = _open(toolbox, report, True)
    ids = _ids(session, doc)
    result = toolbox.dispatch(session, "word_notes", {"doc": doc, "action": "insert", "at": ids["Part 1"],
                                                      "text": "A made-up note.", "ref": "first"})
    assert result.ok, result.error
    assert session.refs(doc)["first"] == result.refs["first"]
    moved = toolbox.dispatch(session, "word_move", {"doc": doc, "blocks": ids["River stone number 1."],
                                                    "after": "end"})
    assert moved.ok, moved.error
    assert session.entry(doc).document.paragraphs()[-1].text == "River stone number 1."


# -- the reads: word_compare_blocks (a reference in another document) and word_design_facts --


def _tables(toolbox, session) -> tuple[str, str, str, str]:
    """Two made-up documents with a table each, the second's header row bold."""
    table = "| Name | Value |\n|---|---|\n| Lantern | 1 |\n| Meadow | 2 |\n"
    docs, ids = [], []
    for k in range(2):
        doc = session.open(Document.new().to_bytes(), f"tables{k}.docx")
        made = toolbox.dispatch(session, "word_insert_markdown", {"doc": doc, "at": "end", "markdown": table})
        assert made.ok, made.error
        docs.append(doc)
        ids.append(session.entry(doc).document.tables()[0].id)
    bold = toolbox.dispatch(session, "word_format", {"doc": docs[1], "targets": [f"{ids[1]}/c1,1"], "bold": True})
    assert bold.ok, bold.error
    return docs[0], ids[0], docs[1], ids[1]


def _reads(toolbox, session, mine, table, theirs, reference) -> list:
    out = []
    for tool, arguments in (
            ("word_compare_blocks", {"doc": mine, "blocks": [table], "reference": {"doc": theirs, "block": reference}}),
            ("word_compare_blocks", {"doc": theirs, "blocks": [reference], "reference": {"block": reference}}),
            ("word_design_facts", {"doc": mine}),
            ("word_design_facts", {"doc": theirs, "include": ["table_styles", "declared_styles"]})):
        result = toolbox.dispatch(session, tool, arguments)
        assert result.ok, result.error
        out.append((tool, result.summary, result.data, result.total, result.next_cursor, result.warnings))
    return out


def test_the_reads_served_with_another_documents_version(toolbox, monkeypatch):
    from docx_agent.rust import reads

    session = toolbox.session()
    mine, table, theirs, reference = _tables(toolbox, session)
    if not isinstance(session.entry(mine), OwnedEntry):
        pytest.skip("no Rust session to own the document")
    before = dict(reads.served())
    served = _reads(toolbox, session, mine, table, theirs, reference)
    assert {tool: reads.served().get(tool, 0) - before.get(tool, 0)
            for tool in ("word_compare_blocks", "word_design_facts")} == {
        "word_compare_blocks": 2, "word_design_facts": 2}, reads.fallbacks()
    assert served[0][2]["differences"], "the bold header row is a difference"
    monkeypatch.setattr(reads, "_session", lambda: None)     # the Python path answers
    assert served == _reads(toolbox, session, mine, table, theirs, reference)
