"""Tests of tools/gates.py's known differences: every entry loads, none names its case by
number, and an entry covers its case however the run numbers it.  Standard library only;
ci.yml's plan job runs them (`python3 -m unittest discover -s tools -p 'test_*.py'`)."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gates  # noqa: E402

CHART_PLACES = "generated/charts/chart-places.docx"
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


if __name__ == "__main__":
    unittest.main()
