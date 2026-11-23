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
    "test_own_rows.py::test_the_probes_edits_made_here_give_words_result":
        f"rust gap: deleting one's own pending row holding another's insertion keeps the row ({CUT_F})",
    "test_own_rows.py::test_a_table_one_inserted_beside_a_table_goes_with_the_paragraph_it_came_with":
        f"rust gap: deleting a table one inserted leaves its tracked separator paragraph ({CUT_F})",
    f"{GOLDEN}[w1-template-report]": TEMPLATE,
    f"{GOLDEN_CORE}[w1-template-report]": TEMPLATE,
}
