"""Charts as the tools edit them, held to what Word kept and drew (``probes/charts_probe.py``,
``observations/charts-word.json``, Word 16.106 for Mac): the tools read a chart's caches, which
Word draws; Word's save keeps every edit the tools made; it records no revision for a chart's
data, and the tools apply such an edit untracked; the places and ids of charts in groups and
text boxes are Word's."""

from __future__ import annotations

import io
import json
import zipfile

from recorded import observed  # noqa: I001  (puts the probes on the path)

import charts_probe  # noqa: E402

OBSERVED = observed("charts-word.json")
#: The observation's keys these tests compare with (index.py lists why the others are not).
READS = {"cache", "extensions", "places", "tracking"}


#: What both reads of a chart say: the recorded ones are docx-agent's Python reads, the tools'
#: read has its own shape around the same facts.
FACTS = ("title", "categories", "series")


def _data(charts: list[dict]) -> list[dict]:
    return [{fact: chart["data"].get(fact) for fact in FACTS} for chart in charts]


def test_the_tools_read_the_cache_word_draws():
    """``cache``: caches and the embedded workbook disagree; Word draws the caches and its save
    keeps them -- what the tools read."""
    word = OBSERVED["cache"]
    assert word["pdf_shows_cache_categories"] and not word["pdf_shows_workbook_categories"]
    assert _data(charts_probe.charts_of(charts_probe.cache_document())) == _data(word["saved_charts"])


def test_words_save_keeps_every_edit_the_tools_made():
    """``extensions``: every chart of charts.docx edited through the tools; Word's save of the
    result reads back as the tools left it."""
    data, _ = charts_probe.edited_charts()
    assert _data(charts_probe.charts_of(data)) == _data(OBSERVED["extensions"]["saved"])
    assert all(OBSERVED["extensions"]["pdf_has"][label] for label in ("Added", "Renamed", "New series"))


def test_a_charts_data_edit_is_applied_untracked_as_word_does():
    """``tracking``: Word records no revision for a chart's data (revisions=0 with tracking on);
    the tools' edit, tracking on, leaves no markup either."""
    assert OBSERVED["tracking"]["charts"]["word_says"] == "revisions=0 tracking=true"
    data, _ = charts_probe.edited_charts(tracking=True)
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        story = archive.read("word/document.xml")
    assert b"<w:ins " not in story and b"<w:del " not in story


def test_charts_in_groups_and_text_boxes_keep_words_places_and_ids():
    """``places``: the tools' edit of chart-places.docx; Word's save keeps every chart where it was,
    drawing ids and group members included."""
    ours = json.loads(json.dumps(charts_probe._ids(charts_probe.edited_places())))
    assert ours == OBSERVED["places"]["saved"] == OBSERVED["places"]["ours"]
