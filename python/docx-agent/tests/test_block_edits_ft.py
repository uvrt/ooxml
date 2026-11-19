"""Many writers on one document where the interpreter runs without its lock (a free-threaded
build; lane L): ``block_stress.run`` with 8 to 24 threads -- plain, ``more`` (redo, fields
updates, lists, neighbouring blocks) and ``more`` with every edit tracked -- each the same,
call by call and byte by byte, as the calls run one at a time in the order their changes were
committed.  Skipped with the interpreter lock: there the threads' Python runs one step at a
time and ``test_block_edits_serial.py`` covers the protocol.  The runs are ordered by the
commits they record, never by clocks."""

from __future__ import annotations

import sys

import pytest

import block_stress

# Calls prepared on drafts: documents the tool layer holds itself (conftest.python_owned).
pytestmark = [
    pytest.mark.any_faces,
    pytest.mark.usefixtures("python_owned"),
    pytest.mark.skipif(getattr(sys, "_is_gil_enabled", lambda: True)(),
                       reason="many writers side by side: only without the interpreter lock"),
]


@pytest.mark.parametrize("seed, threads, more, tracked", [
    (101, 8, False, False),
    (102, 16, True, False),
    (103, 24, True, True),
    (104, 16, True, True),
])
def test_many_writers_on_one_document_equal_their_commit_order(seed, threads, more, tracked):
    found = block_stress.run(seed, threads=threads, calls=6, chapters=4, first_fill=bool(seed % 2),
                             tracked=tracked, more=more)
    assert found["same"], found
    assert found["drafted"] > 0
