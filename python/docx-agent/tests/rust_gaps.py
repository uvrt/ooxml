"""The tests that meet a gap the Rust session or its host still has where docx-agent's Python
did what the test expects (P4e, lane CUT-D's conversion): each is a strict xfail giving the
gap and the lane closing it (conftest's ``pytest_collection_modifyitems``), so it fails once
the gap is closed and its line here goes with the fix.  The tests keep their expectations.
Keyed by the test's file and name as pytest names them.

Must not: import pytest (a plain table).
"""

CUT_F = "lane CUT-F"
CUT6 = "lane CUT6"
CUT_B = "lane CUT-B"
TEMPLATE = "waits for lane CUT-B: template_blob"
GOLDEN = "test_tools_goldens.py::test_a_golden_transcript_replays_to_the_same_results_and_bytes"
GOLDEN_CORE = "test_tools_goldens.py::test_a_golden_transcript_replays_to_the_same_schema_and_core_anywhere"

#: The reason each test fails yet, by ``file::name[params]``.
GAPS: dict[str, str] = {
    "test_charts.py::test_chart_edit_renders[charts-d:5-category]": f"rust gap: a number given as a numeric chart's category (0.25) is refused as not a number ({CUT_B})",
    "test_charts.py::test_chart_edit[charts-d:5-category]": f"rust gap: a number given as a numeric chart's category (0.25) is refused as not a number ({CUT_B})",
    "test_charts.py::test_chart_edit[charts-d:5-add_category]": f"rust gap: a number given as a numeric chart's category (0.25) is refused as not a number ({CUT_B})",
    "test_follow_sample.py::test_report_actionable_leaves_the_copies_own_out_counted":
        f"rust gap: word_follow_sample's report \"\" is refused, not read as the default ({CUT_F})",
    "test_block_of.py::test_a_revision_and_an_endnote_give_their_block": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_block_of.py::test_a_top_level_block_is_its_own": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_block_of.py::test_an_unknown_id_is_not_found_and_a_span_is_refused": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_block_of.py::test_annotations_give_the_block_that_holds_them": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_block_of.py::test_blocks_between_spans_top_level_blocks_from_nested_ends": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_block_of.py::test_nested_ids_give_the_top_level_block": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_batch_cost.py::test_a_mixed_batch_gives_the_results_each_op_gives_and_lays_out_once":
        f"rust gap: a mixed batch lays the document out for several of its ops, not once ({CUT6})",
    "test_tools_word.py::test_new_document_from_a_template_blob_and_save_formats": TEMPLATE,
    "test_locale.py::test_from_a_template_the_locale_is_kept_and_the_settings_are_the_templates": TEMPLATE,
    f"{GOLDEN}[w1-template-report]": TEMPLATE,
    f"{GOLDEN_CORE}[w1-template-report]": TEMPLATE,
}
