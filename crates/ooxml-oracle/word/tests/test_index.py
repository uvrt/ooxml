"""``word/index.py`` is whole and true: every observation has its probe and a reader, every
reader exists and names its observation, and a recorded-Word reader's cases (``READS``) and the
index's ``unread`` are together every case of the observation, its notes aside."""

from __future__ import annotations

import ast
import json
import sys
from pathlib import Path

import pytest

WORD = Path(__file__).resolve().parent.parent
ROOT = WORD.parents[2]
sys.path.insert(0, str(WORD))

import index  # noqa: E402

OBSERVATIONS = WORD / "observations"


def test_every_observation_is_in_the_index_and_nothing_else():
    assert sorted(p.name for p in OBSERVATIONS.glob("*.json")) == sorted(index.OBSERVATIONS)


def test_every_probe_in_the_folder_is_an_observations():
    probes = {p.name for p in (WORD / "probes").glob("*_probe.py")}
    assert probes == {Path(entry["probe"]).name for entry in index.OBSERVATIONS.values()}


def _reads(path: Path, cases: set[str]) -> set[str]:
    """A tier reader's ``READS``: a literal set, or ``set(OBSERVED)`` (every case)."""
    for node in ast.parse(path.read_text(encoding="utf-8")).body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "READS" for t in node.targets):
            if isinstance(node.value, ast.Call) and getattr(node.value.func, "id", None) == "set":
                return set(cases)
            return set(ast.literal_eval(node.value))
    raise AssertionError(f"{path.name} declares no READS")


@pytest.mark.parametrize("name", sorted(index.OBSERVATIONS))
def test_each_observation_has_its_probe_and_readers(name):
    entry = index.OBSERVATIONS[name]
    assert (ROOT / entry["probe"]).is_file(), entry["probe"]
    assert entry["readers"], name
    for reader in entry["readers"]:
        path = ROOT / reader
        assert path.is_file(), reader
        assert name in path.read_text(encoding="utf-8"), (reader, "does not name", name)


@pytest.mark.parametrize("name", sorted(n for n, e in index.OBSERVATIONS.items() if "unread" in e))
def test_a_tier_observations_cases_are_read_or_said_why_not(name):
    entry = index.OBSERVATIONS[name]
    observed = json.loads((OBSERVATIONS / name).read_text(encoding="utf-8"))
    cases = set(observed) - index.NOTES
    read = set().union(*(_reads(ROOT / reader, cases) for reader in entry["readers"]))
    unread = set(entry["unread"])
    assert not read & unread, read & unread
    assert read | unread == cases, (cases - read - unread, (read | unread) - cases)
    assert all(reason.strip() for reason in entry["unread"].values())
