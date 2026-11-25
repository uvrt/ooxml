"""What the tools read from the host is listed in ``served`` (the Rust session's ``Hosting`` went
with phase 3's format-generic tools, lane ph3-format): the sets are the ones the registry
exported, and an adapter or listed tool the registry does not export is refused."""

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
          # p2-serve-i (#167): the review and object tools, word_copy_from
          "word_edit_table", "word_changes", "word_comments", "word_controls", "word_set_tracking",
          "word_copy_from", "set_properties",
          # lane RD2: render, a reading tool the host hands over with its page engine
          "render",
          # p4-chart-e to j: edit_chart's data, title and style actions
          "edit_chart"}
BLOBS = {"word_drawings"}
# (render's page engine is handed to it by reads.serve_render, not read from the export)
LAID = {"word_fields", "word_headers_footers", "word_template", "word_format_table"}
KEEPS_WRITTEN = {"word_set_text", "word_insert_text", "replace_text", "word_insert_markdown",
                 # lane E1d: set_cells' values by reference
                 "word_edit_table"}


def test_the_exported_sets_are_the_lists_they_replace():
    hosting = served.hosting()
    assert SERVED <= hosting.tools
    assert hosting.blobs == BLOBS
    assert hosting.laid == LAID
    assert hosting.keeps_written == KEEPS_WRITTEN


def test_the_tools_are_the_registry_export():
    names = {s["name"] for s in json.loads(ooxml_core.tool_specs("docx"))} - {"read_batch", "undo"}
    assert served.hosting().tools == names
    assert not hasattr(ooxml_core, "tool_hosting")


def test_only_served_tools_have_adapters():
    assert set(served.ADAPTERS) <= SERVED
    # word_styles and word_template take their arguments as given: no adapter.
    assert {"word_styles", "word_template"}.isdisjoint(served.ADAPTERS)
    exported = served.hosting().tools - {"word_set_text"}
    with pytest.raises(ValueError, match="word_set_text"):
        served.Hosting.of(exported)


def test_an_adapter_registers_once():
    with pytest.raises(ValueError, match="two adapters"):
        served.adapter("word_set_text")(lambda call, arguments: arguments)
