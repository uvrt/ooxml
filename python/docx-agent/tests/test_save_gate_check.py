"""Round 53, R109: ``check`` lists what ``save_document`` would refuse.

The save gate compares the document's validation problems with those it had when opened
(ooxml-edit's :mod:`~ooxml_edit.tools.savegate`); every ``check`` result carries the same
comparison as ``validation`` -- whatever its ``include`` or ``range`` -- and ``validate``'s
own delta is read through it, so the three never disagree.  A made-up "Shelf" document is
edited into three new problems: its outcome lines copied in from a patch whose paragraph styles
neither document has (what a copy without its styles leaves), then fixed with ``word_format``.
"""

from __future__ import annotations

import datetime as dt

import pytest

pytest.importorskip("ooxml_edit.tools.savegate", reason="needs ooxml-edit 0.111.0")

from ooxml_edit.tools import Toolbox  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
import word_docs  # noqa: E402

pytestmark = pytest.mark.any_faces

CLOCK = lambda: dt.datetime(2026, 11, 12, tzinfo=dt.timezone.utc)  # noqa: E731
STYLES = ["ShelfTop", "ShelfMiddle", "ShelfLow"]
NEW = sorted(f"[style-missing] word/document.xml: pStyle {name}" for name in STYLES)
#: The patch: an outcome line per place, each in a paragraph style no document here has.
PATCH = "".join(f'<w:p w14:paraId="0000020{k}" w14:textId="77777777"><w:pPr><w:pStyle w:val="{style}"/></w:pPr>'
                f"<w:r><w:t>Outcome: {place} shelf done.</w:t></w:r></w:p>"
                for k, (style, place) in enumerate(zip(STYLES, ("top", "middle", "low"))))


@pytest.fixture
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _ok(box, session, name, /, **arguments):
    result = box.dispatch(session, name, arguments)
    assert result.ok, (name, result.error and result.error.to_json())
    return result


def _shelf(box):
    """The made-up document: three places, each a heading, a table and an outcome line."""
    session = box.session(clock=CLOCK)
    session.open(word_docs.blank(), "shelf.docx")
    first = word_docs.paragraph_ids(box, session, "d1")[0]
    md = "# Shelf\n\nWhat the shelf holds, by place.\n\n"
    for place in ("Top", "Middle", "Low"):
        md += (f"## {place}\n\n| item | count | notes |\n|---|---|---|\n| jar | 3 | glass |\n"
               f"| tin | 5 | partly done |\n\nOutcome: {place.lower()} shelf partly done.\n\n")
    _ok(box, session, "word_insert_markdown", doc="d1", at=f"after:{first}", markdown=md)
    intro = next(p for p, text in word_docs.paragraphs(session, "d1") if text.startswith("What the shelf"))
    session.open(word_docs.with_body(PATCH), "patch.docx")                      # d2
    return session, intro


def _name_styles(box, session) -> list[str]:
    """The patch's outcome lines copied in at the end: three new problems.  Their ids."""
    return _ok(box, session, "word_copy_from", doc="d1", source_doc="d2", range="p:00000200..p:00000202",
               at="end").data["blocks"]


def _save(box, session):
    return box.dispatch(session, "save_document", {"doc": "d1", "name": "shelf.docx",
                                                   "format": "docx"})


@pytest.mark.parametrize("asked", [{"include": ["layout"]}, {"range": "intro"},
                                   {"include": ["fields"]}, {}])
def test_check_lists_the_problems_the_save_refuses_whatever_it_was_asked(box, asked):
    session, intro = _shelf(box)
    _name_styles(box, session)
    arguments = {"doc": "d1", **({"range": intro} if "range" in asked else asked)}
    checked = _ok(box, session, "check", **arguments)
    refused = _save(box, session)
    assert refused.error.code == "refused"
    fact = checked.data["validation"]
    assert sorted(fact["new"]) == NEW and fact["baseline"] == 0
    assert fact["new"] == refused.error.valid_options      # the same words, the same order
    assert "saving would refuse 3 new validation problem(s)" in checked.summary


def test_the_fix_with_the_tools_clears_it_and_the_save_passes(box):
    session, _ = _shelf(box)
    outcomes = _name_styles(box, session)
    fixed = _ok(box, session, "word_format", doc="d1", targets=outcomes, paragraph_style="Normal")
    assert fixed.checks["validate"]["new"] == []
    checked = _ok(box, session, "check", doc="d1", include=["layout"])
    assert checked.data["validation"] == {"new": [], "baseline": 0}
    assert "saving would refuse" not in checked.summary
    saved = _save(box, session)
    assert saved.ok and saved.data["validate"]["new"] == []


def test_validate_the_calls_checks_and_validation_agree(box):
    session, intro = _shelf(box)
    edited = _ok(box, session, "word_copy_from", doc="d1", source_doc="d2", range="p:00000200..p:00000202",
                 at="end")
    checked = _ok(box, session, "check", doc="d1")
    assert edited.checks["validate"]["new"] == checked.data["validate"]["new"] \
        == checked.data["validation"]["new"]
    # scoped: validate names those in the range (none here), validation the whole document
    scoped = _ok(box, session, "check", doc="d1", range=intro)
    assert scoped.data["validate"]["new"] == [] and scoped.data["validate"]["elsewhere"] == 3
    assert scoped.data["validation"]["new"] == checked.data["validation"]["new"]


def test_a_check_inside_a_batch_sees_the_batchs_own_edit(box):
    session, _ = _shelf(box)
    outcomes = _name_styles(box, session)
    _ok(box, session, "check", doc="d1")                    # kept for this version
    result = _ok(box, session, "batch", ops=[
        {"tool": "word_format", "arguments": {"doc": "d1", "targets": outcomes,
                                              "paragraph_style": "Normal"}},
        {"tool": "check", "arguments": {"doc": "d1", "include": ["validate"]}}])
    inside = result.data["ops"][1]["data"]
    assert inside["validation"]["new"] == [] and inside["validate"]["new"] == []
    assert _save(box, session).ok
