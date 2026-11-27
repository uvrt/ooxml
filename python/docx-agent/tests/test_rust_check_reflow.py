"""``check``'s ``reflow`` answered by the Rust session (lane P5b): laid out by the document's
page engine, handed to the session's ``check``, and ``since_last_check`` against what the
document's last check saw, which the session keeps (``Session.last_check``).
"""
from __future__ import annotations

import pytest

from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from conftest import FIXTURE_DIR
import word_docs

FIXTURES = sorted((FIXTURE_DIR).rglob("*.docx"))[::9]


@pytest.fixture(scope="module")
def toolbox():
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)


def _reflow(toolbox, session, doc):
    reply = toolbox.dispatch(session, "check", {"doc": doc, "include": ["reflow"]})
    assert reply.ok, reply.to_json()
    return reply.data["reflow"]


def test_checks_measure_from_the_last_check(toolbox):
    path = FIXTURES[0]
    session = toolbox.session()
    doc = session.open(path.read_bytes(), name=path.name)
    assert "since_last_check" not in _reflow(toolbox, session, doc)
    assert "since_last_check" not in _reflow(toolbox, session, doc)     # the version the last check saw
    target = word_docs.paragraph_ids(toolbox, session, doc)[0]
    assert toolbox.dispatch(session, "word_insert_text", {"doc": doc, "target": target, "text": " x"}).ok
    assert "since_last_check" in _reflow(toolbox, session, doc)
    assert "since_last_check" not in _reflow(toolbox, session, doc)
