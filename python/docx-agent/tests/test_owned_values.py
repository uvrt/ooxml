"""Lane H7e: a check of numbers and a scale on a document the Rust session owns read the session.

The docx format's ``number_texts`` and ``scale_values`` hooks, given the call's subject (an
``OwnedDocument`` for an owned document), read the version as the session reads it
(``Version.number_texts``, ``Version.scale_values``) and place it with the session's own engine,
never the Python view.  ``check``'s numbers and scale on an owned document equal docx-agent's
Python on the same document held by the tool layer itself, for every listed gate fixture and
for made-up documents with tables, labels and values written by reference.
"""

from __future__ import annotations

import dataclasses
import json
from pathlib import Path

import pytest

ooxml_core = pytest.importorskip("ooxml_core")

from docx_agent import Document  # noqa: E402
from docx_agent.rust import hooks  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from ooxml_edit.tools import OwnedDocument, Toolbox  # noqa: E402

FIXTURES = Path(__file__).parent / "fixtures"
ROOT = Path(__file__).resolve().parents[3]
LISTED = sorted(json.loads((ROOT / "tools" / "gates_fixtures.json").read_text()))

STEPS = [{"min": 0, "max": 1.99, "fill": "C62828", "label": "Low"},
         {"min": 2, "max": 3.49, "fill": "F9A825", "label": "Mid"},
         {"min": 3.5, "fill": "2E7D32", "label": "High"}]


@pytest.fixture(scope="module")
def boxes():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as owned, \
            Toolbox(TOOLS, formats=[dataclasses.replace(FORMAT, owner=None)], groups=GROUPS, workers=0) as held:
        yield owned, held


def _both(boxes, data: bytes, name: str, **arguments):
    """``check`` with ``arguments`` on ``data`` owned by the Rust session, and held by the tool
    layer itself (docx-agent's Python hooks): their replies, as JSON."""
    out, sessions = [], []
    for box in boxes:
        session = box.session()
        doc = session.open(data, name)
        sessions.append((session, doc))
        out.append(box.dispatch(session, "check", {"doc": doc, **arguments}).to_json())
    entry = sessions[0][0].entry(sessions[0][1])
    if not entry.owned:
        pytest.skip("no Rust session owns the document here")
    assert isinstance(entry.subject, OwnedDocument)
    assert not sessions[1][0].entry(sessions[1][1]).owned
    return out


def _same(owned, held, key):
    def part(reply):
        data = reply.get("data") or {}
        return {"error": reply.get("error"), "part": data.get(key), "warnings": reply.get("warnings")}
    assert part(owned) == part(held)


def _targets(document: Document) -> list[str]:
    """Scale targets a fixture holds: paragraphs, a range, cells, columns, a reference, and
    targets that name nothing or something else."""
    out: list[str] = []
    for paragraph in document.paragraphs()[:4]:
        out += [paragraph.id, f"{paragraph.id}@0:2"]
    for table in document.tables()[:2]:
        out += [f"{table.id}/c1,0", f"{table.id}/col0", f"{table.id}/col1", table.id]
    return out + ["{{d:scores/*/score}}", "p:7FFFFFFF", "t:7FFFFFFF/col1"]


@pytest.mark.parametrize("name", LISTED)
def test_numbers_of_an_owned_document_are_the_python_ones(boxes, name):
    data = (FIXTURES / name).read_bytes()
    _same(*_both(boxes, data, Path(name).name, include=["numbers"]), "numbers")


@pytest.mark.parametrize("name", LISTED)
def test_a_scale_on_an_owned_document_reads_the_python_values(boxes, name):
    data = (FIXTURES / name).read_bytes()
    scale = {"name": "Score", "steps": STEPS, "targets": _targets(Document.open(data))}
    _same(*_both(boxes, data, Path(name).name, scale=scale), "scale")


def _report() -> bytes:
    document = Document.new()
    document.insert_markdown("# Results\n\nThe mean was 3.4 across 12 sites in 2024.\n\n"
                             "| site | score |\n|---|---|\n| A | 1.5 |\n| B | 3 |\n| C | 4.2 |\n\n"
                             "Low\n\n2.75")
    return document.to_bytes()


@pytest.mark.parametrize("arguments", [
    {"include": ["numbers"]}, {"include": ["numbers"], "pages": [1]}, {"include": ["numbers"], "pages": [2]},
])
def test_numbers_on_pages_and_in_a_range_are_the_python_ones(boxes, arguments):
    data = _report()
    first = Document.open(data).paragraphs()[1].id
    _same(*_both(boxes, data, "report.docx", **arguments), "numbers")
    _same(*_both(boxes, data, "report.docx", range=[first], **arguments), "numbers")


def test_a_scale_s_values_and_steps_now_are_the_python_ones(boxes):
    data = _report()
    document = Document.open(data)
    table = document.tables()[0].id
    paragraphs = [p.id for p in document.paragraphs()]
    for show in ("value", "label"):
        scale = {"name": "Score", "steps": STEPS, "show": show,
                 "targets": [f"{table}/col1", f"{table}/c2,1", paragraphs[-1], paragraphs[-2],
                             f"{paragraphs[1]}@13:16", f"{paragraphs[1]}@13..{paragraphs[2]}@1"]}
        _same(*_both(boxes, data, "report.docx", scale=scale), "scale")
        _same(*_both(boxes, data, "report.docx", scale=scale, pages=[1]), "scale")
        _same(*_both(boxes, data, "report.docx", scale=scale, range=[paragraphs[-1]]), "scale")


def test_the_owned_document_s_reads_are_the_session_s(boxes, monkeypatch):
    seen = []
    for name in ("number_texts", "scale_values"):
        real = getattr(hooks, name)
        monkeypatch.setattr(hooks, name, lambda *a, _real=real, _name=name: seen.append(_name) or _real(*a))
    data = _report()
    table = Document.open(data).tables()[0].id
    _both(boxes, data, "report.docx", include=["numbers"])
    _both(boxes, data, "report.docx", scale={"name": "Score", "steps": STEPS, "targets": [f"{table}/col1"]})
    assert seen == ["number_texts", "scale_values"]
