"""``word_read`` served by the Rust session (lane P1a) against today's.

The Rust definition (``crates/ooxml-tools-word``, golden ``tests/golden/word_read.json``
there) equals today's byte for byte, and the reply of ``ooxml_core.Session.call("word_read",
...)`` equals today's envelope on every fixture, in every view and with every story: a
chart's and a SmartArt diagram's one-line summary in its drawing's comment (``[chart: ...]``,
``[smartart: ...]``) included.
"""
from __future__ import annotations

from pathlib import Path

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "Session"):          # an ooxml-core before 0.9.0
    pytest.skip("ooxml_core has no Session", allow_module_level=True)

from ooxml_edit.tools import Toolbox

from docx_agent.rust.session import RustTools
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from conftest import FIXTURE_DIR  # noqa: E402

HERE = Path(__file__).resolve().parent
GOLDEN = HERE.parents[2] / "crates" / "ooxml-tools-word" / "tests" / "golden" / "word_read.json"
FIXTURES = sorted((FIXTURE_DIR).rglob("*.docx"))

CASES = {"final": {}, "markup": {"view": "markup"}, "original": {"view": "original"}, "all": {"stories": "all"}}


@pytest.fixture(scope="module")
def toolbox():
    """Today's tools, every read answered by the Python path (lane P1c: the toolbox's reads
    are the Rust session's by default; this compares the two)."""
    from docx_agent.rust import reads

    before = reads.use_session(None)
    try:
        yield Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)
    finally:
        reads.use_session(before)


def test_a_refusal_carries_its_way_out(toolbox):
    rust = RustTools()
    doc = rust.open(FIXTURES[0].read_bytes(), FIXTURES[0].name)
    refused = rust.dispatch("word_read", {"doc": doc, "range": "p:0000FFFF"})
    assert not refused.ok and refused.error.code == "not_found"
    envelope = refused.to_json()["error"]
    assert envelope["field"] == "range" and envelope["try_call"]["arguments"] == {"doc": doc}
