"""docx2svg is a thin layer over the Rust core: what it draws is the Rust engine's, the options
it does not take are refused with a way out, and the names it withdrew say what to use instead.

The documents are the committed fixtures (``tests/fixtures``): their pages are the Rust
layout's, so these tests hold docx2svg to ``ooxml_core.LaidPages`` itself, not to numbers of
its own.
"""

from __future__ import annotations

import re
import struct
import xml.etree.ElementTree as ElementTree
import zipfile
from io import BytesIO
from pathlib import Path

import pytest

import docx2svg
from docx2svg import ConvertOptions, LayoutError, rust
from docx2svg.cli import main

FIXTURES = Path(__file__).resolve().parent / "fixtures"
SWEEP = FIXTURES / "layout-sweep.docx"


@pytest.fixture(scope="module")
def data() -> bytes:
    return SWEEP.read_bytes()


@pytest.fixture(scope="module")
def laid(data):
    return rust.word_pages(ConvertOptions()).lay_out(data)


def _png_size(png: bytes) -> tuple[int, int]:
    assert png[:8] == b"\x89PNG\r\n\x1a\n"
    return struct.unpack(">II", png[16:24])


def test_the_pages_are_the_rust_layout_s(data, laid):
    svgs = docx2svg.convert_docx_to_svg(data)
    assert laid.page_count > 1
    assert svgs == [laid.svg(index)[0] for index in range(laid.page_count)]


def test_one_layout_gives_the_layout_and_the_selected_pages(data, laid):
    options = ConvertOptions(pages=[2, 99])
    conversion = docx2svg.convert_docx(data, options)
    assert conversion.page_numbers == [2]
    assert conversion.svgs == [laid.svg(1)[0]]
    assert conversion.layout.page_count == laid.page_count
    assert options.coverage is not None and options.coverage.pages == laid.page_count


def test_a_size_given_on_one_side_keeps_the_page_s_shape(data, laid):
    svg = docx2svg.convert_docx_to_svg(data, ConvertOptions(pages=[1], width=400))[0]
    width, height = (int(v) for v in re.search(r'<svg[^>]* width="(\d+)" height="(\d+)"', svg).groups())
    (_, (width_px, height_px, *_)) = laid.pages()[0]
    assert width == 400 and height == round(400 * height_px / width_px)
    assert svg.replace(f' width="{width}" height="{height}"', "", 1) == \
        re.sub(r' width="[^"]*pt" height="[^"]*pt"', "", laid.svg(0)[0], count=1)


def test_png_is_drawn_at_300_dpi_unless_a_size_is_given(data, laid):
    (_, (width_px, height_px, *_)) = laid.pages()[0]
    png = docx2svg.convert_docx_to_png(data, ConvertOptions(pages=[1]))[0]
    assert _png_size(png) == (width_px, height_px)
    small = docx2svg.convert_docx_to_png(data, ConvertOptions(pages=[1], width=200))[0]
    assert _png_size(small)[0] == 200


def test_svg_to_png_draws_with_the_rust_core():
    svg = '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10"/></svg>'
    assert _png_size(docx2svg.svg_to_png(svg)) == (20, 10)
    assert _png_size(docx2svg.svg_to_png(svg, width=40)) == (40, 20)
    assert docx2svg.available_backends() == ["resvg"]


@pytest.mark.parametrize("field", ["advances", "metrics", "decorations", "names"])
def test_measuring_options_are_refused_with_a_way_out(data, field):
    with pytest.raises(ValueError, match="font_dirs"):
        docx2svg.convert_docx_to_svg(data, ConvertOptions(**{field: object()}))


def test_another_rasteriser_is_refused_with_a_way_out(data):
    with pytest.raises(ValueError, match="backend='auto'"):
        docx2svg.convert_docx_to_png(data, backend="cairosvg")
    with pytest.raises(ValueError, match="backend='auto'"):
        docx2svg.svg_to_png("<svg/>", backend="cairosvg")


def test_an_unknown_text_mode_is_refused(data):
    with pytest.raises(ValueError, match="text, outlines"):
        docx2svg.convert_docx_to_svg(data, ConvertOptions(text="paths"))


@pytest.mark.parametrize("name", sorted(docx2svg.WITHDRAWN))
def test_a_withdrawn_name_says_what_to_use_instead(name):
    with pytest.raises(AttributeError, match="instead"):
        getattr(docx2svg, name)


def test_a_document_the_layout_refuses_is_a_layout_error_with_a_way_out(monkeypatch, data):
    class Refusing:
        def lay_out(self, _data):
            raise NotImplementedError("the Word layout does not lay this version out yet: a frame in a header")

    monkeypatch.setattr(rust, "word_pages", lambda options: Refusing())
    with pytest.raises(LayoutError) as found:
        docx2svg.convert_docx_to_svg(data)
    assert isinstance(found.value, NotImplementedError)
    assert "a frame in a header" in str(found.value) and "word_read" in str(found.value)
    assert isinstance(found.value.__cause__, NotImplementedError)


def test_warnings_and_coverage_are_the_layout_s(data, laid):
    options = ConvertOptions()
    docx2svg.convert_docx_to_svg(data, options)
    summary = laid.coverage_summary()
    said = [(w.code, w.message, w.page) for w in options.warnings if w.code != docx2svg.NOT_AS_WORD]
    assert said == [tuple(w) for w in summary["warnings"]]
    coverage = options.coverage
    assert (coverage.complete, coverage.blocks, coverage.blocks_laid_out) == \
        (summary["complete"], summary["blocks"], summary["laid_out"])
    assert coverage.status in ("complete", "approximate", "partial")


def test_every_drawn_path_resolves_in_the_part(data):
    svg = docx2svg.convert_docx_to_svg(data, ConvertOptions(pages=[1]))[0]
    paths = list(dict.fromkeys(re.findall(r'data-docx-path="(w:body/[^"]+)"', svg)))
    assert paths
    with zipfile.ZipFile(BytesIO(data)) as archive:
        root = ElementTree.fromstring(archive.read("word/document.xml"))
    for path in paths[:20]:
        found = docx2svg.resolve_path(root, path)
        assert found is not None, path
        last = path.split("/")[-1].split("[")[0].split(":")[-1]
        assert last == "label" or found.tag.endswith("}" + last), (path, found.tag)
    assert docx2svg.resolve_path(root, "w:body/w:p[99999]") is None


def test_text_as_outlines_needs_no_font_to_be_viewed(data):
    options = ConvertOptions(pages=[1], text="outlines")
    svg = docx2svg.convert_docx_to_svg(data, options)[0]
    kept = [w for w in options.warnings if w.code == "text-not-outlined"]
    assert "<path" in svg
    assert "<text" not in svg or kept


def test_the_command_line_writes_one_file_per_page(tmp_path, data, laid):
    source = tmp_path / "sweep.docx"
    source.write_bytes(data)
    assert main([str(source), "-o", str(tmp_path / "out"), "-p", "1-2", "-f", "both", "-q"]) == 0
    written = sorted(p.name for p in (tmp_path / "out").iterdir())
    assert written == ["sweep-1.png", "sweep-1.svg", "sweep-2.png", "sweep-2.svg"]
    assert (tmp_path / "out" / "sweep-2.svg").read_text(encoding="utf-8") == laid.svg(1)[0]
    assert main([str(source), "--backend", "resvg", "-o", str(tmp_path / "out"), "-p", "99", "-q"]) == 1
