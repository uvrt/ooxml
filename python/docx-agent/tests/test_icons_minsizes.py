"""Round 17, P6: an icon from the application's asset catalogue, inline or floating, tinted
as Word draws a recoloured SVG (measured: agent-work feedback-round17/measurements-B.md), and
the application's minimum text sizes by kind, which check and the writers both read.
Made-up icons and documents."""

from __future__ import annotations

import io
import json
import re
import struct
import zipfile

import pytest

pytest.importorskip("ooxml_edit.pictures", reason="needs ooxml-edit 0.68 (ooxml_edit.pictures)")

from ooxml_edit.tools import FolderCatalogue, Toolbox  # noqa: E402
from layout_reads import ids_of, table_ids  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS, docx_format  # noqa: E402
import word_docs  # noqa: E402

ICON = (b'<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">'
        b'<circle cx="12" cy="12" r="10" fill="#000000"/><path fill="#FFFFFF" d="M11 6h2v8h-2zM11 16h2v2h-2z"/>'
        b'</svg>')
WIDE = b'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 20"><rect width="40" height="20"/></svg>'
W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
WP = "{http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing}"
A = "{http://schemas.openxmlformats.org/drawingml/2006/main}"


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def _parts(session, doc: str) -> dict[str, bytes]:
    with zipfile.ZipFile(io.BytesIO(word_docs.saved(session, doc))) as archive:
        return {name: archive.read(name) for name in archive.namelist()}


def _blank(box) -> tuple[object, str, str]:
    session = box.session()
    d = session.open(word_docs.blank(), "icons.docx")
    first = ids_of(session, d)[0]
    assert box.dispatch(session, "word_set_text", {"doc": d, "target": first, "text": "Icon: "}).ok
    return session, d, first


def _picture(box, session, d: str, at: str, image: bytes, **arguments):
    blob = session.add_blob(image, "picture.svg" if image.lstrip().startswith(b"<") else "picture.png")
    return box.dispatch(session, "word_drawings", {"doc": d, "action": "insert_picture", "image": blob, "at": at,
                                                   **arguments})


def _size(session, d: str, drawing: str) -> tuple[float, float]:
    """A drawing's extent, in points."""
    root = word_docs.tree(word_docs.saved(session, d))
    frame = next(p for p in root.iter(WP + "docPr") if p.get("id") == drawing.partition(":")[2]).getparent()
    extent = frame.find(WP + "extent")
    return int(extent.get("cx")) / 12700, int(extent.get("cy")) / 12700


def _theme_colour(session, d: str, name: str) -> str:
    root = word_docs.tree(word_docs.saved(session, d), "word/theme/theme1.xml")
    return next(root.iter(A + name)).find(A + "srgbClr").get("val")


# -- an SVG picture, tinted ----------------------------------------------------------------------


def test_an_svg_picture_is_stored_as_word_stores_one(box):
    session, d, first = _blank(box)
    made = _picture(box, session, d, f"{first}@6", ICON, alt_text="A made-up icon")
    assert made.ok, made.to_json()
    assert _size(session, d, made.data["id"]) == (18.0, 18.0)          # 24 px at 96 dpi, as Word places it
    parts = _parts(session, d)
    media = sorted(n for n in parts if n.startswith("word/media/"))
    assert [n.rsplit(".", 1)[1] for n in media] == ["png", "svg"]
    svg_name = next(n for n in media if n.endswith(".svg"))
    png_name = next(n for n in media if n.endswith(".png"))
    assert parts[svg_name] == ICON
    assert struct.unpack(">II", parts[png_name][16:24]) == (48, 48)   # twice its size
    assert b'<Default Extension="svg" ContentType="image/svg+xml"/>' in parts["[Content_Types].xml"]
    xml = parts["word/document.xml"].decode()
    assert re.search(r'<wp:docPr id="\d+" name="Graphic \d+"', xml)
    assert '<a:ext uri="{96DAC541-7B7A-43D3-8B79-37D633B846F1}"><asvg:svgBlip' in xml
    rels = parts["word/_rels/document.xml.rels"].decode()
    assert re.search(r'Type="[^"]+/relationships/image" Target="media/[^"]+\.svg"', rels)
    assert word_docs.validation(box, session, d)["new"] == []
    wide = _picture(box, session, d, f"{first}@0", WIDE, width=20, alt_text="A bar")
    assert wide.ok and _size(session, d, wide.data["id"]) == (20.0, 10.0)


