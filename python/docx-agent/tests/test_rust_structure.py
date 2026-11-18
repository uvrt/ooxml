"""The structure tools' Rust definitions (lane P4b: ``crates/ooxml-tools-word``, golden
``tests/golden``) equal today's byte for byte: ``word_move``, ``word_copy_from``,
``word_sections``, ``word_headers_footers``, ``word_fields``, ``word_notes`` and
``word_links``.  A placing tool's aliases are functions today
(:func:`docx_agent.tools.positions.aliases`); the Rust export writes each as the kind of
place it reads (``place:<kind>``).  The handlers' equality with today's is the edit
oracle's (``crates/ooxml-oracle/python/word_edits_diff.py --structure``); nothing is
switched on here.
"""
from __future__ import annotations

import inspect
from pathlib import Path

import pytest

from docx_agent.tools import TOOLS

HERE = Path(__file__).resolve().parent
GOLDEN = HERE.parents[2] / "crates" / "ooxml-tools-word" / "tests" / "golden"
NAMES = ["word_move", "word_copy_from", "word_sections", "word_headers_footers", "word_fields", "word_notes",
         "word_links"]


def _alias(value):
    if isinstance(value, str):
        return value
    return "place:" + inspect.getclosurevars(value).nonlocals["kind"]


@pytest.mark.parametrize("name", NAMES)
def test_the_rust_definitions_are_todays_byte_for_byte(name):
    from test_rust_tool_specs import export, text

    today = export({tool.name: tool for tool in TOOLS}[name])
    today["aliases"] = {key: _alias(value) for key, value in today["aliases"].items()}
    assert (GOLDEN / f"{name}.json").read_text(encoding="utf-8") == text(today)
