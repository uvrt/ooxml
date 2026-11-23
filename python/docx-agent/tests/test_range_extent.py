"""Round 35, R26: how much of its pages a range takes, in ``check(range=...)``.

Per range, on each page it is drawn on: the top and bottom of its body content in points (what
``word_inspect``'s placements say, line tops to line tops plus pitch), the page's body height
(the page less its section's top and bottom margins), and -- only where a header or footer
reaches past its margin and pushes the body in -- the body's area.  A block's own extent: the
block named as its own range (rule 1: the address forms ``range`` takes already).  Made-up
documents only.
"""

from __future__ import annotations

import re

import pytest

pytest.importorskip("ooxml_edit.tools.shared", reason="needs ooxml-edit (the shared tools)")

from ooxml_edit.tools import Toolbox  # noqa: E402
from layout_reads import block_ids, section_bodies, where, written  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
import word_docs  # noqa: E402

#: The pages a range's extent lists at most (the Rust session's check's).
EXTENT_PAGES = 12

pytestmark = pytest.mark.any_faces

WORDS = "lantern meadow river stone harbour quiet orchard copper valley signal".split()


def _text(seed: int, words: int = 70) -> str:
    return " ".join(WORDS[(seed + k) % len(WORDS)] for k in range(words)).capitalize() + "."


@pytest.fixture
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _check(box, session, **arguments):
    result = box.dispatch(session, "check", arguments)
    assert result.ok, result.error and result.error.to_json()
    return result


def _record(box, session, rows: int = 40) -> tuple[str, list[str]]:
    """A heading, an opening paragraph, a table and an answer paragraph, after enough text
    that the table starts near a page's foot and goes on to the next page."""
    parts = [_text(k) for k in range(8)] + ["## A record", _text(3, 40),
                                             "| Item | Owner |\n|---|---|\n"
                                             + "\n".join(f"| Item {k} | {WORDS[k % 10]} |" for k in range(rows)),
                                             _text(5, 50)]
    d1 = written(box, session, "\n\n".join(parts), "record.docx")
    return d1, block_ids(box, session, d1)[-4:]


def test_a_range_reports_its_extent_on_each_page(box):
    session = box.session()
    d1, record = _record(box, session)
    span = f"{record[0]}..{record[-1]}"
    reported = _check(box, session, doc=d1, range=[span], include=["layout"]).data["range"][0]
    assert len(reported["pages"]) == 2, reported
    first, last = where(box, session, d1, record[0])[0], where(box, session, d1, record[-1])[-1]
    extent = reported["extent"]
    assert [e["page"] for e in extent] == reported["pages"]
    assert extent[0]["top"] == first["top"] and extent[-1]["bottom"] == last["bottom"]
    # The page's body: its height less the section's top and bottom margins.
    [body] = section_bodies(session, d1)
    assert all(e["body"] == body for e in extent)
    assert all("area" not in e and "columns" not in e for e in extent)    # nothing pushed it
    assert reported["height"] == round(sum(e["bottom"] - e["top"] for e in extent), 2)
    assert "extent_more" not in reported


def test_each_block_named_as_its_own_range_has_its_own_extent(box):
    """Per block: the blocks given as ranges beside the span (rule 1: no new field)."""
    session = box.session()
    d1, record = _record(box, session)
    ranges = [f"{record[0]}..{record[-1]}", *record]
    reported = _check(box, session, doc=d1, range=ranges).data["range"]
    for block, entry in zip(record, reported[1:]):
        placements = [p for p in where(box, session, d1, block) if p["story"] == "body"]
        assert [(e["page"], e["top"], e["bottom"]) for e in entry["extent"]] == \
            [(p["page"], p["top"], p["bottom"]) for p in placements]
    table = reported[3]
    assert table["blocks"] == 1 and len(table["extent"]) == 2                # the table over two pages


