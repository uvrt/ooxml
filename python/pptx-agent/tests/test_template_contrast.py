"""Contrast over what a slide shows from its layout and master; a placeholder's inherited
fill; a Slide object kept across a new slide.

Production round 60 (R144).  Measured in PowerPoint 16 for Mac, 23 November 2026 (slide
show captures; pptx2svg's ``tests/test_template_shapes.py`` holds the renderer's side):

* a layout's and a master's shapes are drawn under the slide's -- white text on a layout's
  navy band reads; a layout's ``showMasterSp="0"`` hides the master's shapes, a slide's
  ("Hide background graphics") hides both;
* a slide placeholder stating no fill is drawn with its layout placeholder's, else that
  one's master placeholder's; its own ``a:noFill`` hides it.

The decks are made up here.
"""

from __future__ import annotations

from images import ORANGE

from pptx_agent import Document

PT = 12700
NAVY, WINE = "#003366", "#660033"


def _white_box(slide, y: int, text: str):
    box = slide.add_textbox(40 * PT, y * PT, 400 * PT, 40 * PT, text)
    box.text_frame.paragraphs[0].runs[0].color = "#FFFFFF"
    return box


def _banded(hide: str | None = None):
    """A Title Only slide over a navy layout band (y 300-400 pt) and a wine master band
    (y 420-520 pt), white text on each; ``hide`` ``"layout"`` or ``"slide"`` sets that
    part's ``showMasterSp="0"``."""
    deck = Document.new()
    layout = deck.layout("Title Only")
    layout_band = layout.add_shape("rect", 0, 300 * PT, 960 * PT, 100 * PT, fill=NAVY,
                                   line="none")
    master_band = layout.master.add_shape("rect", 0, 420 * PT, 960 * PT, 100 * PT,
                                          fill=WINE, line="none")
    slide = deck.add_slide(layout)
    on_layout = _white_box(slide, 330, "White on the layout band")
    on_master = _white_box(slide, 450, "White on the master band")
    if hide is not None:
        part = layout.part_path if hide == "layout" else slide.part_path
        deck.package.tree(part).set("showMasterSp", "0")
        deck.package.mark_dirty(part)
    return deck, slide, layout_band, master_band, on_layout, on_master


def _entries(slide) -> dict:
    return {entry.shape: entry for entry in slide.contrast().entries}


def test_text_on_a_layout_or_master_band_is_measured_against_the_band():
    deck, slide, _, _, on_layout, on_master = _banded()
    assert slide.contrast().entries == ()
    # The band is what is behind it: grey text on navy is too faint, and says so.
    on_layout.text_frame.paragraphs[0].runs[0].color = "#203050"
    entry = _entries(slide)[on_layout.id]
    assert (entry.behind, entry.over) == (NAVY, deck.layout("Title Only").shapes[-1].id)
    assert entry.over.startswith("L") and on_master.id not in _entries(slide)


def test_the_master_band_reports_its_address():
    _, slide, _, master_band, _, on_master = _banded()
    on_master.text_frame.paragraphs[0].runs[0].color = "#501030"
    entry = _entries(slide)[on_master.id]
    assert (entry.behind, entry.over) == (WINE, master_band.id)
    assert entry.over.startswith("M")


def test_a_layout_hiding_master_shapes_leaves_the_master_band_out():
    _, slide, _, _, on_layout, on_master = _banded(hide="layout")
    entries = _entries(slide)
    assert on_layout.id not in entries
    assert (entries[on_master.id].ratio, entries[on_master.id].over) == (1.0, "background")


def test_a_slide_hiding_background_graphics_leaves_both_bands_out():
    _, slide, _, _, on_layout, on_master = _banded(hide="slide")
    entries = _entries(slide)
    assert {entries[on_layout.id].over, entries[on_master.id].over} == {"background"}


def _filled_placeholders():
    deck = Document.new()
    layout = deck.layout("Title and Content")
    layout.placeholder("title").fill = NAVY
    deck.masters[0].placeholder("body").fill = WINE
    slide = deck.add_slide(layout)
    slide.title = "Title on the layout's navy"
    body = next(shape for shape in slide.shapes
                if shape.placeholder is not None and shape.placeholder.type != "title")
    body.set_text("Body on the master's wine")
    title = next(shape for shape in slide.shapes
                 if shape.placeholder is not None and shape.placeholder.type == "title")
    return deck, slide, title, body


def test_a_placeholders_effective_fill_comes_from_its_layout_then_its_master():
    deck, _, title, body = _filled_placeholders()
    assert title.fill is None and title.effective_fill.color.value == "003366"
    assert body.fill is None and body.effective_fill.color.value == "660033"
    # On the layout: the layout's own, and its body's from the master.
    layout = deck.layout("Title and Content")
    assert layout.placeholder("body").effective_fill.color.value == "660033"
    # The slide's own noFill hides the inherited one.
    body.fill = "none"
    assert body.effective_fill.kind == "none"


def test_contrast_measures_a_placeholders_text_against_its_inherited_fill():
    _, slide, title, body = _filled_placeholders()
    entries = _entries(slide)
    assert (entries[title.id].behind, entries[title.id].over) == (NAVY, "fill")
    assert (entries[body.id].behind, entries[body.id].over) == (WINE, "fill")
    body.fill = "none"
    assert body.id not in _entries(slide)


def test_an_inherited_picture_fill_names_the_layouts_picture():
    deck = Document.new()
    layout = deck.layout("Title Only")
    layout.placeholder("title").set_image_fill(ORANGE)
    slide = deck.add_slide(layout)
    title = next(shape for shape in slide.shapes if shape.placeholder is not None)
    fill = title.effective_fill
    assert fill.kind == "image" and fill.image_part.startswith("ppt/media/")


def test_a_slide_object_kept_across_a_new_slide_still_adds_shapes_with_text():
    deck = Document.new()
    first = deck.add_slide(deck.layouts[0])
    deck.add_slide(deck.layouts[0])
    shape = first.add_shape("rect", 0, 0, 100 * PT, 100 * PT, text="Still here")
    assert deck.shape(shape.id).text == "Still here"
    assert deck.slides[0] is first
    deck.undo()
    assert shape.id not in [s.id for s in first.shapes]
