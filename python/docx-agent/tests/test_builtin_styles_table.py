"""The Rust core's table of Word's built-in style definitions (``crates/ooxml-word-edit/
data/builtin-styles.tsv``) is the one written from these measured definitions: one source."""

from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest

GENERATOR = Path(__file__).resolve().parents[3] / "crates" / "ooxml-word-edit" / "data" / "generate.py"


@pytest.mark.skipif(not GENERATOR.exists(), reason="outside the monorepo")
def test_the_rust_table_equals_the_measured_definitions():
    spec = importlib.util.spec_from_file_location("builtin_styles_generate", GENERATOR)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    assert module.TABLE.read_text(encoding="utf-8") == module.table()