def test_a_tint_is_a_theme_colour_with_its_class_or_a_hex_colour(box):
    session, d, first = _blank(box)
    assert _picture(box, session, d, f"{first}@6", ICON, width=12, alt_text="Icon", color="accent2").ok
    svg = next(v for k, v in _parts(session, d).items() if k.endswith(".svg")).decode()
    accent2 = _theme_colour(session, d, "accent2")
    assert f".MsftOfcThm_Accent2_Fill_v2 {{\n fill:#{accent2}; \n}}" in svg
    assert svg.count('class="MsftOfcThm_Accent2_Fill_v2"') == 2 and svg.count(f'fill="#{accent2}"') == 2
    session, d, first = _blank(box)
    assert _picture(box, session, d, f"{first}@6", ICON, alt_text="Icon", color="1f4e79").ok
    svg = next(v for k, v in _parts(session, d).items() if k.endswith(".svg")).decode()
    assert "MsftOfcThm" not in svg and svg.count('fill="#1F4E79"') == 2
    session, d, first = _blank(box)
    assert _picture(box, session, d, f"{first}@6", ICON, alt_text="Icon", color="Text 1").ok
    assert "MsftOfcThm_Text1_Fill_v2" in next(v for k, v in _parts(session, d).items() if k.endswith(".svg")).decode()


def test_what_cannot_be_tinted_or_read_is_refused(box):
    from test_tools_word import png as make_png

    session, d, first = _blank(box)
    for image, arguments, said in (
            (make_png(), {"color": "accent1"}, "only an SVG picture can be tinted"),
            (ICON, {"color": "purple-ish"}, "or RRGGBB"),
            (b'<!DOCTYPE svg><svg xmlns="http://www.w3.org/2000/svg"/>', {}, "document type"),
            (b'<svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>', {}, "states no size")):
        refused = _picture(box, session, d, f"{first}@0", image, alt_text="x", **arguments)
        assert not refused.ok and said in refused.error.message, refused.to_json()


# -- the catalogue ---------------------------------------------------------------------------------


@pytest.fixture
def icons(tmp_path):
    folder = tmp_path / "icons"
    (folder / "icon").mkdir(parents=True)
    (folder / "icon" / "alert-circle.svg").write_bytes(ICON)
    (folder / "index.json").write_text(json.dumps([
        {"id": "icon/alert-circle", "file": "icon/alert-circle.svg", "name": "Alert circle",
         "keywords": ["alert", "warning", "circle"]}]))
    return FolderCatalogue(folder)


def test_without_a_catalogue_nothing_is_sent_for_it():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as box:
        names = {d["name"]: d for d in box.definitions("anthropic", defer=False)}
    assert "word_find_asset" not in names
    assert "asset" not in names["word_drawings"]["input_schema"]["properties"]
    assert "color" in names["word_drawings"]["input_schema"]["properties"]


def test_a_catalogue_is_refused_with_the_way_out(icons):
    """An asset catalogue is not supported yet: a picture goes to word_drawings as a blob."""
    with pytest.raises(ValueError, match="not supported yet.*image blob"):
        docx_format(assets=icons)


