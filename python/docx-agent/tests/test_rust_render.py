"""Lane RD1: ``render`` served by the Rust session (``crates/ooxml-tools-word``,
``document/render``): its definition today's byte for byte; its pages and sheets the
pixels today's renderer draws (docx2svg's ``convert_docx_to_png``), its reply today's
(``pages``, ``cached``, ``coverage``, ``sheet``, ``range``), each image kept for the next
call; ``slides`` read as pages with a warning.  Made-up documents only."""

from __future__ import annotations

from pathlib import Path

import pytest

pytest.importorskip("ooxml_core")
pytest.importorskip("resvg_py")

from ooxml_edit.tools import Limits, Toolbox  # noqa: E402
from ooxml_edit.tools.owned import OwnedEntry  # noqa: E402

from docx_agent import Document  # noqa: E402
from docx_agent.rust import reads  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from docx_agent.tools.shared import render_pages  # noqa: E402
from pixels import all_pixels, pixels  # noqa: E402

HERE = Path(__file__).resolve().parent
GOLDEN = HERE.parents[2] / "crates" / "ooxml-tools-word" / "tests" / "golden"


def test_the_rust_definition_is_todays_byte_for_byte():
    from test_rust_tool_specs import export, text

    today = export({tool.name: tool for tool in TOOLS}["render"])
    assert (GOLDEN / "render.json").read_text(encoding="utf-8") == text(today)


def _pages(count: int) -> bytes:
    """``count`` pages whatever the fonts measure: each paragraph after the first starts a
    page, in Arial."""
    from ooxml_edit.xml import qn

    document = Document.new()
    theme = document.package.tree("word/theme/theme1.xml")
    for latin in theme.iter(qn("a:latin")):
        latin.set("typeface", "Arial")
    document.package.mark_dirty("word/theme/theme1.xml")
    for number in range(count):
        made = document.append_paragraph(f"Page {number + 1} of a made-up note.")
        if number:
            document.format_paragraph(made.id, page_break_before=True)
    return document.to_bytes()


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, font_dirs=[], workers=0,
                 limits=Limits(image_budget=None)) as made:
        yield made


def _served(box, data: bytes):
    session = box.session()
    doc = session.open(data, "note.docx")
    if not isinstance(session.entry(doc), OwnedEntry):
        pytest.skip("no Rust session to own the document")
    return session, doc


def _render(box, session, **arguments):
    before = reads.served().get("render", 0)
    result = box.dispatch(session, "render", arguments)
    assert result.ok, result.error and result.error.to_json()
    assert reads.served().get("render", 0) == before + 1, reads.fallbacks().get("render")
    return result


def test_pages_are_todays_pixels_and_kept_for_the_next_call(box):
    data = _pages(3)
    expected = render_pages(data, [1, 3], 300, [])[0]
    session, doc = _served(box, data)
    first = _render(box, session, doc=doc, pages=[1, 3], width=300)
    assert all_pixels(image.data for image in first.images) == all_pixels(expected)
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
    before = reads.served().get("render", 0)
    tall = box.dispatch(session, "render", {"doc": doc, "pages": list(range(1, 31)),
                                            "sheet": {"columns": 1, "width": 300}})
    assert not tall.ok and tall.error.code == "limit" and "pages" in tall.error.message
    past = box.dispatch(session, "render", {"doc": doc, "pages": [2, 5], "sheet": {"width": 200}})
    assert not past.ok and past.error.code == "not_found"
    assert past.error.message == "the document has 3 page(s)"
    assert reads.served().get("render", 0) == before + 2, reads.fallbacks().get("render")


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
    block = [p.id for p in session.entry(doc).document.paragraphs()][-1]
    result = _render(box, session, doc=doc, range=[block], width=300)
    assert result.data["pages"] == [3]
    assert result.data["range"] == [{"range": block, "blocks": 1, "pages": [3]}]


def test_fast_draws_the_exact_image(box):
    # Measured (probe-LC2/render-time.txt): Rust's exact page costs less than today's fast one,
    # so fast is accepted and draws exactly; nothing is approximate.
    data = _pages(2)
    session, doc = _served(box, data)
    result = _render(box, session, doc=doc, pages=[2], width=300, fast=True)
    assert all_pixels(image.data for image in result.images) == all_pixels(render_pages(data, [2], 300, [])[0])
    assert "approximate" not in result.data
