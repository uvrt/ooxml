"""``word_read`` served by the Rust session (lane P1a): a refusal carries its way out."""
from __future__ import annotations

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "Session"):          # an ooxml-core before 0.9.0
    pytest.skip("ooxml_core has no Session", allow_module_level=True)

from docx_agent.rust.session import RustTools
from conftest import FIXTURE_DIR  # noqa: E402

FIXTURES = sorted((FIXTURE_DIR).rglob("*.docx"))

def test_a_refusal_carries_its_way_out():
    rust = RustTools()
    doc = rust.open(FIXTURES[0].read_bytes(), FIXTURES[0].name)
    refused = rust.dispatch("word_read", {"doc": doc, "range": "p:0000FFFF"})
    assert not refused.ok and refused.error.code == "not_found"
    envelope = refused.to_json()["error"]
    assert envelope["field"] == "range" and envelope["try_call"]["arguments"] == {"doc": doc}
