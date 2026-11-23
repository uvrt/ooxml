"""Lane H7e: a check of numbers and a scale on a document the Rust session owns read the session.

The docx format's ``number_texts`` and ``scale_values`` hooks, given the call's subject (an
``OwnedDocument`` for an owned document), read the version as the session reads it
(``Version.number_texts``, ``Version.scale_values``) and place it with the session's own engine.
Made-up documents only.
"""

from __future__ import annotations

import pytest

ooxml_core = pytest.importorskip("ooxml_core")

from docx_agent.rust import hooks  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from ooxml_edit.tools import OwnedDocument, Toolbox  # noqa: E402

import word_docs  # noqa: E402

STEPS = [{"min": 0, "max": 1.99, "fill": "C62828", "label": "Low"},
         {"min": 2, "max": 3.49, "fill": "F9A825", "label": "Mid"},
         {"min": 3.5, "fill": "2E7D32", "label": "High"}]
REPORT = ("# Results\n\nThe mean was 3.4 across 12 sites in 2024.\n\n"
          "| site | score |\n|---|---|\n| A | 1.5 |\n| B | 3 |\n| C | 4.2 |\n\nLow\n\n2.75")


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as made:
        yield made


def test_the_owned_document_s_reads_are_the_session_s(box, monkeypatch):
    seen = []
    for name in ("number_texts", "scale_values"):
        real = getattr(hooks, name)
        monkeypatch.setattr(hooks, name, lambda *a, _real=real, _name=name: seen.append(_name) or _real(*a))
    session = box.session()
    doc = session.open(word_docs.blank(), "report.docx")
    word_docs.call(box, session, "word_insert_markdown", doc=doc, at="end", markdown=REPORT)
    assert isinstance(session.entry(doc).subject, OwnedDocument)
    table = next(block for block, _ in word_docs.blocks(box, session, doc) if block.startswith("t:"))
    numbers = box.dispatch(session, "check", {"doc": doc, "include": ["numbers"]})
    scale = box.dispatch(session, "check", {"doc": doc, "scale": {"name": "Score", "steps": STEPS,
                                                                  "targets": [f"{table}/col1"]}})
    assert seen == ["number_texts", "scale_values"], (numbers.to_json(), scale.to_json())
