"""The frozen reference: where the harness imports it from (crates/ooxml-oracle/python/
reference_packages.py) and the check that the harness names only what it has
(tools/gates_frozen.py).  Run: ``python3 -m unittest discover -s tools -p 'test_*.py'``."""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gates_frozen  # noqa: E402
import reference_packages  # noqa: E402  (gates_frozen puts the harness on the path)

#: A made-up reference: docx_agent with a package-level name, a submodule with a private
#: class, and a module whose names cannot be listed.
SOURCES = {
    "docx_agent": "from docx_agent.document import Document\nVERSION = '1'\n",
    "docx_agent.document": "class Document:\n    pass\n",
    "docx_agent.layout_facts": "class _Facts:\n    pass\nif True:\n    _W = 1\n",
    "docx_agent.dynamic": "def __getattr__(name):\n    return name\n",
}
HARNESS = """\
import docx_agent.layout_facts
from docx_agent import Document, layout_facts, gone
from docx_agent.layout_facts import _Facts, _Missing
from docx_agent.dynamic import anything
from docx_agent.nowhere import x
import helper

a = docx_agent.layout_facts._W
b = docx_agent.layout_facts._lost
c = layout_facts._Facts.method
d = Document.open
"""


def reference() -> gates_frozen.Reference:
    return gates_frozen.Reference({name: name for name in SOURCES}, SOURCES.__getitem__)


class FrozenApi(unittest.TestCase):
    def missing(self, files: dict[str, str]) -> list[str]:
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for name, text in files.items():
                (root / name).write_text(text)
            found = gates_frozen.missing(reference(), harness=root, root=root)
        return sorted(f"{u.module}.{u.name}" if u.name else u.module for u in found)

    def test_names_the_reference_lacks_are_found(self) -> None:
        found = self.missing({"word_edits_diff.py": HARNESS, "word_reads_diff.py": "import json\n",
                              "helper.py": "from docx_agent.layout_facts import _Gone\n",
                              "unused.py": "from docx_agent import never_imported\n"})
        self.assertEqual(found, ["docx_agent.gone", "docx_agent.layout_facts._Gone", "docx_agent.layout_facts._Missing",
                                 "docx_agent.layout_facts._lost", "docx_agent.nowhere.x"])

    def test_the_harness_at_head_names_only_the_pinned_reference(self) -> None:
        sha = reference_packages.pinned_sha()
        if sha is None or subprocess.run(["git", "cat-file", "-e", f"{sha}^{{commit}}"], cwd=gates_frozen.ROOT,
                                         capture_output=True).returncode:
            self.skipTest("the frozen reference's commit is not fetched (the gates' plan job checks it)")
        self.assertEqual(gates_frozen.missing(gates_frozen.Reference.at(sha)), [])

    def test_the_gates_harness_is_followed(self) -> None:
        names = {path.name for path in gates_frozen.harness_modules()}
        self.assertTrue({"word_edits_diff.py", "word_reads_diff.py", "word_facts_cases.py", "facts_documents.py",
                         "shelf_tables.py", "reference_cache.py"} <= names, names)


class ReferencePackages(unittest.TestCase):
    def test_the_pin_is_a_full_commit(self) -> None:
        sha = reference_packages.pinned_sha()
        self.assertIsNotNone(sha)
        with tempfile.TemporaryDirectory() as folder:
            short = Path(folder) / "reference.toml"
            short.write_text('sha = "a512c90"\n')
            with self.assertRaises(SystemExit):
                reference_packages.pinned_sha(short)

    def test_frozen_runs_leave_the_checkout_off_the_path(self) -> None:
        sha = reference_packages.pinned_sha()
        before, env = list(sys.path), os.environ.get(reference_packages.ENV)
        try:
            os.environ[reference_packages.ENV] = sha
            reference_packages.use_sources("docx-agent", "pptx2svg")
            added = [p for p in sys.path if p not in before]
            self.assertEqual(added, [str(reference_packages.ROOT / "python" / "pptx2svg" / "src")])
            os.environ[reference_packages.ENV] = "0" * 40
            with self.assertRaises(SystemExit):
                reference_packages.frozen_sha()
        finally:
            sys.path[:] = before
            if env is None:
                os.environ.pop(reference_packages.ENV, None)
            else:
                os.environ[reference_packages.ENV] = env

    def test_runs_by_hand_put_the_checkout_first(self) -> None:
        before, env = list(sys.path), os.environ.pop(reference_packages.ENV, None)
        try:
            reference_packages.use_sources("docx-agent")
            self.assertEqual(sys.path[0], str(reference_packages.ROOT / "python" / "docx-agent" / "src"))
            self.assertEqual(reference_packages.installed_sources(), [])
        finally:
            sys.path[:] = before
            if env is not None:
                os.environ[reference_packages.ENV] = env


if __name__ == "__main__":
    unittest.main()
