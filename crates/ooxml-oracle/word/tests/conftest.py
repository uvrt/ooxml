"""The recorded-Word tier (runs anywhere, no Word): each test makes, through the tools, the edit a
probe made in Word (``crates/ooxml-oracle/word/probes``) on the probe's own document, and holds
what the tools wrote to what Word wrote (``crates/ooxml-oracle/word/observations``).
``crates/ooxml-oracle/word/index.py`` names, for every observation, the tests that read it or why
none does."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))
#: The oracle driver (``word/oracle.py``), for its own tests.
sys.path.append(str(Path(__file__).resolve().parent.parent))


@pytest.fixture(scope="session")
def box():
    """The tool host docx-agent serves (the tools as a caller gets them), no workers."""
    from docx_agent.tools import FORMAT, GROUPS, TOOLS
    from ooxml_edit.tools import Toolbox

    with Toolbox(list(TOOLS), formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox
