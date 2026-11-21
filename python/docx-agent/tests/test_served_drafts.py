"""Lane H2: no call of a document the Rust session owns is prepared on a draft, so none is
sent to the Python handler for being on one -- edits side by side under ``block_edits``
"always" are each served; and another document a call names is the session's whether the
call holds it as the document or as a reader of a committed version of it.  Made-up
documents only."""

from __future__ import annotations

import threading
from types import SimpleNamespace

import pytest

from docx_agent import Document
from docx_agent.rust import edits
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry

WRITERS = 6


def _report(title: str) -> bytes:
    doc = Document.new()
    parts = [f"# {title}"] + [f"Lantern meadow {k} by the harbour." for k in range(WRITERS)]
    doc.insert_markdown("\n\n".join(parts))
    return doc.to_bytes()


def _drafted(fallbacks: dict[str, dict[str, int]]) -> int:
    return sum(n for reasons in fallbacks.values() for why, n in reasons.items() if "draft" in why)


@pytest.mark.parametrize("mode", ["always", "auto"])
def test_an_owned_documents_edits_are_served_never_on_drafts(mode):
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0, block_edits=mode) as box:
        session = box.session()
        doc = session.open(_report("A made-up report"), "report.docx")
        entry = session.entry(doc)
        if not isinstance(entry, OwnedEntry):
            pytest.skip("no Rust session to own the document")
        targets = [p.id for p in entry.document.paragraphs() if p.text.startswith("Lantern")]
        served, drafted = edits.served().get("word_set_text", 0), _drafted(edits.fallbacks())
        gate = threading.Barrier(WRITERS)
        results: list = []

        def writer(k: int) -> None:
            view = session.view(context={"agent": f"w{k}"})
            gate.wait(10)
            results.append(box.dispatch(view, "word_set_text", {"doc": doc, "target": targets[k],
                                                                "text": f"River stone {k}."}))

        threads = [threading.Thread(target=writer, args=(k,)) for k in range(WRITERS)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(60)
        assert len(results) == WRITERS and all(r.ok for r in results), [r.error for r in results]
        assert all(r.block is None for r in results), [r.block for r in results]
        assert edits.served().get("word_set_text", 0) - served == WRITERS, edits.fallbacks()
        assert _drafted(edits.fallbacks()) == drafted
        texts = {p.text for p in entry.document.paragraphs()}
        assert {f"River stone {k}." for k in range(WRITERS)} <= texts


def test_another_document_held_as_a_reader_is_the_owners():
    """A reader of a committed version of an owned document (ooxml-edit's ``Committed.entry``:
    no owner, ``live`` the owner's entry) names that document in the session."""
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(_report("A made-up report"), "report.docx")
        other = session.open(_report("Another made-up report"), "source.docx")
        entry, source = session.entry(doc), session.entry(other)
        if not isinstance(entry, OwnedEntry):
            pytest.skip("no Rust session to own the document")
        with source.lock:
            source.commit(session.formats[source.kind])
        reader = source.reader()
        assert reader is not None and getattr(reader, "owner", None) is None and reader.live is source
        tool = SimpleNamespace(doc_ids=lambda arguments: [arguments["doc"], arguments["source_doc"]])
        call = SimpleNamespace(tool=tool, entries={doc: entry, other: reader})
        names = edits._names(call, entry, {"doc": doc, "source_doc": other})
        assert names == {doc: entry.handle.doc, other: source.handle.doc}
