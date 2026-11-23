"""Round 58 (R129): a table style's formatting by region, and a new table in another's frame.

* ``word_styles`` ``add`` / ``modify`` of a table style take ``regions`` -- every region Word's
  Table Design has, named as ``word_format_table`` ``look`` names them where they are the
  same place -- and ``describe`` reads them back, with the look options they show under and
  the tables whose look leaves them off (rules 1-4, 6, 7).
* What Word draws where regions overlap, which rows are the header row, band sizes, region
  borders and automatic text on a dark fill: measured (``tools/table_regions_probe.py``,
  ``tests/observations/table-regions-word.json``), and docx2svg's drawing of the same
  document agrees cell by cell and line by line (rule 8).
* ``word_insert_markdown`` ``like`` a table: a new table in its frame -- rows, cell and text
  formatting, no text -- for a look no style carries (a middle column of its own); tracked
  as an insertion; refused, with the shape and Markdown that fits, where a frame cannot be
  copied faithfully (rules 2, 5).

Made-up tables only.
"""

from __future__ import annotations

import datetime as dt
import json
import re
from pathlib import Path
from types import SimpleNamespace

import pytest
from lxml import etree
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

ROOT = Path(__file__).resolve().parent.parent

import table_regions_cases as probe  # noqa: E402

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
SVG = "{http://www.w3.org/2000/svg}"
OBSERVED = json.loads((ROOT / "tests" / "observations" / "table-regions-word.json").read_text(encoding="utf-8"))
CLOCK = lambda: dt.datetime(2026, 11, 16, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731

LEDGER = [{"region": "header_row", "shading": "1F3864", "color": "FFFFFF", "bold": True,
           "borders": {"style": "single", "sides": ["bottom"], "width": 1.5}},
          {"region": "banded_rows", "shading": "F2F2F2"},
          {"region": "first_column", "bold": True},
          {"region": "total_row", "bold": True, "borders": {"style": "double", "sides": ["top"]}}]


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, result.error and result.error.to_json())
    return result


