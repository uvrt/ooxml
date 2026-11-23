"""Lane H4: ``word_changes``' reading actions, ``list`` and ``preview``, answered by the Rust
session on the version the call reads -- as docx-agent's own handler answers them on the
review fixtures, paged by the host's limits -- and a preview's count of the records this
session wrote told from the document as the session opened it.  Made-up documents and the
generated review fixtures only."""

from __future__ import annotations

import json
import re
from pathlib import Path

import pytest

ooxml_core = pytest.importorskip("ooxml_core")

from ooxml_edit.tools import Limits, Toolbox  # noqa: E402
from ooxml_edit.tools.owned import OwnedEntry  # noqa: E402

from docx_agent import Document  # noqa: E402
from docx_agent.rust import reads  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from conftest import FIXTURE_DIR  # noqa: E402

GENERATED = FIXTURE_DIR / "generated"

#: The generated documents holding tracked changes.
REVIEWED = ["markdown/review.docx", "review-follow.docx", "tables-accepted.docx", "ids-and-markup.docx",
            "own-tracked-note.docx", "controls.docx"]

#: A page of three items: the listings page.
LIMITS = Limits(max_list_items=3)

_DATE = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ?")


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0, limits=LIMITS) as box:
        yield box


def _plain(result) -> dict:
    return json.loads(_DATE.sub("<date>", json.dumps(result.to_json(), sort_keys=True, ensure_ascii=False)))


def _both(toolbox, session, doc: str, arguments: dict) -> tuple[dict, dict]:
    """The call's reply from the Rust session (counted as served) and from the Python
    handler (no Rust session: every read by the Python path)."""
    arguments = {"doc": doc, **arguments}
    served = reads.served().get("word_changes", 0)
    rust = _plain(toolbox.dispatch(session, "word_changes", arguments))
    assert reads.served().get("word_changes", 0) == served + 1, (arguments, reads.fallbacks().get("word_changes"))
    before = reads.use_session(None)
    try:
        python = _plain(toolbox.dispatch(session, "word_changes", arguments))
    finally:
        reads.use_session(before)
    return rust, python


def _readings(document) -> list[dict]:
    """The reading calls of a document's changes: every listing, by author, kind and place,
    a second page, and previews of all, an id, an author and a place."""
    changes = document.changes()
    out = [{"action": "list"}, {"action": "list", "detail": "records"}, {"action": "preview", "all": True}]
    if not changes:
        return out
    first = changes[0]
    out += [{"action": "list", "kind": first.kind}, {"action": "preview", "ids": [first.id]}]
    if first.author:
        out += [{"action": "list", "author": first.author}, {"action": "preview", "author": first.author}]
    if first.paragraph_ids:
        out += [{"action": "list", "within": first.paragraph_ids[0]},
                {"action": "preview", "within": first.paragraph_ids[0]}]
    return out


def test_a_preview_counts_the_records_written_since_the_document_was_opened(toolbox):
    doc0 = Document.new()
    doc0.insert_markdown("\n\n".join(f"River stone number {c}." for c in range(1, 6)))
    session = toolbox.session()
    doc = session.open(doc0.to_bytes(), "report.docx")
    if not isinstance(session.entry(doc), OwnedEntry):
        pytest.skip("no Rust session to own the document")
    ids = {p.text: p.id for p in session.entry(doc).document.paragraphs()}
    assert toolbox.dispatch(session, "word_set_tracking", {"doc": doc, "on": True, "author": "Reviewer A"}).ok
    for c in range(1, 6):
        written = toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": ids[f"River stone number {c}."],
                                                              "text": f"River pebble {c}."})
        assert written.ok, written.error
    served = reads.served().get("word_changes", 0)
    preview = toolbox.dispatch(session, "word_changes", {"doc": doc, "action": "preview", "all": True})
    assert reads.served().get("word_changes", 0) == served + 1, reads.fallbacks().get("word_changes")
    for action in ("accept", "reject"):
        outcome = preview.data[action]
        # every record a tracked edit of this session wrote: a deletion and an insertion each
        assert outcome["records"] == 10 and outcome["this_session"] == 10, preview.data
    assert "(all written this session, since the document was opened)" in preview.summary
    assert len(preview.data["changes"]) == 3 and preview.data["changes_not_shown"]["count"] == 2
