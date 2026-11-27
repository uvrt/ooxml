"""``ooxml_core`` is internal (decision D6, Phase 3 E2): applications and the format libraries use
ooxml-edit's facade (``ooxml_edit.tools``, ``ooxml_edit.tools.hosted``, ``ooxml_edit.chunks``).
This holds every Python file of the repository to that: only the internal layer below imports the
Rust core's module.  Run: ``python3 -m unittest discover -s tools -p 'test_*.py'``."""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

#: The internal layer: who may import ``ooxml_core``, and why.
ALLOWED = {
    "python/ooxml-edit/src/ooxml_edit/": "the facade itself: the Rust host and the chunk writer reach the core here",
    "python/docx2svg/src/docx2svg/": "the Word renderer, a thin wrapper of the core's layout and drawing",
    "python/ooxml-core/": "the core's own package and tests",
    "crates/ooxml-oracle/": "the oracle's and the gates' harness, which drive the core directly",
}

#: Where the format libraries live: none of their files (sources, tests, tools) imports it.
LIBRARIES = ("python/docx-agent/", "python/xlsx-agent/", "python/pptx-agent/")

_IMPORT = re.compile(r"^\s*(?:import\s+ooxml_core\b|from\s+ooxml_core\b)"
                     r"|(?:import_module|importorskip|__import__)\(\s*[\"']ooxml_core[\"']", re.M)


def importers() -> list[str]:
    """Every Python file of the repository that imports ``ooxml_core``, relative to the root."""
    out = []
    for path in sorted(ROOT.rglob("*.py")):
        relative = path.relative_to(ROOT).as_posix()
        if "/.venv/" in f"/{relative}" or relative.startswith(("target/", ".git/")):
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        if _IMPORT.search(text):
            out.append(relative)
    return out


class CoreBoundary(unittest.TestCase):
    def test_no_format_library_imports_the_core(self) -> None:
        found = [path for path in importers() if path.startswith(LIBRARIES)]
        self.assertEqual(found, [], "ooxml_core is internal: use ooxml_edit.tools.hosted or ooxml_edit.chunks")

    def test_only_the_internal_layer_imports_the_core(self) -> None:
        found = [path for path in importers() if not path.startswith(tuple(ALLOWED))]
        self.assertEqual(found, [], "a new importer of ooxml_core: go through the facade, or add it to "
                                    "ALLOWED with the reason")


if __name__ == "__main__":
    unittest.main()
