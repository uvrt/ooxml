"""Round 59 (R134): no whole layout comes to the calling process for ``word_inspect``'s
placements, a table or a span kept together, a change's reflow, or a batch's reads; and
``word_inspect`` gives each table cell's fill and the colour its text is drawn in.

What each test holds:

* through a toolbox with a worker, each of those paths reads what it needs in the worker
  holding the document's layout -- nothing is laid out or converted in this process
  (``docx_agent.layout.convert`` is never called here) -- and answers exactly what the same
  call answers on a toolbox without workers, where the layout is read here as before;
* each cell's ``fill`` and ``text_color`` are what docx2svg draws for every cell of the round
  58 regions document (``tools/table_regions_probe.py``; docx2svg draws it as Word does,
  ``tests/test_table_regions_r58.py``), and for theme fills with a tint or shade, direct
  shading over a region's, automatic text on dark fills and runs of several colours.

Tool design rules: 4 (the answer's size: two keys per cell, a third only where runs differ),
8 (the colours are those Word draws, measured; docx2svg's drawing compared).  Made-up
documents only."""

from __future__ import annotations

import datetime as dt
import json
import sys
from pathlib import Path

import pytest
from ooxml_edit.tools import Toolbox

from docx_agent import Document
from docx_agent import layout as layout_module
from docx_agent.tools import FORMAT, GROUPS, TOOLS

from test_liveview import report

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))

import table_regions_probe as probe  # noqa: E402

pytestmark = pytest.mark.any_faces

CLOCK = lambda: dt.datetime(2026, 11, 17, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731


@pytest.fixture(scope="module")
def boxes():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=1) as worker, \
            Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as here:
        yield worker, here


@pytest.fixture
def no_layout_here(monkeypatch):
    """Fails a test that lays a document out, or converts one, in this process."""
    calls: list = []

    def refused(*args, **kwargs):
        calls.append(args)
        raise AssertionError("a whole layout was made in the calling process")

    monkeypatch.setattr(layout_module, "convert", refused)
    return calls


def ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, result.error and result.error.to_json())
    return result


def long_table_doc() -> bytes:
    """A made-up document: a paragraph, a table of 70 rows (taller than a page), a closing
    paragraph, and a short table."""
    document = Document.new()
    rows = "\n".join(f"| Row {k} | Owner {k % 7} | {k * 3} |" for k in range(70))
    document.insert_markdown("# Long table\n\nAn opening paragraph.\n\n| Item | Owner | Amount |\n|---|---|---|\n"
                             + rows + "\n\nA closing paragraph.\n\n| A | B |\n|---|---|\n| 1 | 2 |")
    return document.to_bytes()


# -- word_inspect's placements ---------------------------------------------------------------------


def test_inspect_places_blocks_in_the_worker_as_here(boxes, no_layout_here):
    data = report()
    document = Document.open(data)
    table = document.tables()[1].id
    span = f"{document.paragraphs()[0].id}..{table}"

    def steps(box, session, doc):
        return [ok(box, session, "word_inspect", doc=doc, target=target, layout=True).data
                for target in (table, span, f"{table}/c2,1")]

    worker = steps(*_open(boxes[0], data))
    assert no_layout_here == []
    here = _here(boxes[1], data, steps)
    assert json.dumps(worker) == json.dumps(here)
    placed = [b for b in worker[1]["blocks"] if b.get("placements")]
    assert len(placed) > 5 and all(p["page"] >= 1 for b in placed for p in b["placements"])


def test_inspect_in_a_batch_holding_the_document_is_placed_in_the_worker(boxes, no_layout_here):
    data = report()
    document = Document.open(data)
    table = document.tables()[0].id
    first = document.paragraphs()[2].id

    def steps(box, session, doc):
        result = ok(box, session, "batch", ops=[
            {"tool": "word_set_text", "arguments": {"doc": doc, "target": first, "text": "A shorter paragraph."}},
            {"tool": "word_inspect", "arguments": {"doc": doc, "target": table, "layout": True}},
            {"tool": "word_set_text", "arguments": {"doc": doc, "target": first, "text": "Shorter again."}}])
        return _op(result, 1)

    worker = steps(*_open(boxes[0], data))
    assert no_layout_here == []
    here = _here(boxes[1], data, steps)
    assert json.dumps(worker, sort_keys=True) == json.dumps(here, sort_keys=True)