def test_a_header_taller_than_its_margin_pushes_the_body_and_says_so(box):
    session = box.session()
    d1, record = _record(box, session)
    lines = "\n".join(["Header line"] + [f"Header line {k}" for k in range(6)])
    header = box.dispatch(session, "word_headers_footers", {"doc": d1, "section": "s:body", "which": "header",
                                                            "action": "set", "text": lines})
    assert header.ok, header.to_json()
    reported = _check(box, session, doc=d1, range=[record[1]]).data["range"][0]
    [entry] = reported["extent"]
    top, bottom = entry["area"]
    # The body starts below the header, not at the margin.
    section = word_docs.part(word_docs.saved(session, d1))
    margin = int(re.search(r'<w:pgMar w:top="(\d+)"', section).group(1)) / 20
    assert top > margin + 1
    assert bottom - top < entry["body"]
    assert entry["top"] >= top


def test_each_page_has_its_own_sections_body(box):
    """A landscape section: its pages' body is its page height less its margins."""
    session = box.session()
    d1 = written(box, session, "\n\n".join(_text(k) for k in range(4)))
    paragraphs = block_ids(box, session, d1)
    inserted = box.dispatch(session, "word_sections", {"doc": d1, "action": "insert_break", "after": paragraphs[1],
                                                       "start": "nextPage"})
    assert inserted.ok, inserted.to_json()
    first = box.dispatch(session, "word_sections", {"doc": d1, "action": "list"}).data[0]["id"]
    landscape = box.dispatch(session, "word_sections", {"doc": d1, "action": "set", "section": first,
                                                        "orientation": "landscape"})
    assert landscape.ok, landscape.to_json()
    paragraphs = block_ids(box, session, d1)
    found = _check(box, session, doc=d1, range=[paragraphs[0], paragraphs[3]]).data["range"]
    before, after = found[0]["extent"][0]["body"], found[1]["extent"][0]["body"]
    assert [before, after] == section_bodies(session, d1)
    assert before != after


def test_columns_are_counted_and_the_extent_spans_them(box):
    session = box.session()
    d1 = written(box, session, "\n\n".join(_text(k, 90) for k in range(3)))
    set_ = box.dispatch(session, "word_sections", {"doc": d1, "action": "set", "section": "s:body", "columns": 2})
    assert set_.ok, set_.to_json()
    ids = block_ids(box, session, d1)
    [found] = _check(box, session, doc=d1, range=[f"{ids[0]}..{ids[-1]}"]).data["range"][0]["extent"]
    assert found["columns"] == 2
    placements = [p for i in ids for p in where(box, session, d1, i)]
    assert found["top"] == min(p["top"] for p in placements) and found["bottom"] == max(p["bottom"] for p in placements)


def test_what_is_not_drawn_has_no_extent(box):
    """An id the document does not have is drawn nowhere: a range of it is refused, naming
    the range (rule 2), never an empty extent read as a place; a one-line paragraph's extent is
    one entry, on its page."""
    session = box.session()
    d1 = written(box, session, "One.\n\nTwo.")
    missing = box.dispatch(session, "check", {"doc": d1, "range": ["p:00000000"]})
    assert not missing.ok and missing.error.code == "not_found" and missing.error.field == "range"
    first = block_ids(box, session, d1)[0]
    [extent] = _check(box, session, doc=d1, range=[first]).data["range"][0]["extent"]
    [placed] = where(box, session, d1, first)
    assert (extent["page"], extent["top"], extent["bottom"]) == (1, placed["top"], placed["bottom"])
    assert placed["lines"] == 1


def test_a_long_range_lists_its_first_pages_and_counts_the_rest(box):
    """Rule 4: the extent's pages are capped; the rest are counted, never cut silently.  The
    cap as it is (the Rust session's check's: 12), on a record whose table runs past it; the
    height is every page's, as the blocks' placements give them."""
    session = box.session()
    d1, record = _record(box, session, rows=600)
    reported = _check(box, session, doc=d1, range=[f"{record[0]}..{record[-1]}"]).data["range"][0]
    pages = len(reported["pages"])
    assert pages > EXTENT_PAGES
    assert len(reported["extent"]) == EXTENT_PAGES
    assert reported["extent_more"] == pages - EXTENT_PAGES
    spans: dict[int, list[float]] = {}
    for block in record:
        for placed in where(box, session, d1, block):
            if placed["story"] == "body":
                top, bottom = spans.get(placed["page"], [placed["top"], placed["bottom"]])
                spans[placed["page"]] = [min(top, placed["top"]), max(bottom, placed["bottom"])]
    assert reported["height"] == round(sum(bottom - top for top, bottom in spans.values()), 2)
