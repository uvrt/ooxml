"""Which error code a tool call's failure gets (review, S9).

ooxml-edit 0.66.0 types its refusals (``ooxml_edit.errors``): its dispatcher gives an
``ArgumentError`` ``invalid_arguments`` and a ``UnitError`` (a value out of its range)
``unit``, in every format.  Since the cut (P4e) a Word document's calls are the Rust
session's, whose refusals come with their codes: the Word format maps no exception of its own.
"""

from __future__ import annotations

import pytest
from ooxml_edit.errors import ArgumentError, UnitError
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


@pytest.mark.parametrize("error, code", [
    (ArgumentError("a diagram keeps at least one node"), "invalid_arguments"),
    (UnitError("text is longer than a cell holds"), "unit"),
    # Python's own ValueError is still unit; its TypeError internal.
    (ValueError("invalid literal"), "unit"),
    (TypeError("unsupported operand"), "internal"),
])
def test_the_codes(box, error, code):
    assert box.map_exception(error).code == code
