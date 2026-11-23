"""Lane P2e1: a served call's writes are dated by the session's clock, to the second
(``Session.edit(date=...)``, every dated write).  Made-up documents only."""

from __future__ import annotations

import datetime as dt
import pytest

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox

import word_docs

CLOCK = lambda: dt.datetime(2026, 11, 20, 9, 41, 17, tzinfo=dt.timezone.utc)  # noqa: E731
STAMP = 'w:date="2026-11-20T09:41:17Z"'


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


def test_a_served_revision_is_dated_to_the_second(toolbox):
    session = toolbox.session(clock=CLOCK)
    doc = session.open(word_docs.blank(), "report.docx")
    word_docs.call(toolbox, session, "word_insert_markdown", doc=doc, at="end",
                   markdown="# A made-up report\n\nLantern meadow by the harbour.\n")
    word_docs.call(toolbox, session, "word_set_tracking", doc=doc, on=True, author="Ann")
    target = next(block for block, text in word_docs.paragraphs(session, doc) if text == "Lantern meadow by the harbour.")
    word_docs.call(toolbox, session, "word_set_text", doc=doc, target=target, text="Lantern meadow by the river.")
    assert STAMP in word_docs.part(word_docs.saved(session, doc))