def _op(result, k: int) -> dict:
    """A batch's op ``k`` as it answered, less its timing."""
    found = json.loads(json.dumps(result.data["ops"][k]))
    found.pop("timing", None)
    found.get("data", {}).pop("timing", None)
    return found


def _open(box, data: bytes):
    session = box.session(clock=CLOCK)
    return box, session, session.open(data, "made.docx")


def _here(box, data: bytes, steps):
    """``steps`` on the toolbox without workers, with ``convert`` as it is (the fixture's
    refusal undone for it)."""
    from _pytest.monkeypatch import MonkeyPatch

    saved = layout_module.convert
    with MonkeyPatch.context() as patch:
        patch.setattr(layout_module, "convert", _ORIGINAL_CONVERT)
        out = steps(*_open(box, data))
    assert layout_module.convert is saved
    return out


_ORIGINAL_CONVERT = layout_module.convert


# -- a table and a span kept together ----------------------------------------------------------------


def test_a_table_kept_together_is_measured_in_the_worker_as_here(boxes, no_layout_here):
    data = long_table_doc()
    tables = [t.id for t in Document.open(data).tables()]

    def steps(box, session, doc):
        out = []
        for table in tables:
            result = ok(box, session, "word_format_table", doc=doc, table=table, scope="table", keep_together=True)
            out.append(result.warnings)
        return out

    worker = steps(*_open(boxes[0], data))
    assert no_layout_here == []
    here = _here(boxes[1], data, steps)
    assert worker == here
    assert any("taller than a page" in w for w in worker[0]) and not any("taller" in w for w in worker[1])


def test_a_span_kept_together_is_measured_in_the_worker_as_here(boxes, no_layout_here):
    data = long_table_doc()
    document = Document.open(data)
    paragraphs = document.paragraphs()
    span = f"{paragraphs[1].id}..{document.tables()[0].id}"

    def steps(box, session, doc):
        ok(box, session, "check", doc=doc, include=["reflow"])         # laid out: measured before
        return ok(box, session, "word_format", doc=doc, targets=[span], keep_together=True).warnings

    worker = steps(*_open(boxes[0], data))
    assert no_layout_here == []
    here = _here(boxes[1], data, steps)
    assert worker == here and any("cannot be kept on one page" in w for w in worker)


# -- a change's reflow, a batch's check ------------------------------------------------------------


def test_a_changes_reflow_is_compared_from_the_workers_summary_as_here(boxes, no_layout_here):
    data = report()
    paragraph = [p for p in Document.open(data).paragraphs() if len(p.text) > 200][1].id

    def steps(box, session, doc):
        ok(box, session, "check", doc=doc, include=["reflow"])
        return ok(box, session, "word_set_text", doc=doc,
                  items=[{"target": paragraph, "text": "Short now."}]).checks["reflow"]

    worker = steps(*_open(boxes[0], data))
    assert no_layout_here == []
    here = _here(boxes[1], data, steps)
    assert worker == here and worker["changed"]


def test_a_batchs_check_and_page_count_are_read_in_the_worker_as_here(boxes, no_layout_here):
    data = report()
    paragraph = [p for p in Document.open(data).paragraphs() if len(p.text) > 200][1].id

    def steps(box, session, doc):
        result = ok(box, session, "batch", ops=[
            {"tool": "word_set_text", "arguments": {"doc": doc, "target": paragraph, "text": "Short now."}},
            {"tool": "check", "arguments": {"doc": doc, "include": ["reflow"]}},
            {"tool": "word_set_text", "arguments": {"doc": doc, "target": paragraph, "text": "Shorter."}}])
        return _op(result, 1)

    worker = steps(*_open(boxes[0], data))
    assert no_layout_here == []
    here = _here(boxes[1], data, steps)
    assert json.dumps(worker, sort_keys=True) == json.dumps(here, sort_keys=True)


# -- each cell's fill and drawn text colour (rules 4, 8) -------------------------------------------


