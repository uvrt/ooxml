"""The oracle driver's cache bound (``word/oracle.py``'s ``evict``), without Word: the answers
least recently used go first, a folder in the cache is kept."""

from __future__ import annotations

import os
from pathlib import Path

import oracle


def _answer(folder: Path, name: str, size: int, used: int) -> Path:
    path = folder / name
    path.write_bytes(b"x" * size)
    os.utime(path, (used, used))
    return path


def test_the_least_recently_used_go_first_past_the_file_bound(tmp_path):
    old, middle, new = (_answer(tmp_path, f"{n}.pdf", 10, used) for n, used in (("a", 100), ("b", 200), ("c", 300)))
    deleted = oracle.evict(tmp_path, files=2, size=10 ** 6)
    assert deleted == [old]
    assert not old.exists() and middle.exists() and new.exists()


def test_the_least_recently_used_go_first_past_the_size_bound(tmp_path):
    old = _answer(tmp_path, "a-saved.docx", 40, 100)
    middle = _answer(tmp_path, "b.pdf", 40, 200)
    new = _answer(tmp_path, "c.pdf", 40, 300)
    assert oracle.evict(tmp_path, files=10, size=50) == [old, middle]
    assert new.exists()


def test_a_hit_is_a_use(tmp_path):
    first = _answer(tmp_path, "a.pdf", 10, 100)
    second = _answer(tmp_path, "b.pdf", 10, 200)
    oracle._used(first)
    assert oracle.evict(tmp_path, files=1, size=10 ** 6) == [second]


def test_within_bounds_nothing_goes_and_folders_stay(tmp_path):
    (tmp_path / "measured").mkdir()
    (tmp_path / "measured" / "table.tsv").write_text("kept")
    _answer(tmp_path, "a.pdf", 10, 100)
    assert oracle.evict(tmp_path, files=1, size=10) == []
    assert oracle.evict(tmp_path, files=0, size=0) == [tmp_path / "a.pdf"]
    assert (tmp_path / "measured" / "table.tsv").exists()
