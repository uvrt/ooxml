"""Lane H2: a tracking mode the Rust session refuses is the call's refusal: nothing changes.
Made-up documents only."""

from __future__ import annotations

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox

import word_docs


def _made(markdown: str) -> bytes:
    """A blank document with ``markdown`` inserted by the toolbox, as bytes (every paragraph
    with its paraId)."""
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(word_docs.blank(), "made.docx")
        word_docs.call(box, session, "word_insert_markdown", doc=doc, at="end", markdown=markdown)
        return word_docs.saved(session, doc)


def _report() -> bytes:
    return _made("# A made-up report\n\nLantern meadow by the harbour.\n\nRiver stone number one.")
