"""Lane H4: ``word_changes``' preview answered by the Rust session on the version the call
reads, paged by the host's limits: its count of the records this session wrote told from the
document as the session opened it.  Made-up documents only."""

from __future__ import annotations

import pytest

ooxml_core = pytest.importorskip("ooxml_core")

from ooxml_edit.tools import Limits, Toolbox  # noqa: E402

from docx_agent.rust import reads  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
import word_docs  # noqa: E402

# The Python host's own serving of a Word document (its adapters, owner and mirror), which Phase 3 E2
# deletes with these tests; their calls' answers are the host goldens' (replayed by the Rust host).
pytestmark = pytest.mark.python_host

#: A page of three items: the listings page.
LIMITS = Limits(max_list_items=3)


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0, limits=LIMITS) as box:
        yield box


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


def test_a_preview_counts_the_records_written_since_the_document_was_opened(toolbox):
    session = toolbox.session()
    doc = session.open(_made("\n\n".join(f"River stone number {c}." for c in range(1, 6))), "report.docx")
    ids = _ids(session, doc)
    assert toolbox.dispatch(session, "word_set_tracking", {"doc": doc, "on": True, "author": "Reviewer A"}).ok
    for c in range(1, 6):
        written = toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": ids[f"River stone number {c}."],
                                                              "text": f"River pebble {c}."})
        assert written.ok, written.error
    served = reads.served().get("word_changes", 0)
    preview = toolbox.dispatch(session, "word_changes", {"doc": doc, "action": "preview", "all": True})
    assert reads.served().get("word_changes", 0) == served + 1
    for action in ("accept", "reject"):
        outcome = preview.data[action]
        # every record a tracked edit of this session wrote: a deletion and an insertion each
        assert outcome["records"] == 10 and outcome["this_session"] == 10, preview.data
    assert "(all written this session, since the document was opened)" in preview.summary
    assert len(preview.data["changes"]) == 3 and preview.data["changes_not_shown"]["count"] == 2
