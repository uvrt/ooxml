"""tools/gates_goldens.py: a run's records turned into goldens and blessed."""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import MappingProxyType
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gates
import gates_goldens
from gates_goldens import goldens

CASE = {"tool": "word_fields", "arguments": {"doc": "d1", "action": "list"}}
OTHER = {"tool": "word_notes", "arguments": {"doc": "d1", "action": "list"}}
ONE = MappingProxyType({"rust": {"x": 1}})
NINE = MappingProxyType({"rust": {"x": 9}})


class Bless(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        base = self.tmp / "oracle"
        patches = {"CASES": base / "cases", "GOLDENS": base / "goldens", "PENDING": base / "goldens" / "pending",
                   "INPUTS": base / "cases" / "inputs"}
        for name, value in patches.items():
            patcher = mock.patch.object(goldens, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def raw(self, out: Path, tree: Path, lines: list[dict]) -> None:
        out.mkdir(parents=True, exist_ok=True)
        goldens.write_lines(out / gates_goldens.RAW, lines)

    def record(self, name: str, lines: list[dict], *, mode="structure", reference=True, per_tool=0) -> Path:
        out = self.tmp / "run" / name
        out.mkdir(parents=True, exist_ok=True)
        goldens.write_lines(out / gates_goldens.RECORD, lines)
        (out / gates_goldens.META).write_text(
            f'{{"gate": "fixtures", "mode": "{mode}", "per_tool": {per_tool}, "every": 1, '
            f'"reference": {"true" if reference else "false"}}}')
        return out

    def line(self, case: dict, output=ONE, source="frozen Python a512c901, unmeasured", **more) -> dict:
        return {"tree": "fixtures", "document": "a.docx", "mode": "structure", "id": goldens.case_id("structure", case),
                "case": case, "digest": goldens.digest_of(dict(output)), "output": dict(output), "source": source,
                **more}

    def test_finish_gives_each_case_its_source(self) -> None:
        tree = self.tmp / "tree"
        out = self.tmp / "out"
        third = {**CASE, "arguments": {"doc": "d1", "action": "update"}}
        self.raw(out, tree, [
            {"file": str(tree / "a.docx"), "case": CASE, "digest": "sha256:1", "output": {}, "differs": False},
            {"file": str(tree / "a.docx"), "case": OTHER, "digest": "sha256:2", "output": {}, "differs": True},
            {"file": str(tree / "a.docx"), "case": third, "digest": "sha256:3", "output": {}, "differs": True},
            {"file": None, "case": {"case": "f"}, "not_supported": "a form", "python": {"x": 1}}])
        def cover(file, case):
            return {"cause": "#1: why\nmore"} if case.get("tool") == "word_notes" else None

        uncovered = gates_goldens.finish(out, {"gate": "fixtures", "mode": "structure", "per_tool": 0, "every": 1},
                                         tree, cover)
        lines = goldens.read_lines(out / gates_goldens.RECORD)
        self.assertEqual([x["source"] for x in lines],
                         [gates_goldens.frozen_source(), "known difference: #1: why", None, gates_goldens.frozen_source()])
        self.assertEqual(uncovered, 1)
        self.assertEqual((lines[3]["document"], lines[3]["not_supported"]), (goldens.MADE, "a form"))

    def test_a_reference_record_freezes_the_cases_and_their_goldens(self) -> None:
        folder = self.record("a", [self.line(CASE), self.line(OTHER, source="known difference: x"),
                                   {**self.line({"case": "r"}), "not_supported": "f", "python": {"y": 1}}])
        code, lines = gates_goldens.bless(self.tmp / "run", run="https://example.invalid/run/1")
        self.assertEqual(code, 0, lines)
        cases = goldens.read_lines(goldens.CASES / "fixtures" / "a.docx.jsonl")
        self.assertEqual([c["case"] for c in cases], [CASE, OTHER, {"case": "r"}])
        blessed = goldens.goldens_of("fixtures", "a.docx")
        self.assertEqual([g.get("digest") for g in blessed.values()], [goldens.digest_of(dict(ONE)), goldens.digest_of(dict(ONE)), None])
        self.assertEqual(goldens.read_lines(goldens.PENDING / "fixtures" / "a.docx.jsonl.gz")[0]["python"], {"y": 1})
        self.assertIn("| fixtures | structure | 3 | 0 | 0 | 0 | 0 | 0 |", lines)
        self.assertIn("https://example.invalid/run/1", "\n".join(lines))
        # again: nothing changes, and a case no longer made is removed with its golden
        folder.joinpath(gates_goldens.RECORD).unlink()
        self.record("a", [self.line(CASE)])
        code, lines = gates_goldens.bless(self.tmp / "run")
        self.assertIn("| fixtures | structure | 0 | 0 | 0 | 0 | 2 | 1 |", lines)
        self.assertEqual(list(goldens.goldens_of("fixtures", "a.docx")), [goldens.case_id("structure", CASE)])

    def test_a_changed_answer_needs_a_source(self) -> None:
        self.record("a", [self.line(CASE)])
        gates_goldens.bless(self.tmp / "run")
        changed = self.record("a", [{**self.line(CASE, output=NINE), "source": None}], reference=False)
        code, lines = gates_goldens.bless(self.tmp / "run")
        self.assertEqual(code, 1)
        self.assertIn("without a source", "\n".join(lines))
        self.assertEqual(goldens.goldens_of("fixtures", "a.docx")[goldens.case_id("structure", CASE)]["digest"],
                         goldens.digest_of(dict(ONE)))
        code, lines = gates_goldens.bless(changed.parent, source="Word 16.106: e4-word.json#fields")
        self.assertEqual(code, 0, lines)
        golden = goldens.goldens_of("fixtures", "a.docx")[goldens.case_id("structure", CASE)]
        self.assertEqual((golden["digest"], golden["source"]), (goldens.digest_of(dict(NINE)), "Word 16.106: e4-word.json#fields"))
        # a golden run's record keeps the case list
        self.assertEqual(len(goldens.read_lines(goldens.CASES / "fixtures" / "a.docx.jsonl")), 1)

    def test_a_sampled_reference_record_keeps_the_case_list(self) -> None:
        self.record("a", [self.line(CASE), self.line(OTHER)])
        gates_goldens.bless(self.tmp / "run")
        self.record("a", [self.line(CASE)], per_tool=1)
        code, lines = gates_goldens.bless(self.tmp / "run")
        self.assertEqual(code, 0)
        self.assertEqual(len(goldens.read_lines(goldens.CASES / "fixtures" / "a.docx.jsonl")), 2)
        self.assertIn("fixtures/structure", "\n".join(lines))

    def test_a_check_writes_nothing(self) -> None:
        self.record("a", [self.line(CASE)])
        code, _ = gates_goldens.bless(self.tmp / "run", write=False)
        self.assertEqual(code, 0)
        self.assertFalse((goldens.GOLDENS / "fixtures").exists())
        self.assertFalse(gates_goldens.on_goldens("fixtures"))

    def test_inputs_are_kept(self) -> None:
        out = self.record("a", [self.line({**CASE, "input": "$INPUT/ab.docx"})])
        (out / "inputs").mkdir()
        (out / "inputs" / "ab.docx").write_bytes(b"x")
        gates_goldens.bless(self.tmp / "run")
        self.assertEqual((goldens.INPUTS / "ab.docx").read_bytes(), b"x")
        self.assertTrue(gates_goldens.on_goldens("fixtures"))
        self.assertTrue(gates_goldens.on_goldens("reads"))
        self.assertFalse(gates_goldens.on_goldens("corpus"))


class DiffOnGoldens(unittest.TestCase):
    """gates.py diff: a frozen tree with goldens runs golden_gate.py, without docx-agent, and
    its known differences are the goldens' own."""

    def run_diff(self, on_goldens: bool, **more) -> tuple[list[str], dict]:
        ran = []

        def run(command, **_):
            ran.append(command)
            results = Path(command[command.index("--results") + 1])
            results.write_text(json.dumps({"file": "/x/a.docx", "case": "objects:1", "input": {"tool": "word_x"},
                                           "differences": ["d"]}) + "\n"
                               + json.dumps({"totals": {"cases": 2, "panics": 0, "not_supported_forms": {},
                                                        "golden_known": 1}}) + "\n")
            return subprocess.CompletedProcess(command, 1)

        with tempfile.TemporaryDirectory() as tmp:
            args = argparse.Namespace(record=True, reference=False, out=tmp, tree=None, gate="fixtures", mode="objects",
                                      shard="1/1", every=1, per_tool=0, fixtures="generated/controls.docx",
                                      base=None, cache=None, bin="/bin", **more)
            with mock.patch.object(gates_goldens, "on_goldens", lambda gate: on_goldens), \
                    mock.patch.object(gates.subprocess, "run", run), \
                    mock.patch.object(gates, "known_entries", lambda: [{"mode": "objects", "cause": "c",
                                                                       "input": {"tool": "word_x"}}]), \
                    contextlib.redirect_stdout(io.StringIO()):
                gates.run_diff(args)
            return ran[0], json.loads((Path(tmp) / "summary.json").read_text())

    def test_goldens_run_the_gate_without_the_reference(self) -> None:
        command, summary = self.run_diff(True)
        self.assertTrue(command[1].endswith("golden_gate.py"), command)
        self.assertIn("--made", command)
        self.assertEqual(command[-1], "generated/controls.docx")
        self.assertTrue(command[command.index("--rust") + 1].endswith("word-edits"))
        # a known difference does not cover a case on goldens
        self.assertEqual((summary["differences"], summary["known"], summary["goldens"]), (1, 1, True))

    def test_without_goldens_the_reference_records(self) -> None:
        command, summary = self.run_diff(False)
        self.assertTrue(command[1].endswith("word_edits_diff.py"), command)
        self.assertIn("--record", command)
        self.assertEqual((summary["differences"], summary["known"], summary["goldens"]), (0, 1, False))


    def test_goldens_plan_every_mode_warm(self) -> None:
        printed = io.StringIO()
        with mock.patch.object(gates_goldens, "on_goldens", lambda gate: True), contextlib.redirect_stdout(printed), \
                contextlib.redirect_stderr(io.StringIO()):
            gates.main(["matrix", "--gates", "fixtures,corpus", "--modes", "all", "--fixtures", "all"])
        jobs = json.loads(printed.getvalue())["include"]
        self.assertTrue(jobs)
        # warm: sized by the recorded warm times, never the cold three shards a mode
        self.assertFalse([job for job in jobs if job["shard"].endswith("/3")])


if __name__ == "__main__":
    unittest.main()
