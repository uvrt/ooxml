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
            [(r.kind, r.path, r.x, r.y, r.width, r.height, r.color, r.style) for r in page.rules],
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


def _charts(probe) -> bytes:
    """The chart probe's charts but those text wraps beside (laid out by docx2svg's Python
    layout yet)."""
    saved = probe.CASES
    probe.CASES = tuple(case for case in saved if case.where != "square")
    try:
        return probe.build("15")
    finally:
        probe.CASES = saved


def _made() -> dict:
    """Made-up documents of what lane L4d paints in Rust: SmartArt, VML shapes and text paths
    (watermarks), and charts in another locale than en-US."""
    import sys

    sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))
    import make_chart_probe
    import make_smartart_probe
    import test_carry_on as made
    from ooxml_common.locales import Locale

    vml = 'xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office"'
    body = "".join(made._p(f"Paragraph {k} of the body, some words on a line.") for k in range(6))
    shapes = (f'<w:p><w:r><w:pict {vml}><v:rect style="position:absolute;margin-left:20pt;margin-top:10pt;'
              'width:120pt;height:60pt;z-index:2" fillcolor="#4472C4" strokecolor="red" strokeweight="2pt"/>'
              f'</w:pict></w:r><w:r><w:pict {vml}><v:oval style="width:60pt;height:20pt" fillcolor="#ccc" '
              'stroked="f"><v:fill opacity="32768f"/></v:oval></w:pict></w:r></w:p>')
    italic = made.FITTED_WATERMARK.replace("font-size:1pt", "font-size:1pt;font-style:italic;font-weight:bold")
    nl = Locale(decimal_separator=",", thousands_separator=".", date_order="dmy", language="nl-NL",
                list_separator=";")
    return {
        "watermark": (made._docx(body, final=made._sect("rIdH1"),
                                 headers={"rIdH1": f'<w:hdr {made.NS}>{made.WATERMARK}</w:hdr>'}), None),
        "fitted-italic": (made._docx(body + italic), None),
        "vml-shapes": (made._docx(shapes + body), None),
        "smartart": (make_smartart_probe.build("15"), None),
        "charts-nl": (_charts(make_chart_probe), nl),
    }


@pytest.mark.parametrize("name", ["watermark", "fitted-italic", "vml-shapes", "smartart", "charts-nl"])
def test_smartart_vml_and_charts_in_a_locale_are_laid_out_and_drawn_as_docx2svg(name: str):
    data, locale = _made()[name]
    options = docx2svg.ConvertOptions(locale=locale) if locale else docx2svg.ConvertOptions()
    ours = rust.Engine(options).lay_out(data, options)
    assert ours is not None, rust.fallbacks()
    theirs_options = docx2svg.ConvertOptions(locale=locale) if locale else docx2svg.ConvertOptions()
    theirs, _, fonts = docx2svg.lay_out_docx(data, theirs_options, keep=False)
    svgs = docx2svg.page_svgs(theirs, data, theirs_options, fonts=fonts)
    assert [_page(page) for page in ours.pages] == [_page(page) for page in theirs.pages]
    assert [ours.page_svg(index) for index in range(len(svgs))] == svgs
    assert [(w.code, w.message, w.page) for w in ours.warnings] == [
        (w.code, w.message, w.page) for w in theirs_options.warnings]
    assert list(ours.approximations) == list(theirs.approximations)
    assert docx2svg.drawn_faces(ours) == docx2svg.drawn_faces(theirs)


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


def _pixels(png: bytes) -> tuple:
    """An 8-bit RGBA PNG's size and pixels (resvg's and tiny-skia's encoders write no other)."""
    import struct
    import zlib

    at, chunks = 8, {}
    while at < len(png):
        size, = struct.unpack(">I", png[at:at + 4])
        kind = png[at + 4:at + 8]
        chunks[kind] = chunks.get(kind, b"") + png[at + 8:at + 8 + size]
        at += 12 + size
    width, height, depth, colour = struct.unpack(">IIBB", chunks[b"IHDR"][:10])
    assert (depth, colour) == (8, 6)
    raw, stride, out, previous = zlib.decompress(chunks[b"IDAT"]), width * 4, bytearray(), bytearray(width * 4)
    for row in range(height):
        kind, line = raw[row * (stride + 1)], bytearray(raw[row * (stride + 1) + 1:(row + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - 4] if i >= 4 else 0
            b = previous[i]
            c = previous[i - 4] if i >= 4 else 0
            if kind == 1:
                line[i] = (line[i] + a) & 255
            elif kind == 2:
                line[i] = (line[i] + b) & 255
            elif kind == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif kind == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        out += line
        previous = line
    return (width, height), bytes(out)


@pytest.mark.parametrize("name", ["watermark", "vml-shapes"])
def test_a_page_drawn_in_rust_is_png_as_resvg_py_draws_it(name: str):
    pytest.importorskip("resvg_py")
    from docx2svg.png import svg_to_png

    data, _ = _made()[name]
    options = docx2svg.ConvertOptions()
    ours = rust.Engine(options).lay_out(data, options)
    page = ours.pages[0]
    svg = ours.page_svg(0, width=page.width_px // 4, height=page.height_px // 4)
    assert _pixels(rust.page_png(svg)) == _pixels(svg_to_png(svg))


@pytest.mark.parametrize("name", ["smartart", "charts-nl"])
def test_a_chart_s_and_a_diagram_s_text_faces_are_drawn_faces(name: str):
    """The faces a chart's or a diagram's text is drawn in are handed to the rasteriser
    (``drawn_faces``): without them a page whose body names no such face -- a chart alone,
    on a machine where no handed face answers to the generic family -- draws none of the
    chart's words or numbers."""
    from docx2svg.drawing import markup_faces

    data, locale = _made()[name]
    options = docx2svg.ConvertOptions(locale=locale) if locale else docx2svg.ConvertOptions()
    ours = rust.Engine(options).lay_out(data, options)
    theirs, _, _ = docx2svg.lay_out_docx(data, docx2svg.ConvertOptions(locale=locale) if locale
                                         else docx2svg.ConvertOptions(), keep=False)
    written = {face for page in theirs.pages for placed in page.floats for primitive in placed.primitives
               if primitive.kind == "markup" and primitive.what in ("chart", "diagram")
               for face in markup_faces(primitive.markup)}
    assert written and written <= set(docx2svg.drawn_faces(theirs))
    assert docx2svg.drawn_faces(ours) == docx2svg.drawn_faces(theirs)


def test_a_chart_s_text_faces_are_read_from_its_markup():
    from docx2svg.drawing import markup_faces

    markup = ('<g><text x="0"><tspan font-size="12" font-family="Aptos, Carlito, sans-serif">0.2</tspan></text>'
              '<text><tspan font-family="\'Aptos Display\', sans-serif" font-weight="bold" font-style="italic">T'
              '</tspan><tspan font-family="Aptos, Carlito, sans-serif">x</tspan></text><rect/></g>')
    assert markup_faces(markup) == [("Aptos", False, False), ("Aptos Display", True, True)]
