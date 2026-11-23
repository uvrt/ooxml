"""The names the sibling packages import from docx2svg: removing one fails here.

``USED_BY`` is that surface, each name with the siblings that import it (in their ``src``,
``tests`` and ``tools``); docs/api.md ("Used by docx-agent") documents it, and
``test_the_documented_names_are_these`` holds the two to the same names.  A name withdrawn
goes to ``docx2svg.WITHDRAWN`` (with what to use instead) for one release.
"""

from __future__ import annotations

import importlib
import re
from pathlib import Path

import pytest

import docx2svg

DOC = Path(__file__).resolve().parent.parent / "docs/api.md"
SECTION = "## Used by docx-agent"

#: module -> {name: the siblings that import it}.
USED_BY: dict[str, dict[str, str]] = {
    "docx2svg": {
        "ConvertOptions": "docx-agent",
        "convert_docx": "docx-agent",
        "convert_docx_to_svg": "docx-agent",
    },
    "docx2svg.rust": {
        "engine_key": "docx-agent",
        "memory_budget": "docx-agent",
        "page_setup": "docx-agent",
        "word_fonts": "docx-agent",
    },
}


def _used() -> list[tuple[str, str]]:
    return [(module, name) for module, names in USED_BY.items() for name in names]


@pytest.mark.parametrize(("module", "name"), _used())
def test_a_name_a_sibling_imports_is_there(module, name):
    assert hasattr(importlib.import_module(module), name), f"{module}.{name} is gone, and {USED_BY[module][name]} " \
        "imports it"


def test_the_public_names_are_these():
    assert sorted(docx2svg.__all__) == sorted([
        "Conversion", "ConvertOptions", "LayoutError", "RasterizerNotAvailable", "Warning", "available_backends",
        "convert_docx", "convert_docx_to_png", "convert_docx_to_svg", "embed_glyphs", "glyph_sheet",
        "resolve_path", "svg_to_png", "user_font_dirs", "__version__"])
    for name in docx2svg.__all__:
        assert hasattr(docx2svg, name), name
    assert not set(docx2svg.__all__) & set(docx2svg.WITHDRAWN)


def _documented() -> set[tuple[str, str]]:
    lines = DOC.read_text(encoding="utf-8").splitlines()
    start = lines.index(SECTION) + 1
    end = next((i for i in range(start, len(lines)) if lines[i].startswith("## ")), len(lines))
    found = set()
    for line in lines[start:end]:
        if line.startswith("- `"):
            module, *names = re.findall(r"`([^`]+)`", line)
            found.update((module, name) for name in names)
    return found


def test_the_documented_names_are_these():
    assert _documented() == set(_used())
