"""The Rust core's Markdown tables (``crates/ooxml-markdown``: Unicode punctuation and the
HTML entity names markdown-it reads) are the ones this Python and markdown-it give: one
source."""

from __future__ import annotations

import importlib.util
import unicodedata
from pathlib import Path

import pytest

GENERATOR = Path(__file__).resolve().parents[3] / "crates" / "ooxml-markdown" / "data" / "generate.py"


@pytest.mark.skipif(not GENERATOR.exists(), reason="outside the monorepo")
def test_the_rust_tables_equal_markdown_its():
    spec = importlib.util.spec_from_file_location("markdown_tables_generate", GENERATOR)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    assert module.ENTITIES.read_text(encoding="utf-8") == module.entities()
    written = module.TABLE.read_text(encoding="utf-8")
    # The punctuation table is one Unicode version's (the one its header names); another
    # Python's unicodedata may be a later version, whose table is its own.
    if f"Unicode {unicodedata.unidata_version}'s" in written:
        assert written == module.table()
