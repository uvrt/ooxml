"""The tests that meet a gap the Rust session or its host still has where docx-agent's Python
did what the test expects (P4e, lane CUT-D's conversion): each is a strict xfail giving the
gap and the lane closing it (conftest's ``pytest_collection_modifyitems``), so it fails once
the gap is closed and its line here goes with the fix.  The tests keep their expectations.
Keyed by the test's file and name as pytest names them.

Must not: import pytest (a plain table).
"""

#: The reason each test fails yet, by ``file::name[params]``.
GAPS: dict[str, str] = {}
