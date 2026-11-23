"""The Word-oracle suite over the tools' edits (opt-in, a Mac with Word: ``pytest -m word``,
serial).  Every case (``edit_sets.py``) is run through the tool host docx-agent serves; Word is
asked about each output through the oracle driver (``word/oracle.py``: the machine-wide lock,
the content-hash cache) and its answers are written to ``word/observations/suite-word.json``,
which the recorded tier (``word/tests/test_suite_recorded.py``) holds the tools to anywhere."""

from __future__ import annotations

import os
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
for _path in (HERE, HERE.parent, HERE.parent / "probes"):
    if str(_path) not in sys.path:
        sys.path.insert(0, str(_path))

import probe_paths  # noqa: E402,F401  (docx-agent's test helpers on the path)


def pytest_configure(config):
    config.addinivalue_line("markers", "word: drives Microsoft Word (opt-in: pytest -m word, serial)")


def pytest_collection_modifyitems(config, items):
    """Word is one instance per machine: the suite runs only when asked for, never under
    xdist's workers."""
    asked = "word" in (config.option.markexpr or "")
    reason = ("the Word oracle runs serially: pytest -m word, without -n" if os.environ.get("PYTEST_XDIST_WORKER")
              else None if asked else "opt-in: pytest -m word")
    if reason:
        for item in items:
            if item.get_closest_marker("word") is not None:
                item.add_marker(pytest.mark.skip(reason=reason))


@pytest.fixture(scope="session")
def box():
    from docx_agent.tools import FORMAT, GROUPS, TOOLS
    from ooxml_edit.tools import Toolbox

    with Toolbox(list(TOOLS), formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox
