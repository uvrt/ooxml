"""The tests that meet a gap the Rust session or its host still has where docx-agent's Python
did what the test expects (P4e, lane CUT-D's conversion): each is a strict xfail giving the
gap and the lane closing it (conftest's ``pytest_collection_modifyitems``), so it fails once
the gap is closed and its line here goes with the fix.  The tests keep their expectations.
Keyed by the test's file and name as pytest names them.

Must not: import pytest (a plain table).
"""

CUT_F = "lane CUT-F"
CUT6 = "lane CUT6"
TEMPLATE = "waits for lane CUT-B: template_blob"
GOLDEN = "test_tools_goldens.py::test_a_golden_transcript_replays_to_the_same_results_and_bytes"
GOLDEN_CORE = "test_tools_goldens.py::test_a_golden_transcript_replays_to_the_same_schema_and_core_anywhere"

#: The reason each test fails yet, by ``file::name[params]``.
GAPS: dict[str, str] = {
    "test_toc_numbers.py::test_the_entries_are_words[t-clear]":
        f"rust gap: a TOC entry lacks the right dot tab stop Word writes where the toc style clears its own "
        f"({CUT_F})",
    "test_tools_word.py::test_checks_report_the_reflow_once_the_document_was_laid_out":
        f"rust gap: an edit's checks after describe report no reflow (describe makes no page engine) ({CUT6})",
    "test_tools_word.py::test_a_long_document_reports_stale_reflow_beyond_twenty_pages":
        f"rust gap: an edit's checks after describe report no reflow (describe makes no page engine) ({CUT6})",
    "test_tools_word.py::test_a_check_lays_out_with_the_lock_let_go":
        f"rust gap: an edit made while a check lays out waits for the check ({CUT6})",
    "test_tools_word.py::test_new_document_from_a_template_blob_and_save_formats": TEMPLATE,
    f"{GOLDEN}[w1-template-report]": TEMPLATE,
    f"{GOLDEN_CORE}[w1-template-report]": TEMPLATE,
}
