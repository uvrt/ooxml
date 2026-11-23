"""Where the Word probes find what they use, and where what they measure goes.

The probes (``python crates/ooxml-oracle/word/probes/<name>_probe.py``, on a Mac with Word,
through the oracle driver's machine-wide lock) write Word's observations into
:data:`OBSERVATIONS`; the recorded-Word tests (``crates/ooxml-oracle/word/tests``) hold the
tools to them on any machine.  Importing this module puts on ``sys.path``: the probes' folder
(they share helpers), the probe documents (:data:`CASES`), docx-agent's fixture makers
(``python/docx-agent/tools``: ``make_fixtures``) and the oracle driver
(``crates/ooxml-oracle/word/oracle.py``).

Must not: import pytest or docx-agent's library.
"""

from __future__ import annotations

import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
#: The repository's root (``crates/ooxml-oracle/word/probes`` is four folders down).
ROOT = HERE.parents[3]
ORACLE = ROOT / "crates" / "ooxml-oracle"
#: Word's recorded answers, one JSON file per probe.
OBSERVATIONS = ORACLE / "word" / "observations"
#: The probes' made-up documents, shared with the tests that read the observations.
CASES = ORACLE / "word" / "cases"
#: The committed Word fixtures (the gates' documents).
FIXTURES = ORACLE / "fixtures"
DOCX_AGENT = ROOT / "python" / "docx-agent"

for _path in (CASES, HERE, ORACLE / "word"):
    if str(_path) not in sys.path:
        sys.path.insert(0, str(_path))
for _path in (DOCX_AGENT / "tools", DOCX_AGENT / "tests"):
    if str(_path) not in sys.path:
        sys.path.append(str(_path))


def measured(name: str) -> Path:
    """Where a probe writes a table it measured (``name``): the oracle's own folder, not the
    repository.  The committed tables built from these measurements (e.g.
    ``crates/ooxml-word-edit/data/builtin-styles.tsv``, ``crates/ooxml-tools-word/data/word_new.py``)
    are generated from the frozen reference's copy of them; a new measurement is compared with
    that by hand before any table changes."""
    import oracle

    folder = oracle.ORACLE_DIR / "measured"
    folder.mkdir(parents=True, exist_ok=True)
    return folder / name
