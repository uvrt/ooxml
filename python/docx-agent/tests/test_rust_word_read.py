"""``word_read`` on the Rust host (lane P1a): a refusal carries its way out."""
from __future__ import annotations

from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from conftest import FIXTURE_DIR

FIXTURES = sorted((FIXTURE_DIR).rglob("*.docx"))


def test_a_refusal_carries_its_way_out():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        session = toolbox.session()
        doc = session.open(FIXTURES[0].read_bytes(), FIXTURES[0].name)
        refused = toolbox.dispatch(session, "word_read", {"doc": doc, "range": "p:0000FFFF"})
    assert not refused.ok and refused.error.code == "not_found"
    envelope = refused.to_json()["error"]
    assert envelope["field"] == "range" and envelope["try_call"]["arguments"] == {"doc": doc}
