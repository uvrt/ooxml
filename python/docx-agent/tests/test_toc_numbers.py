"""Round 32, R12 and R14: a table of contents' entries for numbered headings, and its right
tab stop next to a ``toc N`` style's own, as Word's own update writes them (measured once with
``crates/ooxml-oracle/word/probes/toc_numbers_probe.py``: ``crates/ooxml-oracle/word/observations/toc-numbers-word.json``).  The update is
``word_fields`` ``update`` on the Rust session."""

from __future__ import annotations

import io
import json
import zipfile

import pytest
from lxml import etree
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import toc_number_cases
from toc_number_cases import W, entries
from conftest import OBSERVATIONS  # noqa: E402

OBSERVED = json.loads((OBSERVATIONS / "toc-numbers-word.json").read_text())["cases"]
CASES = {**toc_number_cases.CASES, **toc_number_cases.CASES2, **toc_number_cases.CASES3}


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _ok(result):
    assert result.ok, result.to_json()
    return result


def updated(box, name: str, times: int = 1) -> tuple[bytes, dict]:
    """The case's document after ``times`` field updates, and the last check's validation."""
    session = box.session()
    doc = session.open(CASES[name], f"{name}.docx")
    for _ in range(times):
        _ok(box.dispatch(session, "word_fields", {"doc": doc, "action": "update"}))
    validate = _ok(box.dispatch(session, "check", {"doc": doc, "include": ["validate"]})).data["validate"]
    return session.save(doc).data, validate


@pytest.mark.parametrize("name", sorted(OBSERVED))
def test_the_entries_are_words(box, name):
    """Every entry's tab stops and runs as Word wrote them: the number, a tab run and the
    text (or the number, a space and the text in one run), the stop after the number, and
    a right dot stop only where the toc style has no stop of its own."""
    assert entries(updated(box, name)[0]) == OBSERVED[name]


def test_the_number_s_tab_run_takes_the_mark_s_formatting(box):
    data, validate = updated(box, "n-tab")
    root = etree.fromstring(zipfile.ZipFile(io.BytesIO(data)).read("word/document.xml"))
    entry = next(p for p in root.iter(W + "p")
                 if (p.find(f"{W}pPr/{W}pStyle") is not None
                     and p.find(f"{W}pPr/{W}pStyle").get(W + "val").lower() in ("toc1", "toc 1")))
    tab = [r for r in entry.find(W + "hyperlink") if r.find(W + "tab") is not None][0]
    assert [c.tag for c in tab.find(W + "rPr")] == [W + "noProof"]
    assert validate["new"] == [], validate


def test_an_update_is_stable(box):
    """A second update writes the same entries (the numbers counted once per update)."""
    assert entries(updated(box, "n-wide", times=2)[0]) == entries(updated(box, "n-wide")[0])
