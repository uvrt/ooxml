"""The style tools' Rust definitions (lane P4d: ``crates/ooxml-tools-word``, golden
``tests/golden``) equal today's byte for byte: ``word_lists`` so far.  The handlers'
equality with today's is the edit oracle's (``crates/ooxml-oracle/python/word_edits_diff.py
--style``); nothing is switched on here.
"""
from __future__ import annotations

from pathlib import Path

import pytest

from docx_agent.tools import TOOLS

HERE = Path(__file__).resolve().parent
GOLDEN = HERE.parents[2] / "crates" / "ooxml-tools-word" / "tests" / "golden"
NAMES = ["word_lists"]


@pytest.mark.parametrize("name", NAMES)
def test_the_rust_definitions_are_todays_byte_for_byte(name):
    from test_rust_tool_specs import export, text

    today = export({tool.name: tool for tool in TOOLS}[name])
    assert (GOLDEN / f"{name}.json").read_text(encoding="utf-8") == text(today)
