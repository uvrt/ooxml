"""Lane P2d2: values written by reference (``{{d:scores/Gamma/rating}}``) in a text tool the
Rust session serves -- ``word_set_text``, ``word_insert_text``, ``replace_text`` -- are kept
on the paragraphs they went into as docx-agent's handler keeps them: the same call, answered
by the session and by the Python handler, leaves the same custom XML item (made where the
document had none: the item, its properties, their relationships and content types) and
lists the same ``data.facts``.

Made-up data only: the toy dataset of :mod:`test_tables_from_data`."""

from __future__ import annotations

import io
import zipfile

import pytest

from docx_agent.edit.provenance import NS
from docx_agent.rust import edits
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry

from test_served_records import FIXTURE, _serve_none, recorded  # noqa: F401 -- a fixture
from test_tables_from_data import ds_put


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox([*TOOLS, ds_put], formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


@pytest.fixture(scope="module")
def plain() -> bytes:
    return FIXTURE.read_bytes()


def _kept(data: bytes) -> dict[str, bytes | None]:
    """What the records leave in a package: the item and its properties and relationships,
    the main part's relationships, the content types and every part's name."""
    with zipfile.ZipFile(io.BytesIO(data)) as package:
        names = sorted(package.namelist())
        read = package.read
        items = [n for n in names if n.startswith("customXml/item") and "Props" not in n and NS.encode() in read(n)]
        item = items[0] if items else None
        props = item.replace("item", "itemProps", 1) if item else None
        rels = item.replace("customXml/", "customXml/_rels/", 1) + ".rels" if item else None
        return {"item": read(item) if item else None,
                "props": read(props) if props in names else None,
                "item rels": read(rels) if rels in names else None,
                "rels": read("word/_rels/document.xml.rels"),
                "types": read("[Content_Types].xml"),
                "names": "\n".join(names).encode()}


def _run(toolbox, data: bytes, calls: list[tuple[str, dict]], served: bool) -> list[tuple[dict, object]]:
    """Each call's records and reported facts after it, on a document the session owns."""
    session = toolbox.session()
    doc = session.open(data, name="recorded.docx")
    entry = session.entry(doc)
    if not isinstance(entry, OwnedEntry):
        pytest.skip("no Rust session to own the document")
    assert toolbox.dispatch(session, "ds_put", {"id": "scores"}).ok
    out = []
    for tool, arguments in calls:
        before = sum(edits.served().values())
        result = toolbox.dispatch(session, tool, {"doc": doc, **arguments})
        assert result.ok, result.error
        assert (sum(edits.served().values()) > before) == served, (tool, edits.fallbacks())
        out.append((_kept(entry.document.to_bytes()), (result.data or {}).get("facts")))
    session.close(doc)
    return out


#: Cases on ``recorded`` (records there already), the rest on the fixture without any.
ON_RECORDED = {"set beside kept facts", "replace a kept fact"}

CASES = {
    "set a whole value": [("word_set_text", {"target": "p:6B000004", "text": "{{d:scores/Gamma/rating}}"})],
    "set a sentence": [("word_set_text", {"target": "p:6B000004",
                                          "text": "Gamma {{d:scores/Gamma/rating:0.0}}, due {{d:scores/Gamma/due}}"})],
    "set items": [("word_set_text", {"items": [
        {"target": "p:6B000004", "text": "Alpha {{d:scores/Alpha/rating}}"},
        {"target": "p:6B000005", "text": " {{d:scores/Beta/check}} "}]})],
    "set two lines": [("word_set_text", {"target": "p:6B000004",
                                         "text": "First {{d:scores/Alpha/rating}}\nthen {{d:scores/Beta/rating}}"})],
    "set a value of two lines": [("word_set_text", {"target": "p:6B000004", "text": "{{d:scores/Gamma/note}}"})],
    "insert": [("word_insert_text", {"target": "p:6B000004@0", "text": "Rated {{d:scores/Beta/rating}}: "})],
    "insert items": [("word_insert_text", {"items": [
        {"target": "p:6B000004@0", "text": "{{d:scores/Beta/rating}} "},
        {"find": "third", "where": "after", "text": " ({{d:scores/Gamma/rating}})"}]})],
    "replace": [("replace_text", {"find": "third", "replace": "{{d:scores/Alpha/rating}}", "expect": "one"})],
    "the value twice": [("word_set_text", {"target": "p:6B000004",
                                                           "text": "{{d:scores/Gamma/rating}} or 1.25"})],
    "beside a vulgar fraction": [("word_set_text", {"target": "p:6B000004",
                                                    "text": "\u00bd{{d:scores/Gamma/rating}}"})],
    "beside a superscript digit": [("word_set_text", {"target": "p:6B000004",
                                                      "text": "{{d:scores/Gamma/rating}}\u00b2 and 1.25"})],
    "set beside kept facts": [("word_set_text", {"target": "p:6B000004",
                                                 "text": "A second level of {{d:scores/Beta/rating}}"})],
    "replace a kept fact": [("replace_text", {"find": "A second level", "replace": "A {{d:scores/Alpha/check}} level",
                                              "expect": "one"})],
    "then edited by hand": [
        ("word_set_text", {"target": "p:6B000004", "text": "Gamma {{d:scores/Gamma/rating}} today"}),
        ("word_insert_text", {"target": "p:6B000004@0", "text": "So "}),
    ],
}


@pytest.mark.parametrize("case", list(CASES))
def test_served_values_by_reference_are_kept_as_docx_agent_keeps_them(toolbox, request, case, monkeypatch):
    calls = CASES[case]
    data = request.getfixturevalue("recorded" if case in ON_RECORDED else "plain")
    rust = _run(toolbox, data, calls, served=True)
    _serve_none(monkeypatch)
    python = _run(toolbox, data, calls, served=False)
    for k, ((ours, our_facts), (theirs, their_facts)) in enumerate(zip(rust, python)):
        assert our_facts == their_facts, (case, k)
        for key in theirs:
            assert ours[key] == theirs[key], (case, k, key, ours[key], theirs[key])
    if case == "set a whole value":
        assert rust[0][0]["item"] is not None and b'"at":0' in rust[0][0]["item"]
