"""Lane P2c13: ``word_fields``, ``word_headers_footers`` and ``word_template`` served by the
Rust session, which lays the versions out with the document's page engine
(``docx_agent.pages.word_pages``, ``CallOptions(layout=...)``); their replies as docx-agent's
own handler gives them (a session with the Rust session off is answered by the Python handler).
Made-up documents only."""

from __future__ import annotations

import datetime as dt

import pytest

from docx_agent import Document
from docx_agent.rust import edits
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry
from python_handler import python_session
from conftest import FIXTURE_DIR  # noqa: E402

CLOCK = lambda: dt.datetime(2026, 11, 20, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
MODE14 = FIXTURE_DIR / "generated" / "mode14.docx"


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


@pytest.fixture(scope="module")
def report() -> bytes:
    doc = Document.new()
    parts = ["# A made-up report"]
    for c in range(1, 4):
        parts += [f"# Part {c}", " ".join(["Lantern meadow river stone harbour."] * 40)]
    doc.insert_markdown("\n\n".join(parts))
    return doc.to_bytes()


def _open(toolbox, data: bytes, served: bool):
    """A session that serves, or one the Python handler answers (the Rust session off); both
    date their writes by ``CLOCK``."""
    if not served:
        return python_session(toolbox, data, "report.docx", clock=CLOCK)
    session = toolbox.session(clock=CLOCK)
    doc = session.open(data, "report.docx")
    if served and not isinstance(session.entry(doc), OwnedEntry):
        pytest.skip("no Rust session to own the document")
    return session, doc


def _calls(toolbox, data: bytes, served: bool, calls) -> list:
    session, doc = _open(toolbox, data, served)
    out = []
    for tool, arguments in calls(session, doc):
        before = dict(edits.served())
        result = toolbox.dispatch(session, tool, {"doc": doc, **arguments})
        assert result.ok, result.error
        if served:
            assert edits.served().get(tool, 0) == before.get(tool, 0) + 1, edits.fallbacks()
        data = result.data
        if isinstance(data, dict) and "timing" in data:
            # Round 45 (R67): where an update's time went -- seconds, so only its parts
            # (docx-agent's own has its work ahead in Python, which a served update skips,
            # and its reflow, which a served edit's checks compare where the view is current).
            data = {**data, "timing": sorted(set(data["timing"]) - {"ahead", "spacing", "reflow"})}
        out.append((result.summary, data, sorted(result.changed), result.warnings))
    return out


def _report_calls(session, doc):
    document = session.entry(doc).document
    section = document.sections()[0].id
    heads = [p.id for p in document.paragraphs() if p.text.startswith("Part ")]
    yield "word_headers_footers", {"section": section, "which": "footer", "action": "set",
                                   "text": "A made-up report", "page_number": "page_x_of_y"}
    yield "word_fields", {"action": "insert_toc", "before": heads[0]}
    yield "word_fields", {"action": "update"}


def test_the_upgrade_served_with_its_reflow(toolbox):
    calls = lambda *_: [("word_template", {"action": "upgrade_to_modern"})]  # noqa: E731
    served = _calls(toolbox, MODE14.read_bytes(), True, calls)
    assert served[0][1]["mode"] == 15
    assert served == _calls(toolbox, MODE14.read_bytes(), False, calls)
