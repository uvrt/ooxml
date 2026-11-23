"""A range-scoped check of a long report costs what the range asks (round 57, R124):
``include: ["numbers"]`` names each text's page, and a range or ``pages`` keep only the texts
there.  (The Python layout's and the numbering reads' own costs went with the Python view.)
Made-up documents only."""

from __future__ import annotations

import pytest

pytest.importorskip("ooxml_edit.tools.numbers", reason="needs ooxml-edit (numbers)")

from ooxml_edit.tools import Toolbox  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS as WORD_TOOLS  # noqa: E402

import word_docs  # noqa: E402
from test_tables_from_data import ds_put  # noqa: E402

#: With a made-up dataset to compare numbers with (test_tables_from_data's).
TOOLS = [*WORD_TOOLS, ds_put]

pytestmark = pytest.mark.any_faces

WORDS = "lantern meadow river stone harbour quiet orchard copper valley signal".split()


def _text(seed: int, words: int) -> str:
    return " ".join(WORDS[(seed + k) % len(WORDS)] for k in range(words)).capitalize() + "."


@pytest.fixture(scope="module")
def made() -> bytes:
    parts = ["# Made-up report"]
    for c in range(1, 4):
        parts += [f"# Part {c}", f"{_text(c, 60)} Rated {c}.5 of 5 in part {c}.",
                  "- first point\n- second point\n  - a deeper point",
                  "1. one step\n2. another step",
                  f"| Item | Amount |\n|---|---|\n| lantern | {c}2.5 |\n| meadow | {c}7.25 |",
                  _text(c + 3, 120)]
    with Toolbox(WORD_TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(word_docs.blank(), "report.docx")
        word_docs.call(box, session, "word_insert_markdown", doc=doc, at="end", markdown="\n\n".join(parts))
        return word_docs.saved(session, doc)


def _session(box, data):
    session = box.session()
    doc = session.open(data, "report.docx")
    assert box.dispatch(session, "ds_put", {"id": "values"}).ok
    return session, doc


def test_numbers_name_their_pages_and_a_range_keeps_its_own(made):
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session, doc = _session(box, made)
        result = box.dispatch(session, "check", {"doc": doc, "include": ["numbers", "validate"]})
        assert result.ok, result.to_json()
        texts = result.data["numbers"]["texts"]
        assert texts and all(t.get("page") for t in texts), texts
        assert len({t["page"] for t in texts}) > 1                 # more than one page named
        # A range: only its texts, their pages named the same way.
        heads = [block for block, text in word_docs.paragraphs(session, doc) if text.startswith("Part ")]
        scoped = box.dispatch(session, "check", {"doc": doc, "include": ["numbers"],
                                                 "range": [f"{heads[1]}..{heads[2]}"]})
        assert scoped.ok, scoped.to_json()
        found = scoped.data["numbers"]["texts"]
        assert found and len(found) < len(texts) and all(t.get("page") for t in found), found
        pages = {t["at"]: t["page"] for t in texts}
        assert all(pages.get(t["at"]) == t["page"] for t in found), (found, pages)


def test_numbers_on_pages_keep_only_the_texts_drawn_there(made):
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session, doc = _session(box, made)
        on_one = box.dispatch(session, "check", {"doc": doc, "include": ["numbers"], "pages": [1]})
        assert on_one.ok and on_one.data["numbers"]["texts"]
        past = box.dispatch(session, "check", {"doc": doc, "include": ["numbers"], "pages": [40]})
        assert past.ok and past.data["numbers"]["texts"] == []
