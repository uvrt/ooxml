"""word_design_facts' ``declared_styles`` (production round 21, T7): every declared style,
used or not, resolved, theme colours as the RGB Word draws (``tools/theme_tint_probe.py``,
``tests/observations/theme-tints-word.json``)."""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS
import word_docs
from conftest import FIXTURE_DIR  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SWATCHES = json.loads((ROOT / "tests" / "observations" / "theme-tints-word.json").read_text())["swatches"]


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def _ok(box, session, tool: str, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, result.to_json()
    return result


def _with_styles(styles: str) -> bytes:
    return word_docs.replaced(word_docs.blank(), {
        "word/styles.xml": lambda xml: xml.replace("</w:styles>", styles + "</w:styles>")})


def _declared(box, session, d: str) -> dict[str, dict]:
    facts = _ok(box, session, "word_design_facts", doc=d, include=["declared_styles"]).data
    return {s["name"]: s for s in facts["declared_styles"]}


def test_a_theme_colour_with_its_tint_or_shade_is_what_word_draws(box):
    """Each swatch's colour as a style's text colour (its w:val a decoy): the RGB Word draws."""
    styles = ""
    for k, swatch in enumerate(SWATCHES):
        assert swatch["word"] == swatch["fill"]
        tint = f' w:themeTint="{swatch["tint"]}"' if swatch["tint"] else ""
        shade = f' w:themeShade="{swatch["shade"]}"' if swatch["shade"] else ""
        styles += (f'<w:style w:type="character" w:customStyle="1" w:styleId="Swatch{k}"><w:name w:val="Swatch {k}"/>'
                   f'<w:rPr><w:color w:val="FF00FF" w:themeColor="{swatch["theme"]}"{tint}{shade}/></w:rPr></w:style>')
    session = box.session()
    declared = _declared(box, session, session.open(_with_styles(styles), "swatches.docx"))
    for k, swatch in enumerate(SWATCHES):
        assert declared[f"Swatch {k}"]["look"]["color"] == swatch["word"], swatch


def _made(box, session) -> str:
    d = session.open(word_docs.blank(), "styles.docx")
    _ok(box, session, "word_styles", doc=d, action="add", name="Chapter title", based_on="heading 1", bold=True)
    _ok(box, session, "word_styles", doc=d, action="add", name="Quiet note", based_on="Normal", italic=True)
    _ok(box, session, "word_styles", doc=d, action="add", name="Marker", kind="character", bold=True)
    first = word_docs.paragraph_ids(box, session, d)[0]
    _ok(box, session, "word_set_text", doc=d, target=first, text="Hello")
    return d


def test_declared_styles_lists_used_and_unused_styles_resolved(box):
    session = box.session()
    d = _made(box, session)
    facts = _ok(box, session, "word_design_facts", doc=d, include=["declared_styles"]).data
    styles = {s["name"]: s for s in facts["declared_styles"]}
    assert styles["Normal"]["used"] and styles["Normal"]["default"]
    assert set(styles["Normal"]["look"]) >= {"font", "size", "color", "space_after"}
    chapter = styles["Chapter title"]
    assert chapter["used"] is False and chapter["based_on"] == ["heading 1", "Normal"]
    # Its colour is the theme's accent1 shaded BF, as Word draws it (the heading's w:val agrees).
    assert chapter["look"]["color"] == "0F4761" and chapter["look"]["theme_color"] == "accent1 shade BF"
    assert chapter["look"]["outline_level"] == 1 and chapter["look"]["bold"] is True
    assert styles["Quiet note"]["look"] == {"italic": True}
    assert styles["Marker"]["type"] == "character" and styles["Marker"]["look"] == {"bold": True}
    # Used styles come first.
    used = [s["used"] for s in facts["declared_styles"]]
    assert used == sorted(used, reverse=True)
    # Not in the default kinds.
    assert "declared_styles" not in _ok(box, session, "word_design_facts", doc=d).data


#: A made-up table style: accent1 borders, a header row filled accent1 with white bold text,
#: row bands a tint of accent1, the fills' and colours' w:val decoys.
TABLE_STYLE = (
    '<w:style xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" w:type="table" '
    'w:styleId="Ledger"><w:name w:val="Ledger"/><w:basedOn w:val="TableNormal"/><w:tblPr>'
    '<w:tblStyleRowBandSize w:val="1"/><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" '
    'w:color="FF00FF" w:themeColor="accent1"/><w:bottom w:val="single" w:sz="4" w:space="0" '
    'w:color="FF00FF" w:themeColor="accent1"/></w:tblBorders></w:tblPr>'
    '<w:tblStylePr w:type="firstRow"><w:rPr><w:b/><w:color w:val="000000" w:themeColor="background1"/></w:rPr>'
    '<w:tcPr><w:shd w:val="clear" w:color="auto" w:fill="00FF00" w:themeFill="accent1"/></w:tcPr></w:tblStylePr>'
    '<w:tblStylePr w:type="band1Horz"><w:tcPr><w:shd w:val="clear" w:color="auto" w:fill="00FF00" '
    'w:themeFill="accent1" w:themeFillTint="33"/></w:tcPr></w:tblStylePr></w:style>')


def test_a_table_style_lists_its_parts_and_places_with_theme_colours(box):
    session = box.session()
    d = session.open(_with_styles(TABLE_STYLE.replace(
        ' xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"', "")), "ledger.docx")
    styles = _declared(box, session, d)
    ledger = styles["Ledger"]
    assert ledger["type"] == "table" and ledger["used"] is False and ledger["parts"] == ["firstRow", "band1Horz"]
    header, band = ledger["places"]["header_row"], ledger["places"]["band1"]
    # The fills and colours are the theme's, not the decoys beside them.
    assert (header["fill"], header["theme_fill"]) == ("156082", "accent1")
    assert header["text"] == {"color": "FFFFFF", "theme_color": "background1", "bold": True}
    assert (band["fill"], band["theme_fill"]) == ("C1E4F5", "accent1 tint 33")
    assert "single 0.5pt 156082 (accent1)" in header["borders"].values()


#: Lane P2c14/P2c15: where the Rust session serves word_design_facts, it also names the theme
#: colour of a band cell's edge drawn by a neighbouring place's border (the header row's
#: bottom, ``w:themeColor="background1"``); docx-agent names only the colours of the place's own
#: conditions.  Rust's is the accurate answer (known-differences.toml, mode style).  Per family
#: of Word's table styles in ``sample-10pages.docx``: the border, and the places and edges that
#: show it.
NEIGHBOUR_DRAWN = {
    "Dark List": ("single 2.25pt FFFFFF", {"band1": ["top,left"], "band2": ["left"]}),
    "Colorful List": ("single 1.5pt FFFFFF",
                      {"band1": ["top"], "first_column": ["top"], "last_column": ["top"]}),
}


def test_the_tool_pages_declared_styles_by_size(box):
    path = FIXTURE_DIR / "wordto" / "sample-10pages.docx"
    session = box.session()
    d1 = session.open(path.read_bytes(), "template.docx")
    seen, cursor, pages = [], None, 0
    while True:
        arguments = {"doc": d1, "include": ["declared_styles", "theme"]}
        if cursor:
            arguments["cursor"] = cursor
        result = _ok(box, session, "word_design_facts", **arguments)
        assert len(json.dumps(result.data)) <= box.limits.max_result_chars - 1500
        assert ("theme" in result.data) == (cursor is None)     # later pages: the styles only
        seen += result.data["declared_styles"]
        pages += 1
        cursor = result.next_cursor
        if cursor is None:
            break
    assert pages > 1 and len({s["name"] for s in seen}) == len(seen)
    assert result.summary.endswith(f"of {len(seen)}")
    # The theme colour of a band cell's edge a neighbouring place's border draws, named.
    labelled = 0
    for style in seen:
        family = next((f for f in NEIGHBOUR_DRAWN if style["name"].startswith(f)), None)
        if family is None:
            continue
        border, places = NEIGHBOUR_DRAWN[family]
        for place, edges in places.items():
            for edge in edges:
                assert style["places"][place]["borders"][edge] == f"{border} (background1)", (style["name"], place)
        labelled += 1
    assert labelled == 14       # Dark List and Colorful List, each with its six accents
