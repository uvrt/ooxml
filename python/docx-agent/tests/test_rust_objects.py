"""The object tools' Rust definitions (lane P4c: ``crates/ooxml-tools-word``, golden
``tests/golden``) equal today's byte for byte: ``word_edit_table``, ``word_format_table``,
``word_drawings`` (as the toolbox without an asset catalogue defines it),
``word_controls`` and ``edit_chart``.  An alias that is a function today is written as what
it reads: a placing tool's place as ``place:<kind>``
(:func:`docx_agent.tools.positions.aliases`), a list of names given to a parameter that
takes none as ``names:<parameter>`` (:func:`ooxml_edit.tools.registry.names_as`); an alias
read only while an argument has a value (:class:`ooxml_edit.tools.AliasWhen`, ``edit_chart``'s
``slide``: ``target`` when ``action`` is ``add``) is the entry both sides write,
``{"name", "when": {argument: value}}``.  The handlers' equality with today's is the
edit oracle's (``crates/ooxml-oracle/python/word_edits_diff.py --objects``); nothing is
switched on here.
"""
from __future__ import annotations

import inspect
from pathlib import Path

import pytest
from ooxml_edit.tools import AliasWhen

from docx_agent.tools import TOOLS

HERE = Path(__file__).resolve().parent
GOLDEN = HERE.parents[2] / "crates" / "ooxml-tools-word" / "tests" / "golden"
NAMES = ["word_edit_table", "word_format_table", "word_drawings", "word_controls", "edit_chart"]


def _alias(value, tool):
    if isinstance(value, str):
        return value
    if isinstance(value, AliasWhen):
        return value.exported()
    free = inspect.getclosurevars(value).nonlocals
    if "kind" in free:
        return "place:" + free["kind"]
    if "name" in free:
        return "names:" + free["name"]
    raise AssertionError(f"{tool.name}: an alias function that reads nothing")


@pytest.mark.parametrize("name", NAMES)
def test_the_rust_definitions_are_todays_byte_for_byte(name):
    from test_rust_tool_specs import export, text

    tool = {tool.name: tool for tool in TOOLS}[name]
    today = export(tool)
    today["aliases"] = {key: _alias(value, tool) for key, value in today["aliases"].items()}
    assert (GOLDEN / f"{name}.json").read_text(encoding="utf-8") == text(today)
