"""The host goldens' recorder hook as a pytest plugin (``pytest -p host_recorder``, with this
folder on ``PYTHONPATH`` and ``HOST_GOLDENS_OUT`` naming the recording's folder): every session a
test makes is recorded (host_goldens.Recorder), each test file's into
``$HOST_GOLDENS_OUT/docx/tests/<file>@<worker>.jsonl``.  ``host_goldens.py record-tests`` runs
docx-agent's suite with it; ``host_goldens.py bless`` keeps what replays.

The hook is installed when the plugin is imported -- before the suite's conftest, so the faces
are pinned (host_goldens.pin_faces) for everything the suite lays out.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest

import host_goldens

OUT = Path(os.environ["HOST_GOLDENS_OUT"])
_current = {"test": "", "file": ""}
RECORDER = host_goldens.install(OUT / "inputs", lambda: _current["test"])
_FILE_OF: dict[str, str] = {}


@pytest.hookimpl(tryfirst=True)
def pytest_runtest_setup(item) -> None:
    _current["test"] = f"{Path(item.path).stem}::{item.name}"
    _FILE_OF[_current["test"]] = Path(item.path).stem


def pytest_sessionfinish(session) -> None:  # noqa: ARG001 -- pytest's hook
    RECORDER.uninstall()
    worker = os.environ.get("PYTEST_XDIST_WORKER", "main")
    source = host_goldens.SOURCE.format(sha=host_goldens.host_sha())

    def stream(case) -> str:
        return "docx/tests/" + _FILE_OF.get(case.test, case.test.split("::")[0] or "_import")

    found, skipped = RECORDER.finished(source, stream)
    for name, lines in found.items():
        host_goldens.write_stream(OUT / f"{name}@{worker}.jsonl", lines)
    host_goldens.write_skipped(OUT, f"docx/tests@{worker}", skipped)
