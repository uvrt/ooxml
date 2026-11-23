"""The tests that meet a gap the Rust session or its host still has where docx-agent's Python
did what the test expects (P4e, lane CUT-D's conversion): each is a strict xfail giving the
gap and the lane closing it (conftest's ``pytest_collection_modifyitems``), so it fails once
the gap is closed and its line here goes with the fix.  The tests keep their expectations.
Keyed by the test's file and name as pytest names them.

Must not: import pytest (a plain table).
"""

CUT6 = "lane CUT6"
CUT_B = "lane CUT-B"
TEMPLATE = "waits for lane CUT-B: template_blob"
GOLDEN = "test_tools_goldens.py::test_a_golden_transcript_replays_to_the_same_results_and_bytes"
GOLDEN_CORE = "test_tools_goldens.py::test_a_golden_transcript_replays_to_the_same_schema_and_core_anywhere"

#: The reason each test fails yet, by ``file::name[params]``.
GAPS: dict[str, str] = {
    "test_tools_word.py::test_a_check_lays_out_with_the_lock_let_go":
        f"rust gap: an edit made while a check lays out waits for the check ({CUT6})",
    "test_session_checks.py::test_a_check_between_a_batch_s_changes_holds_the_document_and_says_so":
        f"rust gap: a check between a batch's changes gives no warning that it held the document ({CUT6})",
    "test_source_refs.py::test_a_documents_own_braces_and_escaped_ones_stay_text":
        f"rust gap: word_set_text of braces naming no source is refused as by reference ({CUT6})",
    "test_charts.py::test_chart_edit_renders[charts-d:5-category]": f"rust gap: a number given as a numeric chart's category (0.25) is refused as not a number ({CUT_B})",
    "test_charts.py::test_chart_edit[charts-d:5-category]": f"rust gap: a number given as a numeric chart's category (0.25) is refused as not a number ({CUT_B})",
    "test_charts.py::test_chart_edit[charts-d:5-add_category]": f"rust gap: a number given as a numeric chart's category (0.25) is refused as not a number ({CUT_B})",
    "test_style_lists.py::test_continue_drops_direct_numbering_where_the_style_numbers_the_items": f"rust gap: word_lists continue keeps the direct numbering of an item its style numbers ({CUT_F})",
    "test_follow_sample.py::test_report_actionable_leaves_the_copies_own_out_counted":
        f"rust gap: word_follow_sample's report \"\" is refused, not read as the default ({CUT_F})",
    "test_copy_settings.py::test_copies_that_look_the_same_do_not_differ":
        f"rust gap: word_lists add bullet level 1 on an item its style numbers stays at level 0 ({CUT_F})",
    "test_block_of.py::test_a_revision_and_an_endnote_give_their_block": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_block_of.py::test_a_top_level_block_is_its_own": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_block_of.py::test_an_unknown_id_is_not_found_and_a_span_is_refused": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_block_of.py::test_annotations_give_the_block_that_holds_them": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_block_of.py::test_blocks_between_refuses_a_reversed_span_as_the_span_tools_do": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_block_of.py::test_blocks_between_spans_top_level_blocks_from_nested_ends": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_block_of.py::test_nested_ids_give_the_top_level_block": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_block_of.py::test_what_is_not_in_the_body_is_none": f"rust gap: block_of/blocks_between of nested, story and annotation ids ({CUT_B})",
    "test_batch_cost.py::test_a_mixed_batch_gives_the_results_each_op_gives_and_lays_out_once":
        f"rust gap: a mixed batch lays the document out for several of its ops, not once ({CUT6})",
    "test_range_extent.py::test_what_is_not_drawn_has_no_extent":
        f"rust gap: a paragraph hidden whole (w:vanish) is given an extent; Word draws nothing ({CUT_F})",
    "test_retired_ids.py::test_a_positional_table_id_in_a_cell_address_resolves_after_the_ids_are_stamped":
        f"rust gap: a positional table id inside a cell address is refused once the ids are stamped ({CUT_F})",
    "test_many_writers.py::test_continue_with_several_targets_continues_each":
        f"rust gap: word_lists continue with several targets continues only the first ({CUT_F})",
    "test_tools_word.py::test_new_document_from_a_template_blob_and_save_formats": TEMPLATE,
    "test_locale.py::test_from_a_template_the_locale_is_kept_and_the_settings_are_the_templates": TEMPLATE,
    f"{GOLDEN}[w1-template-report]": TEMPLATE,
    f"{GOLDEN_CORE}[w1-template-report]": TEMPLATE,
}
