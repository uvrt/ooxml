"""Phase 3 CF2: another editor's version of an open document coming in (co-authoring refresh,
``HostedSession.refresh``) through the facade.

What each test holds: their version comes in while six writers edit other paragraphs, the
changes feed shows it as an ordinary version, and its blocks keep the ids they were written
with; nothing changed here, the document saves as their file; a step here on a block they
changed is handed back, named with its tool, caller and blocks; an undo entangled with their
change is refused, naming the blocks and who changed them, while one apart from it is taken;
a base this session never gave is refused with the way out.  Made-up documents only; the
interleavings a test cannot force from Python are proved in Rust
(``ooxml-word-edit``'s ``foreign`` tests)."""

from __future__ import annotations

import threading

import pytest

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox, ToolError

import word_docs

#: How long the threaded test's calls may take in all before it calls them deadlocked.
DEADLINE = 90.0

WORDS = "lantern meadow river stone harbour quiet orchard copper valley signal".split()


@pytest.fixture
def opened():
    """``(toolbox, session, doc, ids)``: a made-up document of eight paragraphs, open."""
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(word_docs.blank(), "shared.docx")
        markdown = "\n\n".join(" ".join(WORDS[(k + j) % len(WORDS)] for j in range(6)) for k in range(8))
        word_docs.call(box, session, "word_insert_markdown", doc=doc, at="end", markdown=markdown)
        ids = [block for block, text in word_docs.paragraphs(session, doc) if text]
        yield box, session, doc, ids


def _base(session, doc) -> str:
    """The version the server holds: the token of the feed's answer now."""
    return session.changes(doc, pages=False)["token"]


def _theirs(box, session, doc, target: str, text: str) -> bytes:
    """Their file: the document now, opened by another editor, ``target`` set, saved."""
    other = box.session()
    theirs = other.open(session.save(doc).data, "theirs.docx")
    word_docs.call(box, other, "word_set_text", doc=theirs, target=target, text=text)
    return other.save(theirs).data


def _set(box, session, doc, target: str, text: str, agent: str = "a"):
    view = session.view(context={"agent": agent})
    return box.dispatch(view, "word_set_text", {"doc": doc, "target": target, "text": text})


def test_their_version_comes_in_while_six_writers_edit_and_the_feed_shows_it(opened):
    box, session, doc, ids = opened
    base = _base(session, doc)
    theirs = _theirs(box, session, doc, ids[0], "Their words.")
    errors: list = []

    def writer(w: int):
        def write():
            for k in range(3):
                result = _set(box, session, doc, ids[1 + w], f"Writer {w}, round {k}.", agent=f"w{w}")
                assert result.ok, result.to_json()
        return write

    def run(job):
        try:
            job()
        except BaseException as error:  # noqa: BLE001 -- reported below
            errors.append(error)

    writers = [threading.Thread(target=run, args=(writer(w),), daemon=True) for w in range(6)]
    for thread in writers:
        thread.start()
    came = session.refresh(doc, theirs, base=base, editor="Ada")
    for thread in writers:
        thread.join(DEADLINE)
    assert not any(t.is_alive() for t in writers), "deadlock"
    assert not errors, errors
    assert came["changed"] and "word/document.xml" in came["parts"]
    assert ids[0] in came["blocks"] and came["handed_back"] == []
    found = dict(word_docs.paragraphs(session, doc))
    assert found[ids[0]] == "Their words.", "their block keeps the id they wrote it with"
    assert all(found[ids[1 + w]] == f"Writer {w}, round 2." for w in range(6)), "the writers' edits stay"
    feed = session.changes(doc, base, pages=False)
    assert ids[0] in feed["blocks"]["changed"] and feed["version"] >= came["version"]


def test_with_nothing_changed_here_the_document_saves_as_their_file(opened):
    box, session, doc, ids = opened
    base = _base(session, doc)
    theirs = _theirs(box, session, doc, ids[2], "Echo.")
    came = session.refresh(doc, theirs, base=base, editor="Ada")
    assert session.save(doc).data == theirs
    from ooxml_edit.chunks import chunk_file, file_changes

    assert chunk_file(session.save(doc).data).delta(chunk_file(theirs).leaves()).new == []
    assert file_changes(session, doc, since=came["token"]).parts == ()


def test_a_step_here_on_their_block_is_handed_back_and_named(opened):
    box, session, doc, ids = opened
    base, at_base = _base(session, doc), session.save(doc).data
    assert _set(box, session, doc, ids[0], "Mine.").ok
    assert _set(box, session, doc, ids[1], "Kept.", agent="b").ok
    # their file, made from the version the server held before these steps
    other = box.session()
    then = other.open(at_base, "theirs.docx")
    word_docs.call(box, other, "word_set_text", doc=then, target=ids[0], text="Theirs.")
    came = session.refresh(doc, other.save(then).data, base=base, editor="Ada")
    [back] = came["handed_back"]
    assert back["tool"] == "word_set_text" and back["context"] == {"agent": "a"}
    assert back["blocks"] == [ids[0]] and back["undo"] > back["step"]
    found = dict(word_docs.paragraphs(session, doc))
    assert (found[ids[0]], found[ids[1]]) == ("Theirs.", "Kept.")


def test_an_undo_entangled_with_their_change_is_refused_naming_it(opened):
    box, session, doc, ids = opened
    assert _set(box, session, doc, ids[3], "Mine first.").ok
    base = _base(session, doc)
    theirs = _theirs(box, session, doc, ids[3], "Theirs over it.")
    session.refresh(doc, theirs, base=base, editor="Ada")
    view = session.view(context={"agent": "a"})
    refused = box.dispatch(view, "undo", {"doc": doc})
    assert not refused.ok
    error = refused.error.to_json()
    assert error["code"] == "refused" and "Ada" in error["message"]
    assert error["details"]["changed_by"] == "Ada" and ids[3] in error["details"]["changed"]
    assert dict(word_docs.paragraphs(session, doc))[ids[3]] == "Theirs over it."


def test_an_undo_apart_from_their_change_is_taken(opened):
    box, session, doc, ids = opened
    assert _set(box, session, doc, ids[4], "Mine, apart.").ok
    base = _base(session, doc)
    theirs = _theirs(box, session, doc, ids[5], "Theirs, apart.")
    session.refresh(doc, theirs, base=base, editor="Ada")
    view = session.view(context={"agent": "a"})
    undone = box.dispatch(view, "undo", {"doc": doc})
    assert undone.ok, undone.to_json()
    found = dict(word_docs.paragraphs(session, doc))
    assert found[ids[5]] == "Theirs, apart." and found[ids[4]] != "Mine, apart."


def test_a_base_this_session_never_gave_is_refused_with_the_way_out(opened):
    box, session, doc, ids = opened
    theirs = _theirs(box, session, doc, ids[0], "Late.")
    with pytest.raises(ToolError) as refused:
        session.refresh(doc, theirs, base="0000000000000000-1", editor="Ada")
    assert refused.value.code == "not_found" and "reopen" in str(refused.value.to_json())
