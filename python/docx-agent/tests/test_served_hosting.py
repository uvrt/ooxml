"""The tools the Rust session serves come from the registry's export (``ooxml_core.tool_hosting``,
each Rust tool's ``Hosting``), not a list in docx-agent (lane G8): the sets are the ones
docx-agent listed by hand before, and an adapter of a tool not served is refused."""

from __future__ import annotations

import json

import pytest

ooxml_core = pytest.importorskip("ooxml_core")

from docx_agent.rust import served  # noqa: E402

# docx-agent's lists before lane G8 (rust/edits.py SERVED, BLOBS, LAID, KEEPS_WRITTEN, as of main
# with lane P2d3 PR f: word_insert_markdown keeps written values).
SERVED = {"word_set_text", "replace_text", "word_insert_text", "word_delete", "word_format", "word_lists",
          "word_format_table", "word_styles", "word_follow_sample", "word_insert_markdown", "word_fields",
          "word_headers_footers", "word_template", "word_move", "word_sections", "word_notes", "word_links",
          "word_drawings",
          # p4-chart-e to g: edit_chart's data actions and the titles
          "edit_chart"}
BLOBS = {"word_drawings"}
LAID = {"word_fields", "word_headers_footers", "word_template"}
KEEPS_WRITTEN = {"word_set_text", "word_insert_text", "replace_text", "word_insert_markdown"}


def test_the_exported_sets_are_the_lists_they_replace():
    hosting = served.hosting()
    assert hosting.served == SERVED
    assert hosting.blobs == BLOBS
    assert hosting.laid == LAID
    assert hosting.keeps_written == KEEPS_WRITTEN


def test_every_tool_is_exported_with_its_hosting():
    exported = json.loads(ooxml_core.tool_hosting("docx"))
    names = {s["name"] for s in json.loads(ooxml_core.tool_specs("docx"))} - {"read_batch"}
    assert set(exported) == names
    assert all(set(facts) == {"served", "blobs", "layout", "keeps_written"} for facts in exported.values())
    assert json.loads(ooxml_core.tool_hosting("pptx")) == {}


def test_only_served_tools_have_adapters():
    assert set(served.ADAPTERS) <= SERVED
    # word_styles and word_template take their arguments as given: no adapter.
    assert {"word_styles", "word_template"}.isdisjoint(served.ADAPTERS)
    exported = json.loads(ooxml_core.tool_hosting("docx"))
    exported["word_set_text"]["served"] = False
    with pytest.raises(ValueError, match="word_set_text"):
        served.Hosting.of(exported)


def test_an_adapter_registers_once():
    with pytest.raises(ValueError, match="two adapters"):
        served.adapter("word_set_text")(lambda call, arguments: arguments)