def refused(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert not result.ok, (tool, result.summary)
    return result.error


def with_table(box, rows: int = 4, columns: int = 3):
    """A blank document with a table of ``rows`` x ``columns`` empty cells: the session and
    the table."""
    session = box.session(clock=CLOCK)
    session.open(word_docs.blank(), "plain.docx")
    first = word_docs.paragraph_ids(box, session, "d1")[0]
    markdown = "\n".join(["|" + " |" * columns, "|" + "---|" * columns] + ["|" + " |" * columns] * (rows - 1))
    made = ok(box, session, "word_insert_markdown", doc="d1", at=f"after:{first}", markdown=markdown)
    return session, next(b for b in made.data["blocks"] if b.startswith("t:"))


def _styles(session):
    return word_docs.tree(word_docs.saved(session, "d1"), "word/styles.xml")


def style_node(session, name: str):
    return next(node for node in _styles(session).iter(_W + "style")
                if node.get(_W + "type") == "table" and node.find(_W + "name").get(_W + "val") == name)


def default_table_style(session) -> str:
    return next(node.get(_W + "styleId") for node in _styles(session).iter(_W + "style")
                if node.get(_W + "type") == "table" and node.get(_W + "default") == "1")


# -- the style: written as Word writes it, read back (rules 1, 3, 4, 6) --------------------------


def test_a_table_style_is_added_with_its_regions_as_word_writes_them(box):
    session, _ = with_table(box)
    result = ok(box, session, "word_styles", doc="d1", action="add", name="Ledger", kind="table", size=9,
                shading="FBFBFB", regions=LEDGER)
    node = style_node(session, "Ledger")
    # As Word writes a new table style (measured): custom, based on Normal Table, the whole
    # table's shading in its own w:tcPr, a band size in its w:tblPr.
    assert node.get(_W + "customStyle") == "1"
    assert node.find(_W + "basedOn").get(_W + "val") == default_table_style(session)
    assert node.find(f"{_W}tcPr/{_W}shd").get(_W + "fill") == "FBFBFB"
    assert node.find(f"{_W}tblPr/{_W}tblStyleRowBandSize").get(_W + "val") == "1"
    assert any("band_size 1" in w for w in result.warnings)
    parts = {p.get(_W + "type"): p for p in node.findall(_W + "tblStylePr")}
    assert list(parts) == ["firstRow", "lastRow", "firstCol", "band1Horz"]       # Word's own order
    header = parts["firstRow"]
    assert [c.tag.rpartition("}")[2] for c in header] == ["rPr", "tcPr"]
    assert header.find(f"{_W}rPr/{_W}b") is not None and header.find(f"{_W}rPr/{_W}bCs") is not None
    bottom = header.find(f"{_W}tcPr/{_W}tcBorders/{_W}bottom")
    assert (bottom.get(_W + "val"), bottom.get(_W + "sz")) == ("single", "12")


def test_word_styles_describes_regions_and_the_looks_they_need(box):
    session, table = with_table(box)
    d = "d1"
    ok(box, session, "word_format_table", doc=d, table=table, scope="table", look=["header_row", "banded_rows"])
    added = ok(box, session, "word_styles", doc=d, action="add", name="Ledger", kind="table", regions=LEDGER)
    assert added.data["regions"]["header_row"] == {"color": "FFFFFF", "bold": True, "shading": "1F3864",
                                                   "borders": {"bottom": "single 1.5 pt"}}
    assert added.data["regions"]["banded_rows"]["band_size"] == 1
    formatted = ok(box, session, "word_format_table", doc=d, table=table, scope="table", style="Ledger")
    # Rule 7: the regions its look turns off do not show; the look that shows them.
    [note] = [w for w in formatted.warnings if "turns off" in w]
    assert "total_row, first_column" in note and '"header_row", "banded_rows", "total_row", "first_column"' in note
    described = ok(box, session, "word_styles", doc=d, action="describe", name="Ledger").data
    assert described["shown_with_look"] == ["header_row", "total_row", "first_column", "banded_rows"]
    [off] = described["tables_with_look_off"]
    assert off["table"] == table and off["off"] == ["total_row", "first_column"]
    ok(box, session, "word_format_table", **{k: v for k, v in off["fix"].items() if k != "tool"}, doc=d)
    assert "tables_with_look_off" not in ok(box, session, "word_styles", doc=d, action="describe",
                                            name="Ledger").data


def test_modify_sets_what_is_given_keeps_the_rest_and_says_a_no_op(box):
    session, _ = with_table(box)
    d = "d1"
    ok(box, session, "word_styles", doc=d, action="add", name="Ledger", kind="table", regions=LEDGER)
    result = ok(box, session, "word_styles", doc=d, action="modify", name="Ledger",
                regions=[{"region": "header_row", "shading": "C00000"}, {"region": "first_column", "clear": True},
                         {"region": "last_column", "italic": True}])
    regions = result.data["regions"]
    assert regions["header_row"]["shading"] == "C00000" and regions["header_row"]["bold"] is True
    assert "first_column" not in regions and regions["last_column"] == {"italic": True}
    # Rule 3: what is already so is no change, not an error.
    again = ok(box, session, "word_styles", doc=d, action="modify", name="Ledger",
               regions=[{"region": "header_row", "shading": "C00000"}])
    assert again.summary == "No change to style Ledger" and again.changed == []
    assert any("already has this formatting" in w for w in again.warnings)
    # A border side "none" states no border there; shading "none" removes the fill.
    ok(box, session, "word_styles", doc=d, action="modify", name="Ledger",
       regions=[{"region": "header_row", "shading": "none", "borders": {"style": "none", "sides": ["inside_v"]}}])
    header = ok(box, session, "word_styles", doc=d, action="describe", name="Ledger").data["regions"]["header_row"]
    assert "shading" not in header and header["borders"]["inside_v"] == "none"


def test_regions_say_a_table_style_and_names_are_read_as_meant(box):
    session, _ = with_table(box)
    d = "d1"
    # Rule 1: regions given with no kind: a table style, said.
    result = ok(box, session, "word_styles", doc=d, action="add", name="Grid Look",
                regions=[{"region": "banded_columns", "shading": "EEEEEE", "band_size": 2}])
    assert result.warnings[0] == "regions given: added as a table style (kind table)"
    assert result.data["regions"]["banded_columns"] == {"shading": "EEEEEE", "band_size": 2}
    # The nulls a strict decoder fills in are nothing (rule 1).
    ok(box, session, "word_styles", doc=d, action="modify", name="Grid Look",
       regions=[{"region": "header_row", "shading": None, "borders": None, "band_size": None, "bold": True}])


def test_refusals_carry_the_way_out(box):
    session, _ = with_table(box)
    d = "d1"
    error = refused(box, session, "word_styles", doc=d, action="add", name="Mid", kind="table",
                    regions=[{"region": "third_column", "shading": "EEEEEE"}])
    assert error.field == "regions[0].region" and "banded_columns" in error.valid_options
    error = refused(box, session, "word_styles", doc=d, action="add", name="Para", kind="paragraph",
                    regions=[{"region": "header_row", "bold": True}])
    assert "table style" in error.message
    error = refused(box, session, "word_styles", doc=d, action="describe", name="Normal",
                    regions=[{"region": "header_row", "bold": True}])
    assert error.field == "regions"
    refused(box, session, "word_styles", doc=d, action="add", name="Mid", kind="table",
            regions=[{"region": "middle column", "bold": True}])
    error = refused(box, session, "word_styles", doc=d, action="add", name="Mid", kind="table",
                    regions=[{"region": "header_row", "borders": {"style": "single", "color": "accent1"}}])
    assert "color" in error.message


def test_band_size_is_the_banded_regions_and_a_stated_size_is_kept(box):
    session, _ = with_table(box)
    result = ok(box, session, "word_styles", doc="d1", action="add", name="Bands", kind="table",
                regions=[{"region": "header_row", "band_size": 2, "bold": True},
                         {"region": "even_banded_rows", "shading": "EEEEEE", "band_size": 3}])
    assert any("band_size is the banded regions'" in w for w in result.warnings)
    node = style_node(session, "Bands")
    assert node.find(f"{_W}tblPr/{_W}tblStyleRowBandSize").get(_W + "val") == "3"
    assert node.find(f"{_W}tblPr/{_W}tblStyleColBandSize") is None


# -- what Word draws, and docx2svg with it (rule 8, measured) ------------------------------------


def _drawn(data: bytes) -> tuple[dict, dict]:
    """docx2svg's drawing of the probe document: each label's fill, colour, bold, italic and
    left edge (pt); and per case the table's border lines (orientation, colour, rounded place
    from the case's first line)."""
    import docx2svg

    cells: dict = {}
    lines: dict = {}
    for page in docx2svg.convert_docx_to_svg(data):
        root = etree.fromstring(page.encode())
        rects = [(float(r.get("x") or 0), float(r.get("y") or 0), float(r.get("width") or 0), float(r.get("height") or 0), r)
                 for r in root.iter(SVG + "rect")]
        shading = [r for r in rects if r[4].get("data-docx-kind") == "cell-shading"]
        for text in root.iter(SVG + "text"):
            label = (text.text or "").strip()
            if not re.fullmatch(r"[A-Z]\d\d", label):
                continue
            x, y = float(text.get("x").split()[0]), float(text.get("y"))
            under = [r for r in shading if r[0] <= x <= r[0] + r[2] and r[1] <= y - 5 <= r[1] + r[3]]
            fill = min(under, key=lambda r: r[2] * r[3])[4].get("fill").lstrip("#").upper() if under else None
            cells[label] = {"fill": fill, "color": (text.get("fill") or "#000000").lstrip("#").upper(),
                            "bold": text.get("font-weight") in ("bold", "700"),
                            "italic": text.get("font-style") == "italic", "x": x * 72 / 300}
        for k, case in enumerate(probe.CASES):
            mine = [r for r in rects if r[4].get("data-docx-kind") == "table-border"
                    and f"/w:tbl[{k + 1}]/" in r[4].get("data-docx-path") + "/"]
            if mine:
                lines[case[0]] = [{"fill": r[4].get("fill").lstrip("#").upper(), "x": r[0] * 72 / 300,
                                   "y": r[1] * 72 / 300, "w": r[2] * 72 / 300, "h": r[3] * 72 / 300} for r in mine]
    return cells, lines


def _ranks(values: list[float], within: float = 2.0) -> dict[float, int]:
    """Each value's rank among the distinct ones, values within ``within`` pt one."""
    out: dict[float, int] = {}
    rank, last = -1, None
    for value in sorted(set(values)):
        if last is None or value - last > within:
            rank += 1
        out[value] = rank
        last = value
    return out


def _edges(lines: list[dict]) -> set:
    """Each line longer than a junction, by orientation, colour and place -- a horizontal
    one's rank among the horizontal lines' heights and the column its middle is over, a
    vertical one's rank among the vertical lines' places and the row (rank of its middle):
    the same for two drawings of a table however their origins differ."""
    long = [d for d in lines if max(d["w"], d["h"]) > 5]
    across = [d for d in long if d["w"] > d["h"]]
    down = [d for d in long if d["h"] >= d["w"]]
    ys = _ranks([d["y"] for d in across])
    columns = _ranks([d["x"] + d["w"] / 2 for d in across], 30)
    xs = _ranks([d["x"] for d in down])
    middles = _ranks([d["y"] + d["h"] / 2 for d in down], 5)
    return ({("h", d["fill"], ys[d["y"]], columns[d["x"] + d["w"] / 2]) for d in across}
            | {("v", d["fill"], xs[d["x"]], middles[d["y"] + d["h"] / 2]) for d in down})


@pytest.fixture(scope="module")
def drawn():
    return _drawn(probe.document())


@pytest.mark.parametrize("case", [c[0] for c in probe.CASES])
def test_docx2svg_draws_each_region_as_word_does(drawn, case):
    """Fill, text colour, bold and italic of every cell as Word drew them: which region wins
    (S, C, B, K, R), header rows (H, G), band sizes (N), looks off (O), merged cells (M), the
    whole table (W, V) and automatic text on dark fills (A)."""
    cells, _ = drawn
    for rc, word in OBSERVED[case].items():
        mine = cells[case + rc]
        assert {k: mine[k] for k in ("fill", "color", "bold", "italic")} == \
            {k: word[k] for k in ("fill", "color", "bold", "italic")}, (case + rc, word, mine)


def test_docx2svg_aligns_by_region_as_word_does(drawn):
    cells, _ = drawn
    word = OBSERVED["P"]
    for rc in word:
        assert abs((cells["P" + rc]["x"] - cells["P00"]["x"]) - (word[rc]["x"] - word["00"]["x"])) < 0.5, rc


@pytest.mark.parametrize("case", ["D", "E", "F", "Q"])
def test_docx2svg_draws_region_borders_as_word_does(drawn, case):
    """A row region's left and right at its rows' ends, inside_v between its cells; a column
    region's top and bottom at the table's; a band of two rows with inside_h between them."""
    _, lines = drawn
    assert _edges(lines[case]) == _edges(OBSERVED[f"{case}_lines"])


def test_the_effective_formatting_is_the_regions_as_word_draws_them(box):
    """``word_inspect``'s effective formatting is docx2svg's cascade: the column band over the
    row band (C), bands of two (N), the corner over the header row (K)."""
    session = box.session()
    session.open(probe.document(), "regions.docx")
    tables = [b for b, _ in word_docs.blocks(box, session, "d1") if b.startswith("t")]

    def run(case: str, r: int, c: int) -> SimpleNamespace:
        k = [x[0] for x in probe.CASES].index(case)
        return _effective(box, session, f"{tables[k]}/c{r},{c}")

    assert run("C", 1, 1).color == probe.COLOURS["band1Vert"] and run("C", 1, 1).size == probe.SIZES["band1Vert"] / 2
    assert run("C", 2, 1).color == probe.COLOURS["band1Vert"]
    assert run("N", 1, 3).color == probe.COLOURS["band2Vert"] and run("N", 1, 5).color == probe.COLOURS["band1Vert"]
    assert not run("K", 0, 0).bold and run("K", 0, 1).bold
    assert run("W", 0, 0).color == probe.COLOURS["own"] and run("V", 0, 0).color in ("auto", "000000")


def _effective(box, session, cell: str, doc: str = "d1") -> SimpleNamespace:
    """A cell's first run's text and effective font, size, colour and bold, as word_inspect
    says them."""
    block = ok(box, session, "word_inspect", doc=doc, target=cell).data["blocks"][0]
    effective, run = block["effective"], (block.get("runs") or [{}])[0]
    return SimpleNamespace(text=run.get("t", ""), font=run.get("font", effective["font"]),
                           size=run.get("sz", effective["sz"]), color=run.get("color", effective["color"]),
                           bold=run.get("b", effective["b"]))


# -- a new table in another's frame (rules 2, 5) ---------------------------------------------------


def frame_sample(box):
    """A made-up four-column table: a header row, two body rows that alternate a shaded third
    column, a total row; Arial 8 coloured text, the third column centred: the session and the
    table."""
    session = box.session(clock=CLOCK)
    session.open(word_docs.blank(), "frame.docx")
    texts = [["Item", "Count", "Mid", "Note"], ["a", "1", "x", "n"], ["b", "2", "y", "m"], ["Sum", "3", "", ""]]
    markdown = "\n".join(["| " + " | ".join(texts[0]) + " |", "|---|---|---|---|"]
                         + ["| " + " | ".join(row) + " |" for row in texts[1:]])
    first = word_docs.paragraph_ids(box, session, "d1")[0]
    made = ok(box, session, "word_insert_markdown", doc="d1", at=f"after:{first}", markdown=markdown)
    table = next(b for b in made.data["blocks"] if b.startswith("t:"))

    def fmt(**arguments):
        ok(box, session, "word_format_table", doc="d1", table=table, **arguments)

    fmt(scope="table", look=["header_row", "total_row"], header_rows=1)
    for r, fill in ((0, "1F3864"), (1, "DEEAF6"), (2, "BDD7EE"), (3, "1F3864")):
        fmt(scope="cell", row=r, column=2, shading=fill, vertical_alignment="center")
    for r in range(4):
        cells = [f"{table}/c{r},{c}" for c in range(4) if texts[r][c]]
        ok(box, session, "word_format", doc="d1", targets=cells, font="Arial", size=8, color="2F5496", bold=(r == 0))
    return session, table


def _rows(session, table: str):
    return word_docs.element(session, "d1", table).findall(_W + "tr")


def test_a_markdown_table_takes_the_frame_rows_cells_and_text_formatting(box):
    session, sample = frame_sample(box)
    markdown = ("| Name | Qty | **Key** | Remark |\n|---|---|---|---|\n" + "".join(
        f"| r{k} | {k} | k{k} | |\n" for k in range(5)) + "| Total | 10 | | |\n")
    result = ok(box, session, "word_insert_markdown", doc="d1", at="end", markdown=markdown, like=sample)
    new = next(b for b in result.data["blocks"] if b.startswith("t:"))
    rows = _rows(session, new)
    old = _rows(session, sample)
    assert len(rows) == 7
    # Header first, body rows repeated in their order (two alternating looks), total last.
    fills = [row.findall(_W + "tc")[2].find(f"{_W}tcPr/{_W}shd").get(_W + "fill") for row in rows]
    assert fills == ["1F3864", "DEEAF6", "BDD7EE", "DEEAF6", "BDD7EE", "DEEAF6", "1F3864"]
    assert rows[0].find(f"{_W}trPr/{_W}tblHeader") is not None
    assert etree.tostring(rows[1].find(_W + "tc").find(_W + "tcPr")) == \
        etree.tostring(old[1].find(_W + "tc").find(_W + "tcPr"))
    # The table's own properties and grid are the sample's.
    for tag in ("tblPr", "tblGrid"):
        assert etree.tostring(word_docs.element(session, "d1", new).find(_W + tag)) == \
            etree.tostring(word_docs.element(session, "d1", sample).find(_W + tag))
    # The text takes each cell's text formatting, the Markdown's emphasis over it.
    run = _effective(box, session, f"{new}/c1,0")
    assert (run.text, run.font, run.size, run.color) == ("r0", "Arial", 8.0, "2F5496")
    key = _effective(box, session, f"{new}/c0,2")
    assert key.text == "Key" and key.bold and key.font == "Arial"
    texts = word_docs.table_texts(session, "d1", new)
    assert texts[1][3] == ""
    # No text from the sample.
    assert "Sum" not in "".join(text for row in texts for text in row)


def test_a_frame_with_no_text_is_an_empty_markdown_table(box):
    session, sample = frame_sample(box)
    result = ok(box, session, "word_insert_markdown", doc="d1", at="end", like=sample,
                markdown="| | | | |\n|---|---|---|---|\n| | | | |\n| | | | |\n| | | | |\n")
    new = next(b for b in result.data["blocks"] if b.startswith("t:"))
    rows = _rows(session, new)
    assert len(rows) == 4 and rows[0].find(f"{_W}trPr/{_W}tblHeader") is not None
    assert all(not "".join(t.text or "" for t in row.iter(_W + "t")) for row in rows)
    # Each empty cell's mark is formatted as the sample's first run: text typed later takes it.
    mark = rows[1].find(f"{_W}tc/{_W}p/{_W}pPr/{_W}rPr")
    assert mark.find(_W + "rFonts").get(_W + "ascii") == "Arial" and mark.find(_W + "sz").get(_W + "val") == "16"


def test_a_frame_copy_is_tracked_as_an_insertion_and_undone_in_one_step(box):
    session, sample = frame_sample(box)
    d = "d1"
    ok(box, session, "word_set_tracking", doc=d, on=True, author="Editor A")
    result = ok(box, session, "word_insert_markdown", doc=d, at="end", like=sample,
                markdown="| A | B | C | D |\n|---|---|---|---|\n| 1 | 2 | 3 | 4 |\n")
    new = next(b for b in result.data["blocks"] if b.startswith("t:"))
    table = word_docs.element(session, d, new)
    assert all(row.find(f"{_W}trPr/{_W}ins") is not None for row in table.findall(_W + "tr"))
    assert all(p.find(f"{_W}pPr/{_W}rPr/{_W}ins") is not None for p in table.iter(_W + "p"))
    assert all(r.getparent().tag == _W + "ins" for r in table.iter(_W + "r"))
    changes = ok(box, session, "word_changes", doc=d, action="list").data
    assert changes
    ok(box, session, "undo", doc=d)
    assert new not in ok(box, session, "word_read", doc=d).data


def test_frames_refuse_what_they_cannot_copy_with_the_way_out(box):
    session, sample = frame_sample(box)
    d = "d1"
    error = refused(box, session, "word_insert_markdown", doc=d, at="end", like=sample,
                    markdown="| A | B |\n|---|---|\n| 1 | 2 |\n")
    assert error.field == "like" and '"columns": 4' in error.message and "| | | | |" in error.message
    error = refused(box, session, "word_insert_markdown", doc=d, at="end", like=sample, markdown="Text only.")
    assert "has none to write in its frame" in error.message and "| | | | |" in error.message
    error = refused(box, session, "word_insert_markdown", doc=d, at="end", like=[sample, sample],
                    markdown="| A | B | C | D |\n|---|---|---|---|\n")
    assert "one table" in error.message
    # A sample merged down in its body rows: refused until round 59 (R136), now its merges
    # follow the Markdown's empty cells (tests/test_merged_rows_r59.py).
    merged, table = frame_sample(box)
    ok(box, merged, "word_edit_table", doc="d1", table=table, action="merge", first={"row": 1, "col": 0},
       last={"row": 2, "col": 0})
    ok(box, merged, "word_insert_markdown", doc="d1", at="end", like=table,
       markdown="| A | B | C | D |\n|---|---|---|---|\n| 1 | 2 | 3 | 4 |\n")
    # Merged across: the text goes in the merged cell's first column, the others empty.
    across, table = frame_sample(box)
    ok(box, across, "word_edit_table", doc="d1", table=table, action="merge", first={"row": 0, "col": 0},
       last={"row": 0, "col": 1})
    error = refused(box, across, "word_insert_markdown", doc="d1", at="end", like=table,
                    markdown="| A | B | C | D |\n|---|---|---|---|\n| 1 | 2 | 3 | 4 |\n")
    assert "spans columns 0-1" in error.message
    result = ok(box, across, "word_insert_markdown", doc="d1", at="end", like=table,
                markdown="| A | | C | D |\n|---|---|---|---|\n| 1 | 2 | 3 | 4 |\n")
    new = next(b for b in result.data["blocks"] if b.startswith("t:"))
    first = _rows(across, new)[0].findall(_W + "tc")
    assert len(first) == 3 and first[0].find(f"{_W}tcPr/{_W}gridSpan").get(_W + "val") == "2"


def test_like_takes_a_table_and_paragraphs_together(box):
    session, sample = frame_sample(box)
    made = ok(box, session, "word_insert_markdown", doc="d1", at="end", markdown="- An item")
    item = made.data["blocks"][0]
    result = ok(box, session, "word_insert_markdown", doc="d1", at="end", like=[item, sample],
                markdown="- one\n\n| A | B | C | D |\n|---|---|---|---|\n| 1 | 2 | 3 | 4 |\n")
    assert sum(b.startswith("t:") for b in result.data["blocks"]) == 1
