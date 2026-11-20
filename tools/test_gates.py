"""Tests of tools/gates.py: the known differences (every entry loads, none names its case by
number, an entry covers its case however the run numbers it, and a document condition holds by
the document, not by its name) and the modes ``auto`` picks from a branch's changes.  Standard
library only;
ci.yml's plan job runs them (`python3 -m unittest discover -s tools -p 'test_*.py'`)."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gates  # noqa: E402

CHART_PLACES = "generated/charts/chart-places.docx"
FIXTURES = gates.FIXTURES
WP = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
#: A floated picture's case (word_object_cases.py), as the harness makes it.
FLOATED = {"op": "objects", "tool": "word_drawings",
           "arguments": {"doc": "d1", "action": "insert_picture", "at": "p@body/0@0", "wrap": "square",
                         "alt_text": "A floated picture for the oracle"}}
SEQ = {"op": "structure", "tool": "word_fields",
       "arguments": {"doc": "d1", "action": "insert_field", "at": "p@body/2@0",
                     "instruction": "SEQ Figure \\* ARABIC"}}


class KnownDifferences(unittest.TestCase):
    def test_entries_load(self) -> None:
        self.assertTrue(gates.known_entries())

    def test_a_case_number_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "known.toml"
            path.write_text('[[difference]]\nmode = "structure"\ncase = 3\ncause = "x"\n')
            with self.assertRaises(SystemExit):
                gates.known_entries(path)

    def test_covered_whatever_the_case_number(self) -> None:
        # the text-box SEQ field (#84), untracked and tracked, numbered as a full run and as
        # a sampled run number it (gate runs 44606340204, 44612839166: case 33)
        entries = gates.known_entries()
        for number in (3, 28, 33, 173, 198):
            for tracked in (None, {"author": "Writer A", "date": "2026-11-17T10:00:00Z"}):
                case = {"case": number, "input": {**SEQ, "tracked": tracked}}
                self.assertIsNotNone(gates.covering(entries, "fixtures", "structure", CHART_PLACES, case))

    def test_another_place_is_not_covered(self) -> None:
        entries = gates.known_entries()
        case = {"case": 3, "input": {**SEQ, "arguments": {**SEQ["arguments"], "at": "p@body/0@0"}}}
        self.assertIsNone(gates.covering(entries, "fixtures", "structure", CHART_PLACES, case))


class DocumentCondition(unittest.TestCase):
    def covers(self, relative: str, case: dict | None = None) -> dict | None:
        source = str(FIXTURES / relative)
        return gates.covering(gates.known_entries(), "fixtures", "objects", relative,
                              {"case": 7, "input": {**(case or FLOATED), "source": source}}, source)

    def test_every_fixture_without_the_declaration_is_covered(self) -> None:
        # one rule for the ns0 prefix, not an entry per fixture (lane M3)
        lacking = [f for f in sorted(FIXTURES.rglob("*.docx"))
                   if WP not in gates.root_namespaces(str(f), "word/document.xml")]
        self.assertGreaterEqual(len(lacking), 12)
        for path in lacking:
            relative = str(path.relative_to(FIXTURES))
            for action in ("float", "inline"):
                self.assertIsNotNone(self.covers(relative, {**FLOATED, "arguments": {"action": action}}), relative)
            self.assertIsNotNone(self.covers(relative), relative)

    def test_a_fixture_with_the_declaration_is_not_covered(self) -> None:
        declaring = next(f for f in sorted(FIXTURES.rglob("*.docx"))
                         if WP in gates.root_namespaces(str(f), "word/document.xml"))
        self.assertIsNone(self.covers(str(declaring.relative_to(FIXTURES))))

    def test_another_picture_is_not_covered(self) -> None:
        inline = {**FLOATED, "arguments": {**FLOATED["arguments"], "alt_text": "An inline picture"}}
        self.assertIsNone(self.covers("generated/controls.docx", inline))

    def test_no_document_is_not_covered(self) -> None:
        entries = gates.known_entries()
        case = {"case": 7, "input": FLOATED}
        self.assertIsNone(gates.covering(entries, "fixtures", "objects", "generated/controls.docx", case))
        self.assertIsNone(gates.covering(entries, "fixtures", "objects", "x.docx", case, "/no/such/x.docx"))

    def test_a_malformed_condition_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "known.toml"
            for condition in ('"word/document.xml"', '{ part = "word/document.xml" }',
                              '{ lacks_namespace = "x", root = "y" }'):
                path.write_text(f'[[difference]]\nmode = "objects"\ndocument = {condition}\ncause = "x"\n')
                with self.assertRaises(SystemExit):
                    gates.known_entries(path)


class AutoModes(unittest.TestCase):
    """gates.py plan: the modes a branch's changes can affect."""

    def modes(self, *changed: str) -> tuple[set[str], bool]:
        modes, reads = gates.auto_modes(list(changed))
        return set(modes), reads

    def test_notes_and_other_packages_run_nothing(self) -> None:
        self.assertEqual(self.modes("README.md", "docs/architecture/x.md", "python/docx-agent/CHANGELOG.md",
                                    "crates/ooxml-word-edit/README.md", "python/pptx-agent/src/pptx_agent/x.py",
                                    ".github/workflows/ci.yml", "tools/ci.py", "python/docx-agent/tests/test_x.py",
                                    "crates/ooxml-word-edit/tests/integration/x.rs"), (set(), False))

    def test_the_bindings_run_nothing(self) -> None:
        # the oracle binaries are not built from them (gates run docx-agent with Rust off)
        self.assertEqual(self.modes("crates/python/src/lib.rs", "crates/ooxml-python-session/src/lib.rs",
                                    "python/ooxml-core/src/x.rs"), (set(), False))

    def test_a_case_maker_runs_its_modes(self) -> None:
        self.assertEqual(self.modes("crates/ooxml-oracle/python/word_style_cases.py"), ({"style"}, False))
        self.assertEqual(self.modes("crates/ooxml-oracle/python/word_structure_cases.py"),
                         ({"structure", "objects", "style"}, False))
        self.assertEqual(self.modes("crates/ooxml-oracle/python/word_text_cases.py",
                                    "crates/ooxml-oracle/python/word_object_cases.py"), ({"text", "objects"}, False))

    def test_a_binary_runs_its_gate(self) -> None:
        self.assertEqual(self.modes("crates/ooxml-oracle/src/bin/word-reads.rs"), (set(), True))
        self.assertEqual(self.modes("crates/ooxml-oracle/src/bin/word-edits.rs"), (set(gates.EDIT_MODES), False))
        self.assertEqual(self.modes("crates/ooxml-oracle/src/bin/layout-bench.rs"), (set(), False))

    def test_code_the_gates_run_runs_every_mode(self) -> None:
        for path in ("crates/ooxml-word-edit/src/set_text.rs", "crates/ooxml-tools-word/src/describe.md",
                     "crates/ooxml-oracle/src/edits.rs", "python/docx-agent/src/docx_agent/rust/edits.py",
                     "python/ooxml-edit/src/ooxml_edit/tools/served.py",
                     "python/docx-agent/tests/fixtures/generated/x.docx", "python/docx2svg/tools/make_lists.py",
                     "crates/ooxml-oracle/python/word_edits_diff.py", "crates/ooxml-oracle/known-differences.toml",
                     "tools/gates.py", ".github/workflows/gates.yml", "Cargo.lock", "a/new/place.txt"):
            self.assertEqual(self.modes(path), (set(gates.EDIT_MODES), True), path)

    def test_no_change_is_every_mode(self) -> None:
        self.assertEqual(self.modes(), (set(gates.EDIT_MODES), True))

    def test_the_names_exist(self) -> None:
        crates = gates.oracle_crates()
        self.assertIn("ooxml-word-edit", crates)
        self.assertIn("ooxml-tools-word", crates)
        self.assertNotIn("python", crates)
        self.assertNotIn("ooxml-python-session", crates)
        for name in gates.CASE_MAKERS:
            self.assertTrue((gates.HARNESS / name).is_file(), name)
        for name in gates.GATE_BINARIES:
            self.assertTrue((gates.ROOT / "crates" / "ooxml-oracle" / "src" / "bin" / f"{name}.rs").is_file(), name)
        for name in gates.GATE_PACKAGES:
            self.assertTrue((gates.ROOT / "python" / name / "src").is_dir(), name)
        self.assertEqual(set(gates.READ_MODES.values()) | {"word-edits"}, set(gates.GATE_BINARIES))

    def test_the_case_makers_are_not_keyed(self) -> None:
        # reference_cache.py leaves them out of the reference's key (only stdlib at import)
        sys.path.insert(0, str(gates.HARNESS))
        import reference_cache

        self.assertTrue(set(gates.CASE_MAKERS) <= set(reference_cache.NOT_REFERENCE))


if __name__ == "__main__":
    unittest.main()
