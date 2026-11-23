"""The definitions docx-agent's toolbox sends against the Rust session's specs.

A Word tool's definition is the Rust session's spec (``ooxml_core.tool_specs("docx")``,
``docx_agent.rust.session.tool_of``).  A shared tool (``check``, ``render``,
``save_document``...) keeps ooxml-edit's shared definition, which another library's tools
merge with; it must stay the Rust spec byte for byte, so the two cannot drift apart.
"""

from __future__ import annotations

import json

import ooxml_core
import pytest

from ooxml_edit.tools import shared

from docx_agent.tools import TOOLS, WORD_TOOLS
from docx_agent.tools.shared import HANDLERS
from test_rust_tool_specs import export, text

SPECS = {spec["name"]: spec for spec in json.loads(ooxml_core.tool_specs("docx"))}
#: The export's keys compared apart: the aliases (functions here, their names in Rust) and
#: ``canonical`` (a shared tool's covers every kind's fields; the Rust spec's, docx's).
LOOSE = ("aliases", "canonical")


def _plain(data: dict) -> dict:
    return {key: value for key, value in data.items() if key not in LOOSE}


@pytest.mark.parametrize("name", sorted(HANDLERS.keys() & SPECS.keys()))
def test_a_shared_definition_is_the_rust_spec_byte_for_byte(name):
    today = export(shared.definition(name))
    assert text(_plain(today)) == text(_plain(SPECS[name]))
    assert sorted(today["aliases"]) == sorted(SPECS[name]["aliases"])


def test_every_rust_spec_is_a_tool_of_the_toolbox():
    names = [tool.name for tool in TOOLS]
    # undo and read_batch are the tool layer's own (ooxml-edit's session tools)
    assert set(SPECS) - {"undo", "read_batch"} <= set(names)
    assert len(names) == len(set(names))


def test_the_tools_out_until_ported_are_not_offered():
    names = {tool.name for tool in TOOLS}
    assert not names & {"edit_smartart", "style_by_value", "word_find_asset"}


@pytest.mark.parametrize("tool", WORD_TOOLS, ids=lambda tool: tool.name)
def test_a_word_tool_is_its_rust_spec(tool):
    assert text(_plain(export(tool))) == text(_plain(SPECS[tool.name]))