def test_an_icon_in_a_cell_tinted_and_floating(box):
    """The catalogue's icon given as a blob: in a cell, tinted, and floating beside it."""
    session = box.session()
    d = session.open(word_docs.with_body('<w:p/>'), "icons.docx")
    assert box.dispatch(session, "word_insert_markdown", {"doc": d, "at": "end",
                                                          "markdown": "| Item | Note |\n| --- | --- |\n| One | a |"}).ok
    table = table_ids(box, session, d)[0]
    result = _picture(box, session, d, f"{table}/c1,0", ICON, width=12, color="accent2", alt_text="alert circle")
    assert result.ok, result.error and result.error.to_json()
    assert _size(session, d, result.data["id"]) == (12.0, 12.0)
    svg = next(v for k, v in _parts(session, d).items() if k.endswith(".svg")).decode()
    assert "MsftOfcThm_Accent2_Fill_v2" in svg
    floating = _picture(box, session, d, f"{table}/c1,1", ICON, width=24, wrap="square", align="right",
                        alt_text="Warning")
    assert floating.ok, floating.to_json()
    root = word_docs.tree(word_docs.saved(session, d))
    frame = next(p for p in root.iter(WP + "docPr") if p.get("id") == floating.data["id"].partition(":")[2])
    assert frame.getparent().tag == WP + "anchor"
    refused = _picture(box, session, d, f"{table}/c1,0", ICON, color="x", alt_text="x")
    assert not refused.ok and refused.error.field == "color"
    assert word_docs.validation(box, session, d)["new"] == []


# -- minimum text sizes by kind -------------------------------------------------------------------


def test_the_setting_is_read_once_for_everything():
    docx_format(min_text_size=9)
    docx_format(min_text_size={"table": 8, "caption": None})
    for bad in ({"chart": 9}, {}, 0, "9", True, {"slides": 10}):
        with pytest.raises(ValueError):
            docx_format(min_text_size=bad)


def _box_session(box, markdown: str):
    session = box.session()
    d = session.open(word_docs.blank(), "sizes.docx")
    result = box.dispatch(session, "word_insert_markdown", {"doc": d, "at": "end", "markdown": markdown})
    assert result.ok, result.error and result.error.to_json()
    return session, d, result


def test_writers_and_check_report_text_below_its_kinds_minimum():
    markdown = "Body text.\n\n| Item | Note |\n| --- | --- |\n| One | a |"
    with Toolbox(TOOLS, formats=[docx_format(min_text_size={"table": 30})], groups=GROUPS) as box:
        session, d, result = _box_session(box, markdown)
        small = result.data["small_text"]
        assert small and {e["kind"] for e in small} == {"table"} and all(e["min"] == 30 for e in small)
        assert any(w.startswith("small_text:") for w in result.warnings)
        checked = box.dispatch(session, "check", {"doc": d, "include": ["layout"]})
        assert checked.ok, checked.to_json()
        layout = checked.data["layout"]
        assert layout["small_text"] and {e["kind"] for e in layout["small_text"]} == {"table"}
        assert all(e["min"] == 30 for e in layout["small_text"])
        # The same paragraphs, whichever reads it.
        assert {e["para"] for e in layout["small_text"]} == {e["para"] for e in small}
    # A caption has a minimum of its own.
    with Toolbox(TOOLS, formats=[docx_format(min_text_size={"caption": 30})], groups=GROUPS) as box:
        session, d, result = _box_session(box, "Body text.")
        assert "small_text" not in result.data
        body = next(p for p, text in word_docs.paragraphs(session, d) if text == "Body text.")
        assert box.dispatch(session, "word_fields", {"doc": d, "action": "insert_caption", "after": body,
                                                     "label": "Table", "text": "Made-up items"}).ok
        found = box.dispatch(session, "check", {"doc": d, "include": ["layout"]}).data["layout"]["small_text"]
        assert [e["kind"] for e in found] == ["caption"]


def test_without_the_setting_nothing_changes():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as box:
        session, d, result = _box_session(box, "Body text.\n\n| a | b |\n| --- | --- |\n| 1 | 2 |")
        assert "small_text" not in result.data and not any(w.startswith("small_text") for w in result.warnings)
        layout = box.dispatch(session, "check", {"doc": d, "include": ["layout"]}).data["layout"]
        assert "small_text" not in layout
        # check's own min_size is one number for every kind, as before: no "min" per entry.
        numbered = box.dispatch(session, "check", {"doc": d, "min_size": 30}).data["layout"]["small_text"]
        assert numbered and all("min" not in e for e in numbered)
