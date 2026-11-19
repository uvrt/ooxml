"""The editing tools' Rust definitions (``crates/ooxml-tools-word``, golden ``tests/golden``)
equal today's byte for byte: the review tools ``word_set_tracking``, ``word_changes`` and
``word_comments`` (lane P4a), and ``word_set_text``, ``replace_text``, ``word_insert_text`` and
``word_delete`` (lane P2c; a placing alias written as the kind of place it reads).  Their
handlers' equality with today's is the edit oracle's
(``crates/ooxml-oracle/python/word_edits_diff.py --review``, ``--tools``, ``--text``).
"""
from __future__ import annotations

import inspect
from pathlib import Path

import pytest

from docx_agent.tools import TOOLS

HERE = Path(__file__).resolve().parent
GOLDEN = HERE.parents[2] / "crates" / "ooxml-tools-word" / "tests" / "golden"


def _alias(value):
    if isinstance(value, str):
        return value
    return "place:" + inspect.getclosurevars(value).nonlocals["kind"]


@pytest.mark.parametrize("name", ["word_set_tracking", "word_changes", "word_comments", "word_set_text",
                                  "replace_text", "word_insert_text", "word_delete"])
def test_the_rust_definitions_are_todays_byte_for_byte(name):
    from test_rust_tool_specs import export, text

    today = export({tool.name: tool for tool in TOOLS}[name])
    today["aliases"] = {key: _alias(value) for key, value in today["aliases"].items()}
    assert (GOLDEN / f"{name}.json").read_text(encoding="utf-8") == text(today)
