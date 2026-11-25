"""The batch concurrency oracle, live (Phase 3 design §7.5): writers on threads, each a random mix
of lone edits, batches (atomic or not, some with a failing op), and reads of one document,
end in the state a serial replay of their changes in commit order gives -- the same check the
recorded concurrent cases (host_goldens.expected_of) hold the host to, on new seeds each run of
the suite's seeds below.  The Rust host's optimistic batches (D2) must keep it true."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import host_features  # noqa: E402
import host_goldens  # noqa: E402


@pytest.mark.parametrize("seed", [101, 102, 103])
def test_random_writers_with_batches_end_as_a_serial_replay_in_commit_order(seed: int, tmp_path) -> None:
    pytest.importorskip("docx_agent.tools")
    recorder = host_goldens.install(tmp_path / "inputs", lambda: f"live::{seed}")
    try:
        host_features.run_scenario(host_features.Scenario(
            "concurrency", f"live_{seed}", lambda r: host_features.random_writers(r, seed, writers=4, rounds=8)))
        recorder.settle()
    finally:
        recorder.uninstall()
    found, skipped = recorder.finished("live", lambda case: "live")
    assert skipped == []
    lines = found["live"]
    assert lines[0].get("concurrent"), "the writers never overlapped: no concurrency was tested"
    expected = host_goldens.expected_of(lines)
    assert any(line["event"] == "change" and line["tool"] == "batch" for line in expected)
    replayed = host_goldens.replay_case(lines, tmp_path)
    assert host_goldens.differences(expected, replayed) == []
