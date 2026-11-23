"""The reflow a changing call's checks report, measured by the Rust session after the call's
edits (lane P5m): from the version its first edit was applied to -- here the one the
session's check saw -- to the version it made, ids followed through the aliases.  It is what
the next check's ``since_last_check`` says of the same two versions."""
from __future__ import annotations


import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "Version"):
    pytest.skip("ooxml_core has no Version", allow_module_level=True)

from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from conftest import FIXTURE_DIR  # noqa: E402
import word_docs  # noqa: E402

FIXTURE = FIXTURE_DIR / "generated" / "trial" / "safety-bulletin.docx"


@pytest.fixture(scope="module")
def toolbox():
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)


def test_an_edit_after_a_check_reports_the_reflow_the_session_measured(toolbox):
    session = toolbox.session()
    doc = session.open(FIXTURE.read_bytes(), name=FIXTURE.name)
    seen = toolbox.dispatch(session, "check", {"doc": doc, "include": ["reflow"]}).data["reflow"]
    assert seen["pages"] <= 20
    target = word_docs.paragraph_ids(toolbox, session, doc)[0]
    done = toolbox.dispatch(session, "word_insert_text", {"doc": doc, "target": target,
                                                         "text": " checked " * 40}).to_json()
    reflow = done["checks"]["reflow"]
    assert isinstance(reflow, dict), done["checks"]
    assert reflow["changed"] and reflow["pages"][0] == seen["pages"], reflow
    after = toolbox.dispatch(session, "check", {"doc": doc, "include": ["reflow"]}).data["reflow"]
    assert after["since_last_check"] == reflow
