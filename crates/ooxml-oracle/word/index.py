"""Every observation of Word's (``observations/<name>.json``): the probe that measured it, the
tests that hold the tools to it, and for each of its cases no test reads, why.

A reader in ``word/tests`` (the recorded-Word tier) declares ``READS``, the observation's
top-level cases it compares with; ``tests/test_index.py`` checks that those and ``unread`` are
together every case of the observation (its notes -- ``_about``, ``source``, ``word`` -- aside).
Readers elsewhere (docx-agent's tests, the Rust crates' integration tests) are listed by path:
they name the observation they read, and their cases are theirs to cover.

Paths are from the repository's root.  ``LAYOUT`` registers docx2svg's former measurements
(``observations/layout``), which wait for the Rust layout's port.  Must not: import anything.
"""

from __future__ import annotations

#: The keys of an observation that are its notes, not Word's answers.
NOTES = frozenset({"_about", "source", "word"})

OBSERVATIONS: dict[str, dict] = {
    "charts-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/charts_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_charts_recorded.py"],
        "unread": {
            "titles": "how Word draws each title form (face, size, colour off its PDF): the drawing's, "
                      "held by the renderers, not an edit's output",
            "smartart": "Word's redraw of diagrams whose drawing and data disagree; no tool edits a "
                        "diagram since edit_smartart was withdrawn",
            "dictionary": "Word's AppleScript dictionary: what the probe could ask Word, not an answer",
        },
    },
    "e1-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/e1_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_e1_recorded.py"],
        "unread": {
            "style_ids_localised": "the Dutch interface's ids for the built-in styles; the tools write English "
                                   "Word's (the readers translate Word's with them)",
        },
    },
    "e2-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/e2_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_e2_recorded.py"],
        "unread": {},
    },
    "e3-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/e3_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_e3_recorded.py"],
        "unread": {
            "cells-pdf": "Word's PDF text of the recorded cell revisions: what Word draws, the renderers' "
                         "concern",
        },
    },
    "e4-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/e4_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_e4_recorded.py"],
        "unread": {
            "notes2": "a section's note numbering (format, start) and the endnotes' place: no tool sets them",
        },
    },
    "e5-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/e5_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_e5_recorded.py",
                    "crates/ooxml-oracle/word/tests/test_e5_tracked_recorded.py"],
        "unread": {},
    },
    "e6-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/e6_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_e6_recorded.py"],
        "unread": {
            "template-dotx": "Word's File > New from a template keeps its body; new_document takes a template's "
                             "styles, theme and layouts, not its content, by design",
            "template-docx": "as template-dotx, the template written as a .docx",
            "resave-11": "Word's plain save of a mode 11 document: the baseline Convert is told apart from",
            "resave-all-11": "as resave-11, every legacy option set",
            "resave-all-14": "as resave-11, mode 14",
            "resave-14-sample": "as resave-11, a corpus document",
        },
    },
    "followups-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/followups_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_followups_recorded.py"],
        "unread": {},
    },
    "grid-values-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/grid_values_probe.py",
        "readers": ["crates/ooxml-oracle/tests/integration/grid_values.rs"],
    },
    "language-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/language_probe.py",
        "readers": ["python/docx-agent/tests/test_language.py"],
    },
    "merged-rows-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/merged_rows_probe.py",
        "readers": ["python/docx-agent/tests/test_merged_rows_r59.py"],
    },
    "paraid-durability.json": {
        "probe": "crates/ooxml-oracle/word/probes/paraid_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_paraid_recorded.py"],
        "unread": {
            "mixed": "docx-agent's retired paragraph stamping beside other tools' ids; the tools stamp one way "
                     "(decided, factors)",
            "word-created": "a document Word made, with no paraIds: its stamping is decided's 'Word-created' case",
        },
    },
    "replace-text-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/replace_text_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_replace_text_recorded.py"],
        "unread": {},
    },
    "row-format-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/row_format_probe.py",
        "readers": ["python/docx-agent/tests/test_rows_r58.py"],
    },
    "rows-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/rows_probe.py",
        "readers": ["python/docx-agent/tests/test_own_rows.py",
                    "crates/ooxml-tools-word/tests/integration/tracked_rows.rs"],
    },
    "separator-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/separator_probe.py",
        "readers": ["python/docx-agent/tests/test_own_rows.py"],
    },
    "styleref-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/styleref_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_styleref_recorded.py"],
        "unread": {},
    },
    "table-regions-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/table_regions_probe.py",
        "readers": ["python/docx-agent/tests/test_table_regions_r58.py"],
    },
    "theme-tints-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/theme_tint_probe.py",
        "readers": ["python/docx-agent/tests/test_declared_styles.py"],
    },
    "toc-numbers-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/toc_numbers_probe.py",
        "readers": ["python/docx-agent/tests/test_toc_numbers.py",
                    "crates/ooxml-tools-word/tests/integration/structure.rs"],
    },
    "toc-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/toc_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_toc_recorded.py"],
        "unread": {},
    },
    "suite-word.json": {
        "probe": "crates/ooxml-oracle/word/suite/test_word_suite.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_suite_recorded.py"],
        "unread": {},
    },
    "tracking-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/tracking_probe.py",
        "readers": ["python/docx-agent/tests/test_own_insertions.py"],
    },
    "trial-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/trial_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_trial_recorded.py"],
        "unread": {
            "document-as-docm": "no tool writes a macro-enabled document",
            "document-saved-docm": "no tool writes a macro-enabled document",
            "word-saves-docm": "what Word's own Save As .docm writes: no tool writes one",
        },
    },
    "typography-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/typography_probe.py",
        "readers": ["python/docx-agent/tests/test_typography.py"],
    },
}


