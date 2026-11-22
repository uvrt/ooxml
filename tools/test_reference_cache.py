"""Tests of the gates' reference cache's key (crates/ooxml-oracle/python/reference_cache.py): its
chain covers the reference's own code alone, so a harness change that only makes cases (a new
case, a case maker, the drivers) keeps every kept result, and a change to the code that runs
docx-agent on a case computes them all again.  Standard library only; ci.yml's plan job runs
them (`python3 -m unittest discover -s tools -p 'test_*.py'`)."""

from __future__ import annotations

import gc
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gates  # noqa: E402
import gates_frozen  # noqa: E402

sys.path.insert(0, str(gates.HARNESS))
import reference_cache  # noqa: E402

#: The harness modules that run docx-agent on a case (the chain's own harness files).
REFERENCE = ("word_edits_reference.py", "word_tool_reference.py")

#: A harness in miniature, laid out as the repository's (crates/ooxml-oracle/python): the
#: driver imports the case makers and the reference, as word_edits_diff.py does.
STUBS = {
    "word_edits_diff.py": "import word_edits_reference\nimport word_structure_cases\nimport word_edits_compare\n"
                          "import word_reads_diff\n",
    "word_reads_diff.py": "def run_rust():\n    return {}\n",
    "word_edits_compare.py": "def settle(a, b):\n    return b, None\n",
    "word_structure_cases.py": "def cases(document):\n    return [{'op': 'structure'}]\n",
    "word_edits_reference.py": "import word_tool_reference\n\ndef python_case(data, case, baseline):\n"
                               "    return {}\n",
    "word_tool_reference.py": "def reply(reply):\n    return reply\n",
}


class Chain(unittest.TestCase):
    """The chain's digest (tools/gates.py cache-key, the store's key on GitHub) on a harness in
    miniature: what changes it and what does not."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp(prefix="reference-cache-"))
        self.addCleanup(shutil.rmtree, self.root)
        self.harness = self.root / "crates" / "ooxml-oracle" / "python"
        self.harness.mkdir(parents=True)
        for name in ("reference_cache.py", "reference_packages.py"):
            shutil.copy(gates.HARNESS / name, self.harness / name)
        for name, text in STUBS.items():
            (self.harness / name).write_text(text)

    def chain(self) -> str:
        env = {k: v for k, v in os.environ.items() if k != "GATES_REFERENCE_SHA"}
        done = subprocess.run([sys.executable, "-E", "-s", str(self.harness / "reference_cache.py")], capture_output=True,
                              text=True, env=env, cwd=self.root, timeout=60, check=False)
        self.assertEqual(done.returncode, 0, done.stderr[-2000:])
        return done.stdout.strip()

    def digests(self, *names: str) -> list[str]:
        """The chain before and after a function is added to each of ``names`` in turn."""
        out = [self.chain()]
        for name in names:
            with (self.harness / name).open("a") as handle:
                handle.write("\n\ndef added():\n    return [{'op': 'new'}]\n")
            out.append(self.chain())
        return out

    def test_a_new_case_keeps_the_chain(self) -> None:
        self.assertEqual(len(set(self.digests("word_structure_cases.py"))), 1)

    def test_a_case_maker_added_keeps_the_chain(self) -> None:
        before = self.chain()
        (self.harness / "word_cells_cases.py").write_text("def cases(document):\n    return []\n")
        with (self.harness / "word_edits_diff.py").open("a") as handle:
            handle.write("import word_cells_cases\n")
        self.assertEqual(self.chain(), before)

    def test_the_drivers_and_comparison_keep_the_chain(self) -> None:
        self.assertEqual(len(set(self.digests(*reference_cache.DRIVERS))), 1)

    def test_the_reference_changes_the_chain(self) -> None:
        names = (*REFERENCE, "reference_cache.py", "reference_packages.py")
        self.assertEqual(len(set(self.digests(*names))), len(names) + 1)


class Split(unittest.TestCase):
    """The repository's harness: the reference's modules and the rest kept apart."""

    def test_the_drivers_and_every_case_maker_are_not_keyed(self) -> None:
        makers = {p.name for p in gates.HARNESS.glob("word_*_cases.py")}
        self.assertTrue(makers, "no case makers found")
        self.assertTrue(makers | set(reference_cache.DRIVERS) <= set(reference_cache.NOT_REFERENCE))
        self.assertTrue(set(gates.CASE_MAKERS) <= set(reference_cache.NOT_REFERENCE))

    def test_the_reference_is_keyed(self) -> None:
        for name in REFERENCE:
            self.assertTrue((gates.HARNESS / name).is_file(), name)
            self.assertNotIn(name, reference_cache.NOT_REFERENCE)

    def test_the_reference_imports_no_case_maker_or_driver(self) -> None:
        # a name it took from one (a constant, a helper) would change its results unkeyed
        imported = {p.name for p in gates_frozen.harness_modules(entries=REFERENCE)}
        self.assertEqual(imported & set(reference_cache.NOT_REFERENCE), set())


class Read(unittest.TestCase):
    """A run keeps the store's results of its own cases only (lane PF1: the whole store held
    in memory made every full garbage collection walk it, and a gate job ran three times as
    long)."""

    def setUp(self) -> None:
        self.folder = Path(tempfile.mkdtemp(prefix="reference-store-"))
        self.addCleanup(shutil.rmtree, self.folder)
        value = {"result": {"id": "p1", "renamed": {}}, "parts": {"word/document.xml": "sha256:00"}, "touched": [{}]}
        reference_cache._write(self.folder / "reference.jsonl.gz", {f"a{n}": value for n in range(5000)})
        reference_cache._write(self.folder / "other.jsonl.gz", {"b0": {"refused": "no"}})

    def test_only_the_wanted_results_are_kept(self) -> None:
        found = reference_cache.kept_entries(self.folder, {"a7", "b0", "missing"})
        self.assertEqual(set(found), {"a7", "b0"})
        self.assertEqual(found["b0"], {"refused": "no"})

    def test_the_rest_of_the_store_is_not_held(self) -> None:
        gc.collect()
        before = len(gc.get_objects())
        found = reference_cache.kept_entries(self.folder, {"a1"})
        gc.collect()
        # 5,001 entries read, one kept: the objects the collector tracks grow by that one's
        self.assertLess(len(gc.get_objects()) - before, 100)
        self.assertEqual(len(found), 1)


if __name__ == "__main__":
    unittest.main()
