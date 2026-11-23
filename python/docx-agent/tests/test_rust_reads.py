"""``word_inspect`` served by the Rust session (lane P1b): a refusal carries its way out
(the call that works), and ``layout`` without the host's page engine is refused with one.
"""
from __future__ import annotations

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "Session"):          # an ooxml-core before 0.9.0
    pytest.skip("ooxml_core has no Session", allow_module_level=True)

from docx_agent.rust.session import RustTools  # noqa: E402
from conftest import FIXTURE_DIR  # noqa: E402

FIXTURES = sorted((FIXTURE_DIR).rglob("*.docx"))


def test_a_refusal_carries_its_way_out():
    path = FIXTURES[0]
    rust = RustTools()
    doc = rust.open(path.read_bytes(), path.name)
    refused = rust.dispatch("word_inspect", {"doc": doc, "target": "p:0000FFFF"}).to_json()["error"]
    assert refused["code"] == "not_found" and refused["field"] == "target"
    assert refused["try_call"]["arguments"] == {"doc": doc, "target": "body"}
    laid = rust.dispatch("word_inspect", {"doc": doc, "target": "body", "layout": True}).to_json()["error"]
    assert laid["code"] == "refused" and laid["try_call"]["arguments"] == {"doc": doc, "target": "body"}
