"""Round 59 (R134): ``word_inspect`` gives each table cell's fill and the colour its text is
drawn in -- what docx2svg draws for every cell of the round 58 regions document
(:mod:`table_regions_cases`; docx2svg draws it as Word does, ``tests/test_table_regions_r58.py``),
and for theme fills with a tint or shade, direct shading over a region's, automatic text on
dark fills and runs of several colours.  (That no whole layout came to the calling process for
the layout's reads was the Python layout's: the Rust session lays out where it reads.)

Tool design rules: 4 (the answer's size: two keys per cell, a third only where runs differ),
8 (the colours are those Word draws, measured; docx2svg's drawing compared).  Made-up
documents only."""

from __future__ import annotations

import datetime as dt
import re

import pytest
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import table_regions_cases as probe  # noqa: E402
import word_docs  # noqa: E402

pytestmark = pytest.mark.any_faces

CLOCK = lambda: dt.datetime(2026, 11, 17, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, result.error and result.error.to_json())
    return result


def _tables(box, session, doc: str = "d1") -> list[str]:
    return [b for b, _ in word_docs.blocks(box, session, doc) if b.startswith("t")]


def _inspected_cells(box, data: bytes) -> list[dict]:
    """Every body table's cells as word_inspect gives them, with their text (``_text``)."""
    session = box.session(clock=CLOCK)
    session.open(data, "cells.docx")
    out = []
    for table in _tables(box, session):
        for line in ok(box, session, "word_inspect", doc="d1", target=table).data["blocks"][0]["rows"]:
            for cell in line:
                out.append({**cell, "_text": "".join(b.get("t", "") for b in cell["blocks"]).strip()})
    return out


# -- each cell's fill and drawn text colour (rules 4, 8) -------------------------------------------


def _labelled_cells(box, data: bytes) -> dict[str, dict]:
    """Each cell's ``fill``, ``text_color`` and ``text_colors`` by its text."""
    return {cell["_text"]: {k: cell[k] for k in ("fill", "text_color", "text_colors") if k in cell}
            for cell in _inspected_cells(box, data)}


def _drawn_cells(data: bytes) -> dict[str, dict]:
    from test_table_regions_r58 import _drawn

    return _drawn(data)[0]


def test_every_regions_cell_has_the_fill_and_text_colour_docx2svg_draws(box):
    data = probe.document()
    cells = _labelled_cells(box, data)
    drawn = _drawn_cells(data)
    assert len(drawn) > 300
    for label, mine in drawn.items():
        assert cells[label] == {"fill": mine["fill"], "text_color": mine["color"]}, label


def _themed_doc(box) -> bytes:
    """Made-up cells: a theme fill with a tint, one with a shade, a direct fill over a dark
    header region, a coloured run beside an automatic one, an empty dark cell, no fill."""
    session = box.session(clock=CLOCK)
    session.open(word_docs.blank(), "themed.docx")

    def call(tool, **arguments):
        return ok(box, session, tool, doc="d1", **arguments)

    made = call("word_insert_markdown", at="end", markdown="| Ttint | Tshade | Hdirect |\n|---|---|---|\n"
                                                           "| **Mixed** red | | Plain |")
    table = next(b for b in made.data["blocks"] if b.startswith("t:"))
    call("word_styles", action="add", name="Dark header", kind="table",
         regions=[{"region": "header_row", "shading": "1F3864"}])
    call("word_format_table", table=table, scope="table", style="Dark header")
    for row, column, fill in ((0, 0, "ABCDEF"), (0, 1, "FEDCBA"), (0, 2, "FFF2CC"), (1, 1, "203864")):
        call("word_format_table", table=table, scope="cell", row=row, column=column, shading=fill)
    cell = word_docs.cells_of(word_docs.rows_of(word_docs.element(session, "d1", table))[1])[0]
    paragraph = f"p:{cell.find(f'{{{word_docs.W}}}p').get(word_docs.W14_PARA)}"
    call("word_format", targets=[f"{paragraph}@0:5"], color="C00000")
    # Theme fills (accent 1 with a tint, accent 2 with a shade) in place of the two made-up
    # fills: the hex left as Word writes it beside them.

    def themed(text: str) -> str:
        for fill, theme in (("ABCDEF", 'w:themeFill="accent1" w:themeFillTint="66"'),
                            ("FEDCBA", 'w:themeFill="accent2" w:themeFillShade="BF"')):
            assert f'w:fill="{fill}"' in text
            text = text.replace(f'w:fill="{fill}"', f'w:fill="FFFFFF" {theme}')
        return text

    return word_docs.replaced(word_docs.saved(session, "d1"), {"word/document.xml": themed})


