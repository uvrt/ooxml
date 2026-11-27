"""Lane RD1: ``render`` served by the Rust session (``crates/ooxml-tools-word``,
``document/render``): its pages and sheets, its reply (``pages``, ``cached``, ``coverage``,
``sheet``, ``range``), each image kept for the next call; ``slides`` read as pages with a
warning.  Made-up documents only."""

from __future__ import annotations

import re

import pytest


from ooxml_edit.tools import Limits, Toolbox  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from pixels import all_pixels, pixels  # noqa: E402
from word_docs import blank, paragraph, paragraph_ids, replaced, with_body  # noqa: E402


def _pages(count: int) -> bytes:
    """``count`` pages whatever the fonts measure: each paragraph after the first starts a
    page, in Arial."""
    arial = replaced(blank(), {"word/theme/theme1.xml": lambda xml: re.sub(
        r'(<a:latin typeface=")[^"]*"', r'\1Arial"', xml)})
    body = "".join(paragraph(f"Page {number + 1} of a made-up note.", "<w:pageBreakBefore/>" if number else "")
                   for number in range(count))
    return with_body(body, arial)


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, font_dirs=[], workers=0,
                 limits=Limits(image_budget=None)) as made:
        yield made


def _served(box, data: bytes):
    session = box.session()
    return session, session.open(data, "note.docx")


def _render(box, session, **arguments):
    result = box.dispatch(session, "render", arguments)
    assert result.ok, result.error and result.error.to_json()
    return result


def test_pages_are_drawn_and_kept_for_the_next_call(box):
    session, doc = _served(box, _pages(3))
    first = _render(box, session, doc=doc, pages=[1, 3], width=300)
    assert [pixels(image.data)[0][0] for image in first.images] == [300, 300]
    assert [image.label for image in first.images] == ["page 1", "page 3"]
    assert first.summary == "Rendered page(s) 1, 3 (final view: changes accepted, comments hidden)"
    assert first.data["pages"] == [1, 3] and first.data["cached"] == []
    assert first.data["coverage"]["pages"] == 3
    again = _render(box, session, doc=doc, pages=[3], width=300)
    assert again.data["cached"] == [3]


def test_a_page_past_the_end_names_the_pages_there_are(box):
    session, doc = _served(box, _pages(2))
    result = box.dispatch(session, "render", {"doc": doc, "pages": [5]})
    assert not result.ok and result.error.code == "not_found"


def test_a_sheet_too_tall_is_refused_before_the_pages_are_looked_for(box):
    # docx-agent's order (lane RD3): thirty named pages in one column are too tall at the
    # shortest page the sections declare -- said before anything is laid out, before
    # whether the document has them; a sheet that fits is then told the pages there are.
    session, doc = _served(box, _pages(3))
    tall = box.dispatch(session, "render", {"doc": doc, "pages": list(range(1, 31)),
                                            "sheet": {"columns": 1, "width": 300}})
    assert not tall.ok and tall.error.code == "limit" and "pages" in tall.error.message
    past = box.dispatch(session, "render", {"doc": doc, "pages": [2, 5], "sheet": {"width": 200}})
    assert not past.ok and past.error.code == "not_found"
    assert past.error.message == "the document has 3 page(s)"


def test_slides_are_read_as_pages_with_a_warning(box):
    session, doc = _served(box, _pages(2))
    result = _render(box, session, doc=doc, slides=["2"], width=300)
    assert result.data["pages"] == [2]
    assert "slides read as pages: a document has pages" in result.warnings


def test_a_sheet_tiles_the_pages_into_one_labelled_image(box):
    session, doc = _served(box, _pages(3))
    result = _render(box, session, doc=doc, sheet={"columns": 3, "width": 200})
    assert [image.label for image in result.images] == ["sheet of pages 1-3"]
    assert result.data["pages"] == [1, 2, 3] and result.data["cached"] is False
    assert result.data["sheet"] == {"columns": 3, "width": 200}
    (width, _height), _ = pixels(result.images[0].data)
    assert width == 3 * 200 + 4 * 8
    again = _render(box, session, doc=doc, sheet={"columns": 3, "width": 200})
    assert again.data["cached"] is True


def test_a_range_draws_the_pages_its_blocks_are_on(box):
    session, doc = _served(box, _pages(3))
    block = paragraph_ids(box, session, doc)[-1]
    result = _render(box, session, doc=doc, range=[block], width=300)
    assert result.data["pages"] == [3]
    assert result.data["range"] == [{"range": block, "blocks": 1, "pages": [3]}]


def test_fast_draws_the_exact_image(box):
    # Measured (probe-LC2/render-time.txt): Rust's exact page costs less than today's fast one,
    # so fast is accepted and draws exactly; nothing is approximate.
    session, doc = _served(box, _pages(2))
    result = _render(box, session, doc=doc, pages=[2], width=300, fast=True)
    other, other_doc = _served(box, _pages(2))
    exact = _render(box, other, doc=other_doc, pages=[2], width=300)
    assert all_pixels(image.data for image in result.images) == all_pixels(image.data for image in exact.images)
    assert "approximate" not in result.data
