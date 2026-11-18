"""The editing tools' Rust definitions (``crates/ooxml-tools-word``, golden ``tests/golden``)
equal today's byte for byte: the review tools ``word_set_tracking``, ``word_changes`` and
``word_comments`` (lane P4a), and ``word_set_text`` and ``replace_text`` (lane P2c).  Their
handlers' equality with today's is the edit oracle's
(``crates/ooxml-oracle/python/word_edits_diff.py --review``, ``--tools``, ``--text``).
"""
from __future__ import annotations

from pathlib import Path

import pytest

from docx_agent.tools import TOOLS

HERE = Path(__file__).resolve().parent
GOLDEN = HERE.parents[2] / "crates" / "ooxml-tools-word" / "tests" / "golden"


@pytest.mark.parametrize("name", ["word_set_tracking", "word_changes", "word_comments", "word_set_text",
                                  "replace_text"])
def test_the_rust_definitions_are_todays_byte_for_byte(name):
    from test_rust_tool_specs import export, text

    today = {tool.name: tool for tool in TOOLS}[name]
    assert (GOLDEN / f"{name}.json").read_text(encoding="utf-8") == text(export(today))