def test_theme_tints_direct_fills_mixed_runs_and_empty_cells_are_as_docx2svg_draws(box):
    data = _themed_doc(box)
    cells = _inspected_cells(box, data)
    drawn = _drawn_text_colours(data)
    for cell in cells:
        text = cell["_text"]
        if text in drawn:
            assert cell["fill"] == drawn[text]["fill"], text
            colours = drawn[text]["colors"]
            if len(colours) == 1:
                assert cell["text_color"] == colours[0] and "text_colors" not in cell, text
            else:
                assert cell["text_color"] == "mixed" and cell["text_colors"] == colours, text
    by_text = {c["_text"]: c for c in cells}
    # A tint and a shade of theme colours: hex, neither white nor the theme colour itself.
    assert by_text["Ttint"]["fill"] not in (None, "FFFFFF") and by_text["Tshade"]["fill"] not in (None, "FFFFFF")
    # Direct shading over the dark header region: light, so automatic text is black.
    assert by_text["Hdirect"] == {**by_text["Hdirect"], "fill": "FFF2CC", "text_color": "000000"}
    assert by_text["Mixed red"]["text_color"] == "mixed" and by_text["Mixed red"]["text_colors"] == ["C00000", "000000"]
    assert by_text["Plain"]["fill"] is None and by_text["Plain"]["text_color"] == "000000"
    # An empty cell on a dark fill: the colour text written there would be drawn in.
    [empty] = [c for c in cells if c["row"] == 1 and c["column"] == 1]
    assert empty["fill"] == "203864" and empty["text_color"] == "FFFFFF"


def _drawn_text_colours(data: bytes) -> dict[str, dict]:
    """docx2svg's drawing: per cell text, the cell's fill and its spans' colours in order."""
    import docx2svg
    from lxml import etree

    svg_ns = "{http://www.w3.org/2000/svg}"
    out: dict[str, dict] = {}
    for page in docx2svg.convert_docx_to_svg(data):
        root = etree.fromstring(page.encode())
        shading = [(float(r.get("x") or 0), float(r.get("y") or 0), float(r.get("width") or 0),
                    float(r.get("height") or 0), r) for r in root.iter(svg_ns + "rect")
                   if r.get("data-docx-kind") == "cell-shading"]
        for group in root.iter(svg_ns + "g"):
            path = group.get("data-docx-path") or ""
            if not re.search(r"/w:tc\[\d+\]/w:p\[1\]$", path):
                continue
            texts = list(group.iter(svg_ns + "text"))
            if not texts:
                continue
            label = "".join(t.text or "" for t in texts).strip()
            x, y = float(texts[0].get("x").split()[0]), float(texts[0].get("y"))
            under = [r for r in shading if r[0] <= x <= r[0] + r[2] and r[1] <= y - 5 <= r[1] + r[3]]
            fill = min(under, key=lambda r: r[2] * r[3])[4].get("fill").lstrip("#").upper() if under else None
            colours: list[str] = []
            for text in texts:
                if (text.text or "").strip():
                    colour = (text.get("fill") or "#000000").lstrip("#").upper()
                    if colour not in colours:
                        colours.append(colour)
            out[label] = {"fill": fill, "colors": colours}
    return out


def test_inspect_gives_each_cell_its_fill_and_text_colour(box):
    session = box.session(clock=CLOCK)
    doc = session.open(probe.document(), "regions.docx")
    table = _tables(box, session, doc)[[c[0] for c in probe.CASES].index("A")]
    data = ok(box, session, "word_inspect", doc=doc, target=table).data
    [first] = data["blocks"][0]["rows"][0][:1]
    assert first["fill"] == "000000" and first["text_color"] == "FFFFFF"
    # Rule 4: two keys a cell, a third only where its runs differ.
    assert all(set(cell) - {"id", "row", "column", "row_span", "column_span", "blocks"} == {"fill", "text_color"}
               for line in data["blocks"][0]["rows"] for cell in line)
