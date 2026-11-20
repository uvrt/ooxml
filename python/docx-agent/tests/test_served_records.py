"""Lane P2d1: the records of placed values kept in the document (:mod:`docx_agent.edit.provenance`)
follow an edit the Rust session serves as they follow docx-agent's own: the same tool call,
answered by the session and by the Python handler, leaves the same custom XML item -- text
facts moved with their text or dropped, labels stretched, a deleted table's or paragraph's
entry pruned, the item, its properties and its relationship gone with the last entry.

Made-up records only (a source ``d:scores`` named, no data)."""

from __future__ import annotations

import io
import zipfile
from pathlib import Path

import pytest

from docx_agent import Document
from docx_agent.edit.provenance import NS
from docx_agent.rust import edits
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry

GENERATED = Path(__file__).resolve().parent / "fixtures" / "generated"
FIXTURE = GENERATED / "markdown" / "constructs.docx"
CHARTS = GENERATED / "charts" / "charts.docx"
RECORD = {"v": 1, "source": "d:scores", "columns": [{"column": "rating", "rows": "1-3"}]}


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


@pytest.fixture(scope="module")
def recorded() -> bytes:
    """constructs.docx keeping two tables' records, a paragraph's facts, another's label, and
    a third paragraph's fact."""
    document = Document.open(FIXTURE.read_bytes())
    for table in ("t:6B000029", "t:6B000033"):
        assert document.store_provenance(table, RECORD)
    fact = {"source": "d:scores/Alpha/rating", "format": "0", "document": "scores.csv", "version": 2}
    assert document.keep_text_facts("p:6B000004", [{**fact, "text": "second", "at": 2},
                                                   {**fact, "text": "level", "at": 9}])
    assert document.keep_text_label("p:6B000005", {"source": "d:scores/Beta/rating", "text": "third",
                                                   "value": 3.5, "at": 2})
    assert document.keep_text_facts("p:6B000019", [{**fact, "text": "one", "at": 5}])
    return document.to_bytes()


@pytest.fixture(scope="module")
def stale(recorded) -> bytes:
    """``recorded`` with entries a save left behind: a paragraph that is not there, a
    volatile table id, a record that is not one."""
    with zipfile.ZipFile(io.BytesIO(recorded)) as package:
        parts = {n: package.read(n) for n in package.namelist()}
    item = next(n for n in parts if n.startswith("customXml/item") and NS.encode() in parts[n])
    extra = ('<prov:source kind="text" key="p:7FFFFFF0">{"v":1,"paragraph":"x","facts":[]}</prov:source>'
             '<prov:source kind="table" key="t@body/2">{"v":1,"source":"d:scores"}</prov:source>'
             '<prov:source kind="table" key="t:6B000035">not a record</prov:source>')
    parts[item] = parts[item].replace(b"</prov:sources>", extra.encode() + b"</prov:sources>")
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as package:
        for name, data in parts.items():
            package.writestr(name, data)
    return out.getvalue()


@pytest.fixture(scope="module")
def charted() -> bytes:
    """charts.docx keeping two charts' records."""
    document = Document.open(CHARTS.read_bytes())
    for chart in ("d:1", "d:2"):
        assert document.store_provenance(chart, RECORD)
    return document.to_bytes()


def _kept(data: bytes) -> dict[str, bytes | None]:
    """What the records leave in a package: the item, the main part's relationships, the
    content types and every part's name."""
    with zipfile.ZipFile(io.BytesIO(data)) as package:
        names = sorted(package.namelist())
        items = [n for n in names if n.startswith("customXml/item") and "Props" not in n
                 and NS.encode() in package.read(n)]
        return {"item": package.read(items[0]) if items else None,
                "rels": package.read("word/_rels/document.xml.rels"),
                "types": package.read("[Content_Types].xml"),
                "names": "\n".join(names).encode()}


def _run(toolbox, data: bytes, calls: list[tuple[str, dict]], served: bool) -> list[dict]:
    """Each call's records after it, on a document the session owns: served by the Rust
    session, or (``served`` false) by docx-agent's handler."""
    session = toolbox.session()
    doc = session.open(data, name="recorded.docx")
    entry = session.entry(doc)
    if not isinstance(entry, OwnedEntry):
        pytest.skip("no Rust session to own the document")
    out = []
    for tool, arguments in calls:
        before = sum(edits.served().values())
        result = toolbox.dispatch(session, tool, {"doc": doc, **arguments})
        assert result.ok, result.error
        assert (sum(edits.served().values()) > before) == served, (tool, edits.fallbacks())
        out.append(_kept(entry.document.to_bytes()))
    session.close(doc)
    return out


#: Cases on a document of their own (the fixture's name), the rest on ``recorded``.
ON = {"stale entries pruned": "stale", "chart's paragraph deleted": "charted"}

CASES = {
    "stale entries pruned": [("word_format", {"targets": ["p:6B000002"], "bold": True})],
    "chart's paragraph deleted": [("word_delete", {"target": "p@body/4"})],
    "fact moved": [("word_set_text", {"target": "p:6B000004", "text": "Now a second level"})],
    "fact touched": [("word_set_text", {"target": "p:6B000004", "text": "A 2nd level"})],
    "facts all touched": [("word_set_text", {"target": "p:6B000004", "text": "Other"})],
    "inserted before": [("word_insert_text", {"target": "p:6B000004@0", "text": "Then "})],
    "label stretched": [("replace_text", {"find": "third", "replace": "thirdly", "expect": "one"})],
    "label replaced": [("replace_text", {"find": "A third level", "replace": "Low", "expect": "one"})],
    "formatted": [("word_format", {"targets": ["p:6B000004"], "bold": True})],
    "table deleted": [("word_delete", {"target": "t:6B000029"})],
    "paragraph deleted": [("word_delete", {"target": "p:6B000019"})],
    "text deleted": [("word_delete", {"find": "Step one"})],
    "every record gone": [
        ("word_delete", {"target": "t:6B000029"}),
        ("word_delete", {"target": "t:6B000033"}),
        ("word_set_text", {"target": "p:6B000004", "text": "Gone"}),
        ("word_set_text", {"target": "p:6B000005", "text": "Gone too"}),
        ("word_delete", {"target": "p:6B000019"}),
    ],
}


@pytest.mark.parametrize("case", list(CASES))
def test_served_edits_follow_the_records_as_docx_agent_does(toolbox, request, case, monkeypatch):
    calls = CASES[case]
    data = request.getfixturevalue(ON.get(case, "recorded"))
    rust = _run(toolbox, data, calls, served=True)
    monkeypatch.setattr(edits, "SERVED", {})
    python = _run(toolbox, data, calls, served=False)
    assert rust[0]["item"] != _kept(data)["item"] or case == "formatted"
    for k, (ours, theirs) in enumerate(zip(rust, python)):
        for key in theirs:
            assert ours[key] == theirs[key], (case, k, key, ours[key], theirs[key])
    if case == "fact moved":
        assert b'"text":"second","at":6,"p":0' in rust[0]["item"]
    if case == "every record gone":
        assert rust[-1]["item"] is None and b"customXml" not in rust[-1]["names"]
