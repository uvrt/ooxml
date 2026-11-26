"""The host goldens' mixed streams (crates/ooxml-oracle/host/mixed): a workbook xlsx-agent holds
beside a Word document -- datasets, sources and facts across them -- replayed through today's
Python host with no unlisted difference.  The oracle's own tests replay the docx streams; these
need xlsx-agent, which only this suite installs (crates/ooxml-oracle/python/host_goldens.py)."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

pytest.importorskip("ooxml_core")
pytest.importorskip("docx_agent.tools")

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "crates" / "ooxml-oracle" / "python"))

import host_goldens  # noqa: E402

STREAMS = {name: path for name, path in host_goldens.streams().items() if name.startswith("mixed/")}
CASES = [(name, case, events) for name, path in STREAMS.items()
         for case, events in host_goldens.cases_of(host_goldens.read_stream(path)).items()]


def test_there_are_mixed_streams() -> None:
    assert CASES


@pytest.mark.parametrize("stream,case,events", CASES, ids=[f"{n}::{c}" for n, c, _e in CASES])
def test_a_mixed_case_replays_with_no_unlisted_difference(stream: str, case: str, events: list[dict]) -> None:
    found = host_goldens.differences(host_goldens.expected_of(events), host_goldens.replay_case(events))
    left, _used = host_goldens.unlisted(stream, case, found, host_goldens.known_entries())
    assert left == [], left[:3]
