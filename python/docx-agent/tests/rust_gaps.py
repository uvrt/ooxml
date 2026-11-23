"""The tests that meet a gap the Rust session or its host still has where docx-agent's Python
did what the test expects (P4e, lane CUT-D's conversion): each is a strict xfail giving the
gap and the lane closing it (conftest's ``pytest_collection_modifyitems``), so it fails once
the gap is closed and its line here goes with the fix.  The tests keep their expectations.
Keyed by the test's file and name as pytest names them.

Must not: import pytest (a plain table).
"""

TEMPLATE = "w1 was recorded on the frozen reference; its template document reads otherwise from word_read on (lane CUT6)"
GOLDEN = "test_tools_goldens.py::test_a_golden_transcript_replays_to_the_same_results_and_bytes"
GOLDEN_CORE = "test_tools_goldens.py::test_a_golden_transcript_replays_to_the_same_schema_and_core_anywhere"

#: The reason each test fails yet, by ``file::name[params]``.
GAPS: dict[str, str] = {
    f"{GOLDEN}[w1-template-report]": TEMPLATE,
    f"{GOLDEN_CORE}[w1-template-report]": TEMPLATE,
}