#: The measurements docx2svg's Python layout was held to (its tests/fixtures, moved here untouched in
#: lane svg-thin when that layout was withdrawn): each file's family, the Word that measured it
#: (``None``: not stated), whether it is Word's measurement (``False``: a face's numbers read from
#: its font file, which the scoring took as input), and the probes that made it -- scripts in
#: PROBES at the commit SOURCE.  No test reads them until the Rust layout's port scores LaidPages
#: against them, a family at a time (their readers then take OBSERVATIONS' form).
SOURCE = "6e850e181891c62393daf28dbe804d6673005304"
PROBES = "python/docx2svg/tools"
LAYOUT: dict[str, dict] = {
    "layout/anchor-observations.json": {"family": "anchors", "word": "Word 16.106", "measured": True,
        "probes": ["make_anchor_probe.py", "make_drawing_probe.py", "make_story_anchor_probe.py",
                    "make_wrap_anchor_probe.py", "make_wrap_stack_probe.py", "read_anchor_probe.py"]},
    "layout/autofit-over-observations.json": {"family": "tables", "word": "Word 16.106", "measured": True,
        "probes": ["make_autofit_over_probe.py", "read_autofit_over_probe.py", "read_float_table_probe.py"]},
    "layout/autofit-width-observations.json": {"family": "tables", "word": "Word 16.106", "measured": True,
        "probes": ["make_autofit_width_probe.py", "read_autofit_width_probe.py", "read_float_table_probe.py"]},
    "layout/autospacing-observations.json": {"family": "autospacing", "word": "Word 16.106", "measured": True,
        "probes": ["make_autospacing_probe.py", "read_autospacing_probe.py"]},
    "layout/baseline-observations.json": {"family": "baselines", "word": "Word 16.106", "measured": True,
        "probes": ["read_baselines.py", "make_style_document.py", "make_layout_sweep.py"]},
    "layout/border-group-observations.json": {"family": "paragraph-borders", "word": "Word 16.106", "measured": True,
        "probes": ["make_border_group_probe.py", "read_border_group_probe.py"]},
    "layout/break-advances.json": {"family": "line-breaking", "word": None, "measured": False,
        "probes": ["read_breaks.py", "face_advances.py"]},
    "layout/break-end-observations.json": {"family": "break-end", "word": "Word 16.106", "measured": True,
        "probes": ["make_break_end_probe.py", "read_break_end_probe.py"]},
    "layout/break-rules-observations.json": {"family": "line-breaking", "word": "Word 16.106", "measured": True,
        "probes": ["make_break_rules_probe.py", "read_break_rules_probe.py"]},
    "layout/cell-footnote-observations.json": {"family": "footnotes", "word": "Word 16.106", "measured": True,
        "probes": ["make_cell_footnote_probe.py", "read_cell_footnote_probe.py"]},
    "layout/chart-observations.json": {"family": "charts", "word": None, "measured": True,
        "probes": ["make_chart_probe.py", "read_chart_probe.py"]},
    "layout/chart-text-observations.json": {"family": "charts", "word": None, "measured": True,
        "probes": ["make_chart_text_probe.py", "read_chart_text_probe.py"]},
    "layout/cloud-font-observations.json": {"family": "cloud-fonts", "word": "Word 16.106", "measured": True,
        "probes": ["make_cloud_font_probe.py", "read_cloud_font_probe.py"]},
    "layout/columns-observations.json": {"family": "columns", "word": "Word 16.106", "measured": True,
        "probes": ["make_columns_probe.py", "read_columns_probe.py"]},
    "layout/dml-observations.json": {"family": "dml", "word": "Word 16.106", "measured": True,
        "probes": ["make_dml_probe.py", "read_dml_probe.py"]},
    "layout/drop-cap-observations.json": {"family": "drop-cap", "word": "Word 16.106", "measured": True,
        "probes": ["make_drop_cap_probe.py", "read_drop_cap_probe.py", "read_float_table_probe.py"]},
    "layout/endnote-observations.json": {"family": "endnotes", "word": "Word 16.106", "measured": True,
        "probes": ["make_endnote_probe.py", "read_endnote_probe.py"]},
    "layout/float-table-observations.json": {"family": "tables-float", "word": "Word 16.106", "measured": True,
        "probes": ["make_float_table_probe.py", "read_float_table_probe.py"]},
    "layout/footnote-draw-observations.json": {"family": "footnotes", "word": "Word 16.106", "measured": True,
        "probes": ["make_footnote_draw_probe.py", "read_footnote_draw_probe.py"]},
    "layout/footnote-observations.json": {"family": "footnotes", "word": "Word 16.106", "measured": True,
        "probes": ["make_footnote_probe.py", "read_footnote_probe.py", "pages.py"]},
    "layout/grid-mismatch-observations.json": {"family": "tables", "word": "Word 16.106", "measured": True,
        "probes": ["make_grid_mismatch_probe.py", "read_grid_mismatch_probe.py"]},
    "layout/header-observations.json": {"family": "headers", "word": "Word 16.106", "measured": True,
        "probes": ["make_header_probe.py", "read_header_probe.py", "pages.py"]},
    "layout/hide-mark-observations.json": {"family": "hide-mark", "word": "Word 16.106", "measured": True,
        "probes": ["make_hide_mark_probe.py", "read_hide_mark_probe.py"]},
    "layout/hyphen-faces.json": {"family": "hyphenation", "word": None, "measured": False,
        "probes": ["read_hyphen_probe.py"]},
    "layout/hyphen-observations.json": {"family": "hyphenation", "word": "Word 16.106", "measured": True,
        "probes": ["make_hyphen_probe.py", "read_hyphen_probe.py"]},
    "layout/justify-faces.json": {"family": "justify", "word": None, "measured": False,
        "probes": ["read_justify_probe.py"]},
    "layout/justify-observations.json": {"family": "justify", "word": "Word 16.106", "measured": True,
        "probes": ["make_justify_probe.py", "read_justify_probe.py"]},
    "layout/keep-observations.json": {"family": "keeps", "word": "Word 16.106", "measured": True,
        "probes": ["make_keep_probe.py", "read_keep_probe.py", "pages.py"]},
    "layout/label-observations.json": {"family": "labels", "word": "Word 16.106", "measured": True,
        "probes": ["make_label_probe.py", "read_label_probe.py"]},
    "layout/line-advance-observations.json": {"family": "line-height", "word": "Word 16.106", "measured": True,
        "probes": ["make_line_advance_probe.py", "read_line_advance_probe.py"]},
    "layout/line-box-observations.json": {"family": "line-height", "word": "Word 16.106", "measured": True,
        "probes": ["make_line_box_probe.py", "read_line_box_probe.py"]},
    "layout/line-number-observations.json": {"family": "line-numbers", "word": "Word 16.106", "measured": True,
        "probes": ["make_line_number_probe.py", "read_line_number_probe.py"]},
    "layout/list-level-observations.json": {"family": "numbering", "word": "Word 16.106", "measured": True,
        "probes": ["make_list_level_probe.py", "read_list_level_probe.py"]},
    "layout/margin-observations.json": {"family": "margins", "word": "Word 16.106", "measured": True,
        "probes": ["make_margin_probe.py", "read_margin_probe.py"]},
    "layout/mark-observations.json": {"family": "paragraph-mark", "word": "Word 16.106", "measured": True,
        "probes": ["make_mark_probe.py", "read_mark_probe.py"]},
    "layout/mixed-line-observations.json": {"family": "line-height", "word": "Word 16.106", "measured": True,
        "probes": ["make_mixed_line_probe.py", "read_mixed_line_probe.py"]},
    "layout/narrow-text-observations.json": {"family": "narrow-text", "word": "Word 16.106", "measured": True,
        "probes": ["make_narrow_text_probe.py", "read_narrow_text_probe.py"]},
    "layout/nested-table-observations.json": {"family": "tables", "word": "Word 16.106", "measured": True,
        "probes": ["make_nested_table_probe.py", "read_nested_table_probe.py", "read_float_table_probe.py"]},
    "layout/note-after-observations.json": {"family": "footnotes", "word": None, "measured": True,
        "probes": ["make_note_after_probe.py", "read_note_after_probe.py"]},
    "layout/numbering-observations.json": {"family": "numbering", "word": "Word 16.106", "measured": True,
        "probes": ["make_numbering_probe.py", "read_numbering_probe.py"]},
    "layout/page-advances.json": {"family": "pagination", "word": None, "measured": False,
        "probes": ["read_pages.py"]},
    "layout/page-fit-observations.json": {"family": "pagination", "word": "Word 16.106", "measured": True,
        "probes": ["make_page_fit_probe.py", "read_page_fit_probe.py", "pages.py"]},
    "layout/page-top-collapse-observations.json": {"family": "page-top", "word": "Word 16.106", "measured": True,
        "probes": ["make_page_top_collapse_probe.py", "read_page_top_collapse_probe.py"]},
    "layout/page-top-observations.json": {"family": "page-top", "word": "Word 16.106", "measured": True,
        "probes": ["make_page_top_probe.py", "read_page_top_probe.py"]},
    "layout/pagination-observations.json": {"family": "pagination", "word": "Word 16.106", "measured": True,
        "probes": ["make_pagination_probe.py", "read_pagination_probe.py", "pages.py"]},
    "layout/para-border-observations.json": {"family": "paragraph-borders", "word": "Word 16.106", "measured": True,
        "probes": ["make_para_border_probe.py", "read_para_border_probe.py"]},
    "layout/pct-table-observations.json": {"family": "tables", "word": "Word 16.106", "measured": True,
        "probes": ["make_pct_table_probe.py", "read_pct_table_probe.py", "read_float_table_probe.py"]},
    "layout/picture-bullet-observations.json": {"family": "picture-bullet", "word": "Word 16.106", "measured": True,
        "probes": ["make_picture_bullet_probe.py", "read_picture_bullet_probe.py"]},
    "layout/picture-observations.json": {"family": "pictures", "word": "Word 16.106", "measured": True,
        "probes": ["make_picture_probe.py", "read_picture_probe.py", "pages.py"]},
    "layout/picture-place-observations.json": {"family": "pictures", "word": "Word 16.106", "measured": True,
        "probes": ["make_picture_place_probe.py", "make_picture_probe.py", "read_picture_place_probe.py"]},
    "layout/picture-run-observations.json": {"family": "pictures", "word": "Word 16.106", "measured": True,
        "probes": ["make_picture_run_probe.py", "read_picture_run_probe.py"]},
    "layout/probe-advances.json": {"family": "line-breaking",
        "word": "faces Word 16.106 lays out with on macOS (font files)", "measured": False,
        "probes": ["probe_advances.py"]},
    "layout/render-observations.json": {"family": "render", "word": "Word 16.106", "measured": True,
        "probes": ["make_render_probe.py", "read_render.py", "make_layout_sweep.py", "make_style_document.py"]},
    "layout/revision-observations.json": {"family": "revisions", "word": "Word 16.106", "measured": True,
        "probes": ["make_revision_probe.py", "read_revision_probe.py"]},
    "layout/script-observations.json": {"family": "script", "word": "Word 16.106", "measured": True,
        "probes": ["make_script_probe.py", "read_script_probe.py"]},
    "layout/script-offset-observations.json": {"family": "script", "word": "Word 16.106", "measured": True,
        "probes": ["make_script_offset_probe.py", "read_script_offset_probe.py"]},
    "layout/section-end-observations.json": {"family": "sections", "word": "Word 16.106", "measured": True,
        "probes": ["make_section_end_probe.py", "read_section_end_probe.py"]},
    "layout/section-observations.json": {"family": "sections", "word": "Word 16.106", "measured": True,
        "probes": ["make_section_probe.py", "read_section_probe.py", "pages.py"]},
    "layout/smartart-observations.json": {"family": "smartart", "word": None, "measured": True,
        "probes": ["make_smartart_probe.py", "read_smartart_probe.py"]},
    "layout/story-observations.json": {"family": "stories", "word": "Word 16.106", "measured": True,
        "probes": ["make_story_probe.py", "make_story_select_probe.py", "make_field_probe.py",
                    "read_story_probe.py"]},
    "layout/style-observations.json": {"family": "style-cascade", "word": "Word 16.106", "measured": True,
        "probes": ["make_style_probe.py", "read_style_probe.py"]},
    "layout/styleref-observations.json": {"family": "fields",
        "word": "Word for Mac 16 (English UI, -AppleLocale en_US), PDF export after updating every field",
        "measured": True,
        "probes": ["make_styleref_probe.py"]},
    "layout/tab-leader-observations.json": {"family": "tab-leaders", "word": "Word 16.106", "measured": True,
        "probes": ["make_tab_leader_probe.py", "read_tab_leader_probe.py"]},
    "layout/table-foot-observations.json": {"family": "tables-foot", "word": "Word 16.106", "measured": True,
        "probes": ["make_table_foot_probe.py", "read_table_foot_probe.py"]},
    "layout/table-keep-observations.json": {"family": "tables-keep", "word": "Word 16.106", "measured": True,
        "probes": ["make_table_keep_probe.py", "read_table_keep_probe.py", "pages.py"]},
    "layout/table-line-observations.json": {"family": "table-lines", "word": "Word 16.106", "measured": True,
        "probes": ["make_table_line_probe.py", "read_table_line_probe.py"]},
    "layout/table-observations.json": {"family": "tables", "word": "Word 16.106", "measured": True,
        "probes": ["make_table_geometry_probe.py", "make_table_content_probe.py", "make_table_style_size_probe.py",
                    "make_table_row_probe.py", "make_table_merge_probe.py", "make_table_border_probe.py",
                    "make_table_pages_probe.py", "read_table_probes.py"]},
    "layout/text-box-observations.json": {"family": "text-boxes", "word": "Word 16.106", "measured": True,
        "probes": ["make_text_box_probe.py", "read_text_box_probe.py"]},
    "layout/text-box-top-observations.json": {"family": "text-boxes", "word": "Word 16.106", "measured": True,
        "probes": ["make_text_box_top_probe.py", "read_text_box_top_probe.py"]},
    "layout/wrap-budget-observations.json": {"family": "line-breaking", "word": "Word 16.106", "measured": True,
        "probes": ["make_wrap_budget_probe.py", "read_wrap_budget_probe.py"]},
    "layout/wrap-kern-observations.json": {"family": "line-breaking", "word": "Word 16.106", "measured": True,
        "probes": ["make_wrap_kern_probe.py", "read_wrap_kern_probe.py"]},
    "layout/wrap-observations.json": {"family": "line-breaking", "word": "Word 16.106", "measured": True,
        "probes": ["make_wrap_probe.py", "read_wrap_probe.py"]},
    "layout/wrap-side-observations.json": {"family": "wrap-side", "word": "Word 16.106", "measured": True,
        "probes": ["make_wrap_side_probe.py", "read_wrap_side_probe.py"]},
    "layout/wrap-table-observations.json": {"family": "wrap-table", "word": "Word 16.106", "measured": True,
        "probes": ["make_wrap_table_probe.py", "make_cell_anchor_probe.py", "make_cell_valign_probe.py",
                    "read_wrap_table_probe.py"]},
    "layout/wrap-tables-observations.json": {"family": "wrap-tables", "word": "Word 16.106", "measured": True,
        "probes": ["make_wrap_tables_probe.py", "read_wrap_tables_probe.py"]},
    "layout/xref-field-observations.json": {"family": "fields", "word": "Word 16.106", "measured": True,
        "probes": ["make_xref_field_probe.py", "read_xref_field_probe.py"]},
}
