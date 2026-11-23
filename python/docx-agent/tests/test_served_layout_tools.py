"""Lane P2c13: ``word_template``'s upgrade served by the Rust session, which lays the versions
out with the document's page engine (``CallOptions(layout=...)``)."""

from __future__ import annotations

import datetime as dt

import pytest

from docx_agent.rust import edits
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from conftest import FIXTURE_DIR  # noqa: E402

CLOCK = lambda: dt.datetime(2026, 11, 20, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
MODE14 = FIXTURE_DIR / "generated" / "mode14.docx"


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


def test_the_upgrade_served_with_its_reflow(toolbox):
    session = toolbox.session(clock=CLOCK)
    doc = session.open(MODE14.read_bytes(), "report.docx")
    before = edits.served().get("word_template", 0)
    result = toolbox.dispatch(session, "word_template", {"doc": doc, "action": "upgrade_to_modern"})
    assert result.ok, result.error
    assert edits.served().get("word_template", 0) == before + 1
    assert result.data["mode"] == 15
