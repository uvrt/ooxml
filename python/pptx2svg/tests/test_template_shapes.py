"""What a slide shows from its layout and master: their shapes, and its placeholders' fill
and outline.

Measured in PowerPoint 16 for Mac, 23 November 2026 (pptx-agent round 60, R144; slide show
captures): a layout's and a master's shapes are drawn under the slide's; a layout's
``showMasterSp="0"`` hides the master's; a slide's ("Hide background graphics") hides
both.  A slide placeholder that states no fill or outline is drawn with its layout
placeholder's, else that one's master placeholder's -- a master body's fill through a layout
subtitle that states none -- and its own ``a:noFill`` hides it.

Every deck is ``sample.pptx`` with standard-library zip edits.
"""

from __future__ import annotations

import io
import zipfile
from pathlib import Path

from pptx2svg import ConvertOptions, convert_pptx_to_model
from pptx2svg import model as m

SAMPLE = Path(__file__).parent / "fixtures" / "sample.pptx"
SLIDE = "ppt/slides/slide1.xml"
LAYOUT = "ppt/slideLayouts/slideLayout1.xml"
MASTER = "ppt/slideMasters/slideMaster1.xml"

NAVY = '<a:solidFill><a:srgbClr val="003366"/></a:solidFill>'
WINE = '<a:solidFill><a:srgbClr val="660033"/></a:solidFill>'
RED_LINE = '<a:ln w="76200"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:ln>'


def _band(number: int, y: int, fill: str) -> str:
    return (f'<p:sp><p:nvSpPr><p:cNvPr id="{number}" name="Band {number}"/><p:cNvSpPr/>'
            f'<p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="{y}"/>'
            '<a:ext cx="9144000" cy="1000000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/>'
            f'</a:prstGeom>{fill}</p:spPr></p:sp>')


def _with_band(number: int, y: int, fill: str):
    return lambda xml: xml.replace("</p:spTree>", _band(number, y, fill) + "</p:spTree>", 1)


def _root_attribute(tag: str, name: str, value: str):
    return lambda xml: xml.replace(f"<{tag} ", f'<{tag} {name}="{value}" ', 1)


def _chain(*edits):
    def apply(xml: str) -> str:
        for edit in edits:
            xml = edit(xml)
        return xml
    return apply


def _model(edits: dict) -> m.Slide:
    with zipfile.ZipFile(SAMPLE) as source:
        parts = {name: source.read(name) for name in source.namelist()}
    for name, edit in edits.items():
        parts[name] = edit(parts[name].decode()).encode()
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, payload in parts.items():
            archive.writestr(name, payload)
    return convert_pptx_to_model(out.getvalue(), ConvertOptions()).slides[0]


def _ids(slide: m.Slide) -> list[str]:
    return [element.element_id or "" for element in slide.elements]


def _bands(**more) -> dict:
    edits = {LAYOUT: _with_band(90, 3000000, NAVY), MASTER: _with_band(91, 4500000, WINE)}
    for part, edit in more.items():
        name = {"slide": SLIDE, "layout": LAYOUT}[part]
        edits[name] = _chain(edits[name], edit) if name in edits else edit
    return edits


def test_the_slide_shows_its_masters_then_its_layouts_shapes_under_its_own():
    ids = _ids(_model(_bands()))
    assert ids.index("mst:91") < ids.index("lay:90") < min(
        index for index, key in enumerate(ids) if not key.startswith(("mst:", "lay:")))


def test_a_layout_hiding_master_shapes_keeps_its_own():
    ids = _ids(_model(_bands(layout=_root_attribute("p:sldLayout", "showMasterSp", "0"))))
    assert "lay:90" in ids and "mst:91" not in ids


def test_a_slide_hiding_background_graphics_hides_its_layouts_shapes_too():
    slide = _model(_bands(slide=_root_attribute("p:sld", "showMasterSp", "0")))
    assert not any(key.startswith(("mst:", "lay:")) for key in _ids(slide))
    assert slide.show_master_sp is False


def _placeholder(slide: m.Slide, kind: str) -> m.ShapeElement:
    return next(element for element in slide.elements
                if isinstance(element, m.ShapeElement) and element.placeholder_type == kind)


def _in_sppr(xml: str, name: str, extra: str) -> str:
    """``xml`` with ``extra`` closing the ``p:spPr`` of the shape called ``name``."""
    start = xml.index(f'name="{name}"')
    close = xml.index("</p:spPr>", start)
    return xml[:close] + extra + xml[close:]


def test_a_placeholder_takes_its_layout_placeholders_fill_and_outline():
    slide = _model({LAYOUT: lambda xml: _in_sppr(xml, "Title 1", NAVY + RED_LINE)})
    title = _placeholder(slide, "ctrTitle")
    assert title.fill.color.hex == "#003366"
    assert title.outline.fill.color.hex == "#ff0000" and title.outline.width == 76200


def test_a_placeholder_takes_its_master_placeholders_fill_through_a_layout_stating_none():
    slide = _model({MASTER: lambda xml: _in_sppr(xml, "Text Placeholder 2", WINE)})
    assert _placeholder(slide, "subTitle").fill.color.hex == "#660033"


def test_a_placeholders_own_no_fill_hides_the_inherited_one():
    slide = _model({
        MASTER: lambda xml: _in_sppr(xml, "Text Placeholder 2", WINE),
        SLIDE: lambda xml: xml.replace(
            '<p:ph type="subTitle" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/>',
            '<p:ph type="subTitle" idx="1"/></p:nvPr></p:nvSpPr><p:spPr><a:noFill/></p:spPr>', 1),
    })
    assert isinstance(_placeholder(slide, "subTitle").fill, m.NoFill)
