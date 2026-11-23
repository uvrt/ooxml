"""Every Word probe imports without Word (on any machine), so its documents can be built and
the recorded-Word tests can read them; a probe drives Word only when run as a script."""

from __future__ import annotations

import importlib
from pathlib import Path

import pytest
import recorded  # noqa: F401  (puts the probes on the path)

PROBES = sorted(p.stem for p in (Path(__file__).resolve().parent.parent / "probes").glob("*.py"))


@pytest.mark.parametrize("name", PROBES)
def test_a_probe_imports_without_word(name: str) -> None:
    importlib.import_module(name)
