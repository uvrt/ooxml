"""The tests that meet a gap the Rust session still has where docx-agent's Python matched
Word (P4e, lane CUT-D's conversion; fixed by lane CUT-F): each is a strict xfail naming the
gap (conftest's ``pytest_collection_modifyitems``), so it fails once the gap is closed and
its line here goes with the fix.  The tests keep Word's expectation.  Keyed by the test's
file and name as pytest names them.

Must not: import pytest (a plain table).
"""

#: The gap each test meets, by ``file::name[params]``.
GAPS: dict[str, str] = {
    "test_toc_numbers.py::test_the_entries_are_words[t-clear]":
        "a TOC entry lacks the right dot tab stop Word writes where the toc style clears its own",
}
