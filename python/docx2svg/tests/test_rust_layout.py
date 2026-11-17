"""The layout in Rust (lane L4, :mod:`docx2svg.rust`) reads as docx2svg's own: on every
fixture it lays out, each page's frame, page info, body blocks, lines, spans, rules,
pictures and floating drawings' boxes are the Python layout's, and each page it draws is
the same SVG, byte for byte; a fixture or page it does not is a counted fallback."""

from __future__ import annotations

from pathlib import Path

import pytest

import docx2svg
from docx2svg import rust

pytestmark = pytest.mark.skipif(not rust.available(), reason="ooxml_core without the Rust layout")

FIXTURES = sorted((Path(__file__).parent / "fixtures").rglob("*.docx"))


def _line(line) -> tuple:
    return (line.path, line.number, line.baseline, line.top, line.pitch, line.paragraph_id,
            tuple(line.band) if line.band else None, line.story, line.part, line.column)


def _span(span) -> tuple:
    return (span.path, "".join(span.chars), tuple(span.xs), span.y, span.end, span.face, span.bold, span.italic,
            span.half_points, span.color, span.underline, span.strike, span.double_strike, span.highlight,
            span.shading, span.kind, span.line_baseline, span.vertical_align)


def _page(page) -> tuple:
    return ((page.number, page.width_px, page.height_px, page.width_pt, page.height_pt, tuple(page.column)),
            page.info, list(page.blocks),
            [(_line(line), [_span(s) for s in line.spans]) for line in page.lines],
            [(r.kind, r.path, r.x, r.y, r.width, r.height, r.color) for r in page.rules],
            [(p.path, p.relationship, p.x, p.y, p.width, p.height, p.part) for p in page.pictures],
            [(f.path, f.x, f.y, f.width, f.height, f.behind, f.story) for f in page.floats])


@pytest.mark.parametrize("path", FIXTURES, ids=lambda p: p.name)
def test_the_rust_layout_is_docx2svgs(path: Path):
    data = path.read_bytes()
    options = docx2svg.ConvertOptions()
    ours = rust.Engine(options).lay_out(data, options)
    if ours is None:
        assert rust.fallbacks(), "a fallback is counted by its reason"
        pytest.skip("laid out by docx2svg's Python layout: " + ", ".join(rust.fallbacks()))
    theirs, _, fonts = docx2svg.lay_out_docx(data, docx2svg.ConvertOptions(), keep=False)
    assert len(ours.pages) == len(theirs.pages)
    svgs = docx2svg.page_svgs(theirs, data, docx2svg.ConvertOptions(), fonts=fonts)
    for index, (mine, docx2svgs) in enumerate(zip(ours.pages, theirs.pages)):
        assert _page(mine) == _page(docx2svgs), index
        svg = ours.page_svg(index)
        assert svg is None or svg == svgs[index], index
    assert ours.coverage.pages == theirs.coverage.pages
    assert ours.coverage.blocks == theirs.coverage.blocks
    assert ours.coverage.substituted_fonts == theirs.coverage.substituted_fonts
    assert ours.coverage.missing_fonts == theirs.coverage.missing_fonts


def test_a_version_laid_out_again_is_the_same_layout():
    data = FIXTURES[0].read_bytes()
    options = docx2svg.ConvertOptions()
    engine = rust.Engine(options)
    first = engine.lay_out(data, options)
    assert first is not None and engine.lay_out(data, options) is first


def test_a_rust_layout_is_never_pickled():
    import pickle

    data = FIXTURES[0].read_bytes()
    options = docx2svg.ConvertOptions()
    layout = rust.Engine(options).lay_out(data, options)
    with pytest.raises(TypeError, match="not pickled"):
        pickle.dumps(layout)


def test_measurers_of_the_callers_own_fall_back_counted():
    options = docx2svg.ConvertOptions(text="outlines")
    before = rust.fallbacks().get("outlined text", 0)
    assert rust.Engine(docx2svg.ConvertOptions()).lay_out(FIXTURES[0].read_bytes(), options) is None
    assert rust.fallbacks()["outlined text"] == before + 1
