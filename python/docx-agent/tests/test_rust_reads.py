"""``word_inspect`` on the Rust host (lane P1b): a refusal carries its way out (the call that
works)."""
from __future__ import annotations

from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from conftest import FIXTURE_DIR

FIXTURES = sorted((FIXTURE_DIR).rglob("*.docx"))


def test_a_refusal_carries_its_way_out():
    path = FIXTURES[0]
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        session = toolbox.session()
        doc = session.open(path.read_bytes(), path.name)
        refused = toolbox.dispatch(session, "word_inspect", {"doc": doc, "target": "p:0000FFFF"}).to_json()["error"]
    assert refused["code"] == "not_found" and refused["field"] == "target"
    assert refused["try_call"]["arguments"] == {"doc": doc, "target": "body"}