def _labelled_cells(state: dict) -> dict[str, dict]:
    """Each cell's ``fill``, ``text_color`` and ``text_colors`` by its text."""
    out = {}

    def walk(blocks):
        for block in blocks:
            if block["type"] != "table":
                continue
            for line in block["rows"]:
                for cell in line:
                    text = "".join(b.get("t", "") for b in cell["blocks"]).strip()
                    out[text] = {k: cell[k] for k in ("fill", "text_color", "text_colors") if k in cell}
                    walk(cell["blocks"])

    walk(state["blocks"])
    return out


def _drawn_cells(data: bytes) -> dict[str, dict]:
    from test_table_regions_r58 import _drawn

    return _drawn(data)[0]


def test_every_regions_cell_has_the_fill_and_text_colour_docx2svg_draws():
    data = probe.document()
    cells = _labelled_cells(Document.open(data).state())
    drawn = _drawn_cells(data)
    assert len(drawn) > 300
    for label, mine in drawn.items():
        assert cells[label] == {"fill": mine["fill"], "text_color": mine["color"]}, label


def _themed_doc() -> bytes:
    """Made-up cells: a theme fill with a tint, one with a shade, a direct fill over a dark
    header region, a coloured run beside an automatic one, an empty dark cell, no fill."""
    import io
    import zipfile

    document = Document.new()
    document.insert_markdown("| Ttint | Tshade | Hdirect |\n|---|---|---|\n| **Mixed** red | | Plain |")
    table = document.tables()[0].id
    document.styles.add("Dark header", "table", regions=[{"region": "header_row", "shading": "1F3864"}])
    document.set_table(table, style="Dark header")
    document.format_cell(table, 0, 0, shading="ABCDEF")
    document.format_cell(table, 0, 1, shading="FEDCBA")
    document.format_cell(table, 0, 2, shading="FFF2CC")
    document.format_cell(table, 1, 1, shading="203864")
    document.table(table).cell(1, 0).paragraphs[0].runs[0].format(color="C00000")
    packed = io.BytesIO(document.to_bytes())
    out = io.BytesIO()
    with zipfile.ZipFile(packed) as source, zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for info in source.infolist():
            content = source.read(info.filename)
            if info.filename == "word/document.xml":
                text = content.decode("utf-8")
                # Theme fills (accent 1 with a tint, accent 2 with a shade) in place of the two
                # made-up fills: the hex left as Word writes it beside them.
                for fill, theme in (("ABCDEF", 'w:themeFill="accent1" w:themeFillTint="66"'),
                                    ("FEDCBA", 'w:themeFill="accent2" w:themeFillShade="BF"')):
                    assert f'w:fill="{fill}"' in text
                    text = text.replace(f'w:fill="{fill}"', f'w:fill="FFFFFF" {theme}')
                content = text.encode("utf-8")
            target.writestr(info, content)
    return out.getvalue()


def test_theme_tints_direct_fills_mixed_runs_and_empty_cells_are_as_docx2svg_draws():
    data = _themed_doc()
    document = Document.open(data)
    state = document.state()
    [table] = [b for b in state["blocks"] if b["type"] == "table"]
    cells = [cell for line in table["rows"] for cell in line]
    drawn = _drawn_text_colours(data)
    for cell in cells:
        text = "".join(b.get("t", "") for b in cell["blocks"]).strip()
        if text in drawn:
            assert cell["fill"] == drawn[text]["fill"], text
            colours = drawn[text]["colors"]
            if len(colours) == 1:
                assert cell["text_color"] == colours[0] and "text_colors" not in cell, text
            else:
                assert cell["text_color"] == "mixed" and cell["text_colors"] == colours, text
    by_text = {"".join(b.get("t", "") for b in c["blocks"]).strip(): c for c in cells}
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
    import re

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


def test_inspect_gives_each_cell_its_fill_and_text_colour(boxes):
    box = boxes[1]
    session = box.session(clock=CLOCK)
    doc = session.open(probe.document(), "regions.docx")
    table = Document.open(probe.document()).tables()[[c[0] for c in probe.CASES].index("A")].id
    data = ok(box, session, "word_inspect", doc=doc, target=table).data
    [first] = data["blocks"][0]["rows"][0][:1]
    assert first["fill"] == "000000" and first["text_color"] == "FFFFFF"
    # Rule 4: two keys a cell, a third only where its runs differ.
    assert all(set(cell) - {"id", "row", "column", "row_span", "column_span", "blocks"} == {"fill", "text_color"}
               for line in data["blocks"][0]["rows"] for cell in line)
