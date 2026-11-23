"""The tests that meet a form not supported yet whose port the user deferred until after the
release (p4e-plan.md, "DEFERRED"; lane CUT1): each is a strict xfail naming the form
(conftest's ``pytest_collection_modifyitems``), so it fails once the form is served and its
line here goes with the port.  Keyed by the test's file and name as pytest names them.

Must not: import pytest (a plain table).
"""

#: The deferred form each test meets first, by ``file::name[params]`` (or ``file::name``: each
#: of its parameters).
DEFERRED: dict[str, str] = {
    "test_dataset_text.py::test_deleting_around_a_commented_table_keeps_the_document_whole[row after the row]":
        "word_edit_table delete_row of a row holding a note or comment reference",
    "test_merged_rows_r59.py::test_a_frame_copy_merges_its_body_down_as_the_text_says":
        "word_insert_markdown like",
    "test_merged_rows_r59.py::test_a_frame_copy_with_merges_renders_like_the_sample":
        "word_insert_markdown like",
    "test_merged_rows_r59.py::test_a_frame_copy_keeps_the_refusals_with_the_way_out":
        "word_insert_markdown like",
    "test_table_regions_r58.py::test_a_markdown_table_takes_the_frame_rows_cells_and_text_formatting":
        "word_insert_markdown like",
    "test_table_regions_r58.py::test_a_frame_with_no_text_is_an_empty_markdown_table":
        "word_insert_markdown like",
    "test_table_regions_r58.py::test_like_takes_a_table_and_paragraphs_together":
        "word_insert_markdown like",
    "test_icons_minsizes.py::test_an_svg_picture_is_stored_as_word_stores_one":
        "word_drawings insert_picture of an SVG",
    "test_icons_minsizes.py::test_a_tint_is_a_theme_colour_with_its_class_or_a_hex_colour":
        "word_drawings insert_picture of an SVG",
    "test_icons_minsizes.py::test_what_cannot_be_tinted_or_read_is_refused":
        "word_drawings insert_picture of an SVG",
    "test_icons_minsizes.py::test_an_icon_in_a_cell_tinted_and_floating":
        "word_drawings insert_picture of an SVG",
    "test_icons_minsizes.py::test_writers_and_check_report_text_below_its_kinds_minimum":
        "check: min_text_size by kind",
    "test_retired_ids.py::test_a_deleted_drawing_s_number_is_not_given_to_the_next":
        "word_delete a picture (d:)",
    "test_retired_ids.py::test_a_copy_into_a_reopened_document_keeps_no_drawing_number_it_retired":
        "word_delete a picture (d:)",
    "test_id_state.py::test_a_deleted_drawing_number_is_not_given_again_after_the_reopen":
        "word_delete a picture (d:)",
    "test_accept_edges_r54a.py::test_after_our_accept_the_final_view_is_the_one_before_it[a first cell's end]":
        "word_read final view of Accept All's cell ends",
    "test_accept_edges_r54a.py::test_after_our_accept_the_final_view_is_the_one_before_it[a later row's first cell]":
        "word_read final view of Accept All's cell ends",
    "test_accept_edges_r54a.py::test_after_our_accept_the_final_view_is_the_one_before_it[a middle cell's end]":
        "word_read final view of Accept All's cell ends",
    "test_accept_edges_r54a.py::test_accepting_by_id_or_within_the_table_gives_what_accept_all_gives[a first cell's end]":
        "word_read final view of Accept All's cell ends",
    "test_accept_edges_r54a.py::test_accepting_by_id_or_within_the_table_gives_what_accept_all_gives[a later row's first cell]":
        "word_read final view of Accept All's cell ends",
    "test_accept_edges_r54a.py::test_accepting_by_id_or_within_the_table_gives_what_accept_all_gives[a middle cell's end]":
        "word_read final view of Accept All's cell ends",
    "test_accept_edges_r54a.py::test_accepting_by_id_or_within_the_table_gives_what_accept_all_gives[a row's end]":
        "word_read final view of Accept All's cell ends",
    "test_accept_edges_r54a.py::test_accepting_by_id_or_within_the_table_gives_what_accept_all_gives[the table's last cell]":
        "word_read final view of Accept All's cell ends",
    "test_rows_joins_r54r.py::test_the_joins_read_as_accept_all_leaves_them":
        "word_read final view of Accept All's cell ends",
    "test_tracked_tables_r54.py::test_a_deleted_cell_whose_content_is_kept_widens_the_cell_before_in_the_final_view":
        "word_read final view of Accept All's cell ends",
    "test_tracked_reshape_r55.py::test_the_final_view_reads_the_merge_over_a_deleted_column_as_accepted":
        "word_read final view of Accept All's cell ends",
    "test_retired_ids.py::test_a_replaced_block_names_the_one_that_replaced_it":
        "insert_markdown at replace:",
    "test_id_state.py::test_a_replaced_block_still_names_its_replacement":
        "insert_markdown at replace:",
    "test_replace_keeps.py::test_body_paragraphs_keep_their_style_keeps_page_break_and_indent":
        "insert_markdown at replace:",
    "test_replace_keeps.py::test_list_items_join_the_replaced_list_at_their_levels":
        "insert_markdown at replace:",
    "test_replace_keeps.py::test_a_flat_list_replacing_nested_items_stays_at_their_level":
        "insert_markdown at replace:",
    "test_replace_keeps.py::test_markdown_wins_on_kind_level_start_and_named_styles":
        "insert_markdown at replace:",
    "test_replace_keeps.py::test_headings_keep_a_same_level_heading_style":
        "insert_markdown at replace:",
    "test_replace_keeps.py::test_more_new_paragraphs_take_the_last_donor_without_a_second_page_break":
        "insert_markdown at replace:",
    "test_replace_keeps.py::test_tracked_replacement_keeps_settings_and_stays_valid":
        "insert_markdown at replace:",
    "test_replace_keeps.py::test_the_replacement_lays_out_as_the_text_rewritten_in_place":
        "insert_markdown at replace:",
    "test_side_by_side_edits.py::test_a_dataset_table_put_alone_after_edits_side_by_side":
        "word_insert_markdown from data (from, blob)",
    "test_tools_word.py::test_insert_markdown_with_every_style_role_writes_no_direct_formatting[normal0]":
        "a footnote in Markdown",
    "test_tools_word.py::test_insert_markdown_with_every_style_role_writes_no_direct_formatting[normal1]":
        "a footnote in Markdown",
    "test_typography.py::test_every_writing_tool_spaces_what_it_writes":
        "a footnote in Markdown",
    "test_tools_word.py::test_insert_markdown_from_a_blob_with_an_image_blob":
        "a picture in Markdown",
    "test_provenance.py::test_a_chart_from_a_source_is_keyed_by_its_drawing_id":
        "edit_chart add from",
    "test_tables_from_data.py::test_a_chart_from_a_date_column_and_set_values_matched_either_way[None-en-US]":
        "edit_chart add from",
    "test_tables_from_data.py::test_a_chart_from_a_date_column_and_set_values_matched_either_way[locale1-nl-BE]":
        "edit_chart add from",
    "test_tables_from_data.py::test_a_charts_dates_are_kept_in_the_locales_code":
        "edit_chart add from",
    "test_addresses.py::test_a_reversed_span_is_refused_alike_everywhere":
        "insert_markdown at replace:",
    "test_places.py::test_existing_callers_are_unchanged":
        "insert_markdown at replace:",
    "test_replace_keeps.py::test_the_tool_says_what_was_kept":
        "insert_markdown at replace:",
    "test_tools_word.py::test_find_and_replace_one_or_all_body_or_all_stories":
        "replace_text in the body only while other stories match",
    "test_addresses.py::test_find_text_and_replace_text_take_a_cell_a_section_a_bookmark_and_an_open_span":
        "replace_text range",
    "test_language.py::test_french_spacing_in_english_text_is_a_language_mismatch_never_a_deletion":
        "replace_text range",
    "test_numbers.py::test_the_fix_writes_by_reference_and_the_number_is_left_out_after":
        "replace_text range",
    "test_settle_removed_r56.py::test_an_edit_inside_an_earlier_insertion_keeps_it_the_earlier_ones":
        "replace_text range",
    "test_tools_word.py::test_replace_text_within_a_span_of_paragraphs_and_a_table":
        "replace_text range",
    "test_typography.py::test_problems_list_what_is_left_with_fixes_that_clear_them":
        "replace_text range",
    "test_served_records.py::test_served_reviews_follow_the_records_as_docx_agent_does[accept moving the only record]":
        "word_changes accept moving the document's only table record (its item is made again)",
    "test_served_records.py::test_the_session_reads_the_records_as_docx_agent_does[reviewed_alone]":
        "word_changes accept moving the document's only table record (its item is made again)",
    "test_outline.py::test_the_tools_take_a_heading_range":
        "word_delete a heading without its end",
    "test_tools_word.py::test_picture_from_a_blob_floating_and_drawings":
        "word_drawings ref",
    "test_original_loss_r57.py::test_an_old_describing_comment_is_kept_and_its_refusal_names_the_way_out[delete_column-2]":
        "word_edit_table delete_column of a column holding a note or comment reference",
    "test_original_loss_r57.py::test_an_old_describing_comment_is_kept_and_its_refusal_names_the_way_out[delete_row-0]":
        "word_edit_table delete_row of a row holding a note or comment reference",
    "test_accept_edges_r54a.py::test_check_and_the_table_warning_say_what_our_accept_does":
        "word_edit_table on a table with Accept All's cell ends",
    "test_rows_joins_r54r.py::test_a_from_block_fills_the_rows_as_accepted":
        "word_edit_table on a table with Accept All's cell ends",
    "test_rows_joins_r54r.py::test_a_merge_over_a_pending_deleted_row_is_refused_with_the_accept":
        "word_edit_table on a table with Accept All's cell ends",
    "test_rows_joins_r54r.py::test_a_row_past_the_end_is_refused_with_the_accepted_shape":
        "word_edit_table on a table with Accept All's cell ends",
    "test_rows_joins_r54r.py::test_an_edit_of_the_table_says_what_accept_all_loses":
        "word_edit_table on a table with Accept All's cell ends",
    "test_rows_joins_r54r.py::test_an_insert_beside_a_pending_deleted_row_goes_beside_the_row_named[after the row before]":
        "word_edit_table on a table with Accept All's cell ends",
    "test_rows_joins_r54r.py::test_an_insert_beside_a_pending_deleted_row_goes_beside_the_row_named[before the row after]":
        "word_edit_table on a table with Accept All's cell ends",
    "test_rows_joins_r54r.py::test_an_insert_beside_a_pending_deleted_row_goes_beside_the_row_named[default last]":
        "word_edit_table on a table with Accept All's cell ends",
    "test_rows_joins_r54r.py::test_delete_row_and_format_row_count_the_rows_as_accepted":
        "word_edit_table on a table with Accept All's cell ends",
    "test_rows_joins_r54r.py::test_positions_follow_the_cells_accept_all_takes_in_and_removes":
        "word_edit_table on a table with Accept All's cell ends",
    "test_rows_joins_r54r.py::test_row_label_and_a_column_fill_count_the_rows_as_accepted":
        "word_edit_table on a table with Accept All's cell ends",
    "test_rows_joins_r54r.py::test_row_positions_skip_a_row_whose_deletion_is_pending":
        "word_edit_table on a table with Accept All's cell ends",
    "test_rows_joins_r54r.py::test_sync_and_describe_count_the_rows_as_accepted":
        "word_edit_table on a table with Accept All's cell ends",
    "test_span_rows_r57.py::test_a_new_row_beside_a_column_deleted_pending_is_deleted_there_too":
        "word_edit_table on a table with Accept All's cell ends",
    "test_table_fill_speed.py::test_cells_by_position_write_what_they_wrote_one_by_one[True]":
        "word_edit_table on a table with Accept All's cell ends",
    "test_table_sync.py::test_tracked_when_tracking_is_on":
        "word_edit_table on a table with Accept All's cell ends",
    "test_tracked_reshape_r55.py::test_a_reshape_over_pending_changes_accepts_as_made_untracked[split beside a deleted row]":
        "word_edit_table on a table with Accept All's cell ends",
    "test_tracked_reshape_r55.py::test_the_order_of_the_edits_does_not_matter[merged down over a deleted row]":
        "word_edit_table on a table with Accept All's cell ends",
    "test_dataset_markdown.py::test_a_table_cell_takes_the_value_itself":
        "word_insert_markdown from data (from, blob)",
    "test_dataset_text.py::test_rows_are_still_a_table":
        "word_insert_markdown from data (from, blob)",
    "test_numbers.py::test_a_table_placed_from_a_source_and_a_range_narrow_what_is_read":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_a_copied_table_gets_no_record":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_a_deleted_table_drops_its_entry_in_the_same_undo_step":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_a_table_from_a_source_is_kept_in_one_custom_xml_item":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_a_tracked_table_from_a_source_is_kept_and_rejecting_it_drops_it":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_another_custom_xml_item_is_left_as_it_is":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_columns_shift_and_a_deleted_one_drops_its_entry":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_deleting_a_row_inside_the_block_drops_the_record":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_merge_and_split_drop_the_record":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_rejecting_a_change_in_a_recorded_table_drops_its_record":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_rows_above_shift_the_block_below_keep_it_inside_drop_it":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_set_cells_without_from_drops_only_the_columns_it_overwrites":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_the_document_validates_renders_and_the_definitions_are_unchanged":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_the_table_key_is_its_first_rows_para_id_and_every_paragraph_is_stamped":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_tracked_the_record_describes_the_document_with_changes_accepted":
        "word_insert_markdown from data (from, blob)",
    "test_provenance.py::test_undo_takes_the_item_away_with_the_table":
        "word_insert_markdown from data (from, blob)",
    "test_served_fields.py::test_rows_from_a_dataset_stay_the_hosts":
        "word_insert_markdown from data (from, blob)",
    "test_sync_from_r56.py::test_other_froms_of_the_document_say_the_same[word_insert_markdown-arguments0]":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_dataset_named_like_nothing_drawn_gets_no_drawing_hint":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_date_columns_iso_text_is_written_in_the_sessions_locale[None-due0]":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_date_columns_iso_text_is_written_in_the_sessions_locale[locale1-due1]":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_date_format_is_checked_for_a_table":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_drawings_id_as_from_is_named_a_drawing_not_a_dataset":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_header_given_or_none":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_new_table_from_a_source_is_one_undo_step_and_tracks":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_row_range_of_a_source_is_a_table_whose_header_repeats":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_source_too_big_is_refused_never_cut":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_table_from_a_dataset_header_rows_and_provenance":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_table_from_another_documents_table_merges_its_sources":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_tables_dates_in_a_date_format[en-GB-due0]":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_tables_dates_in_a_date_format[es-ES-due2]":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_tables_dates_in_a_date_format[it-IT-due3]":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_tables_dates_in_a_date_format[nl-NL-due1]":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_value_a_cell_cannot_take_is_refused_naming_it":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_a_values_lines_are_line_breaks_in_one_paragraph":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_an_unlabelled_iso_column_is_written_as_it_is":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_column_names_given_as_columns_are_read_as_from_columns":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_columns_are_picked_and_ordered_by_name_and_default_to_all":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_from_with_markdown_or_without_a_source_is_refused":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_numbers_are_written_in_the_sessions_locale":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_text_goes_in_literally_not_as_markdown":
        "word_insert_markdown from data (from, blob)",
    "test_tables_from_data.py::test_too_many_rows_from_a_source_without_a_row_range_is_refused_not_internal":
        "word_insert_markdown from data (from, blob)",
    "test_like.py::test_a_copied_sample_extended_follows_the_copy":
        "word_insert_markdown like",
    "test_like.py::test_a_level_below_the_given_ones_is_the_list_a_level_deeper":
        "word_insert_markdown like",
    "test_like.py::test_a_sample_in_no_list_and_another_kind_are_said":
        "word_insert_markdown like",
    "test_like.py::test_a_style_map_naming_a_paragraph_is_read_as_like":
        "word_insert_markdown like",
    "test_like.py::test_an_empty_level_is_written_without_like":
        "word_insert_markdown like",
    "test_like.py::test_like_wins_over_the_replaced_items_kept_settings":
        "word_insert_markdown like",
    "test_like.py::test_new_nested_items_follow_the_sample_at_each_level":
        "word_insert_markdown like",
    "test_like.py::test_numbered_items_count_on_from_the_sample":
        "word_insert_markdown like",
    "test_like.py::test_replacing_the_sample_itself_keeps_its_look":
        "word_insert_markdown like",
    "test_table_regions_r58.py::test_a_frame_copy_is_tracked_as_an_insertion_and_undone_in_one_step":
        "word_insert_markdown like",
    "test_table_regions_r58.py::test_frames_refuse_what_they_cannot_copy_with_the_way_out":
        "word_insert_markdown like",
    "test_rust_reads.py::test_the_rust_reads_equal_todays_on_the_fixtures[generated/tables-accepted.docx]":
        "word_inspect of Accept All's cell ends",
    "test_accept_edges_r54a.py::test_the_result_says_where_a_rows_last_cell_text_went_and_undo_brings_it_back":
        "word_read final view of Accept All's cell ends",
    "test_rust_reads_on.py::test_the_reads_answer_as_the_python_path_does[generated/tables-accepted.docx]":
        "word_read final view of Accept All's cell ends",
    "test_rust_word_read.py::test_word_read_in_rust_equals_todays_on_the_fixtures[generated/tables-accepted.docx]":
        "word_read final view of Accept All's cell ends",
    "test_tracked_merge_r54m.py::test_a_block_from_a_source_fills_the_columns_as_accepted":
        "word_read final view of Accept All's cell ends",
    "test_tracked_merge_r54m.py::test_a_column_fill_counts_the_columns_as_accepted":
        "word_read final view of Accept All's cell ends",
    "test_tracked_merge_r54m.py::test_a_column_inserted_beside_a_pending_deletion_lands_where_accept_all_has_it[0-after-| Area | New | Phase | Remark |]":
        "word_read final view of Accept All's cell ends",
    "test_tracked_merge_r54m.py::test_a_column_inserted_beside_a_pending_deletion_lands_where_accept_all_has_it[1-before-| Area | New | Phase | Remark |]":
        "word_read final view of Accept All's cell ends",
    "test_tracked_merge_r54m.py::test_a_column_inserted_beside_a_pending_deletion_lands_where_accept_all_has_it[None-None-| Area | Phase | Remark | New |]":
        "word_read final view of Accept All's cell ends",
    "test_tracked_merge_r54m.py::test_a_position_before_the_deleted_column_says_nothing":
        "word_read final view of Accept All's cell ends",
    "test_tracked_merge_r54m.py::test_col_label_finds_the_columns_as_accepted":
        "word_read final view of Accept All's cell ends",
    "test_tracked_merge_r54m.py::test_delete_column_index_counts_the_columns_as_accepted":
        "word_read final view of Accept All's cell ends",
    "test_tracked_merge_r54m.py::test_set_cells_col_skips_a_column_whose_deletion_is_pending":
        "word_read final view of Accept All's cell ends",
    "test_tracked_merge_r54m.py::test_sync_counts_the_columns_as_accepted":
        "word_read final view of Accept All's cell ends",
    "test_tracked_tables_r54.py::test_a_tracked_column_deletion_reads_as_accepted":
        "word_read final view of Accept All's cell ends",
    "test_text_facts.py::test_a_slide_reference_is_refused_and_an_escaped_one_written":
        "word_set_text by reference",
    "test_text_facts.py::test_an_escape_writes_the_reference_and_a_bad_one_changes_nothing":
        "word_set_text by reference",
    "test_owned_sources.py::test_the_records_of_an_owned_document_are_the_session_s":
        "word_insert_markdown from data (from, blob)",
    "test_owned_sources.py::test_store_provenance_on_an_owned_document_is_an_internal_refusal":
        "word_insert_markdown from data (from, blob)",
    "test_owned_sources.py::test_store_provenance_is_never_handed_the_owner_s_document":
        "word_insert_markdown from data (from, blob)",
}
