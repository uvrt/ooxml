"""Lane H2: no call of a document the Rust session owns is prepared on a draft -- edits side
by side under ``block_edits`` "always" are each served by the session.  Made-up documents
only."""

from __future__ import annotations

import threading

import pytest

from docx_agent.rust import edits
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox

import word_docs

WRITERS = 6


def _made(markdown: str) -> bytes:
    """A blank document with ``markdown`` inserted by the toolbox, as bytes (every paragraph
    with its paraId)."""
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(word_docs.blank(), "made.docx")
        word_docs.call(box, session, "word_insert_markdown", doc=doc, at="end", markdown=markdown)
        return word_docs.saved(session, doc)


def _ids(session, doc) -> dict[str, str]:
    """Each paragraph's id by its text (the first of a text), from the package."""
    out: dict[str, str] = {}
    for block, text in word_docs.paragraphs(session, doc):
        out.setdefault(text, block)
    return out


def _report(title: str) -> bytes:
    return _made("\n\n".join([f"# {title}"] + [f"Lantern meadow {k} by the harbour." for k in range(WRITERS)]))


@pytest.mark.parametrize("mode", ["always", "auto"])
def test_an_owned_documents_edits_are_served_never_on_drafts(mode):
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0, block_edits=mode) as box:
        session = box.session()
        doc = session.open(_report("A made-up report"), "report.docx")
        targets = [block for block, text in word_docs.paragraphs(session, doc) if text.startswith("Lantern")]
        served = edits.served().get("word_set_text", 0)
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
        assert edits.served().get("word_set_text", 0) - served == WRITERS
        texts = {text for _, text in word_docs.paragraphs(session, doc)}
        assert {f"River stone {k}." for k in range(WRITERS)} <= texts
