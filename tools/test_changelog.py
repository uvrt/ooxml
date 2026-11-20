"""Tests of tools/changelog.py (changelog fragments) on made-up packages, and of tools/ci.py's
refusal of a direct CHANGELOG.md edit.  Standard library only; ci.yml's plan job runs them
(`python3 -m unittest discover -s tools -p 'test_*.py'`)."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import changelog  # noqa: E402
import ci  # noqa: E402
import release_notes  # noqa: E402

HEAD = "# Changelog\n\nWhat this package is.\n\n"


def make_package(py: Path, name: str, version: str, changelog_text: str, module_version: str | None = None) -> Path:
    folder = py / name
    (folder / "src" / name.replace("-", "_")).mkdir(parents=True)
    (folder / "pyproject.toml").write_text(f'[project]\nname = "{name}"\nversion = "{version}"\n\n[tool.x]\n'
                                           f'version = "9.9"\n', encoding="utf-8")
    init = folder / "src" / name.replace("-", "_") / "__init__.py"
    init.write_text(f'__version__ = "{module_version or version}"\n', encoding="utf-8")
    (folder / "CHANGELOG.md").write_text(changelog_text, encoding="utf-8")
    return folder


def add(folder: Path, name: str, text: str) -> Path:
    (folder / changelog.CHANGES).mkdir(exist_ok=True)
    path = folder / changelog.CHANGES / name
    path.write_text(text, encoding="utf-8")
    return path


class ChangelogTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.py = Path(self.tmp.name)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_check_the_version_against_the_top_section(self) -> None:
        pkg = make_package(self.py, "pkg", "1.2.0", HEAD + "## Unreleased\n\nold\n\n## 1.2.0 -- unreleased\n\nx\n")
        self.assertEqual(changelog.problems(pkg), [])
        # A raised version needs a fragment that says what changed.
        pkg = make_package(self.py, "raised", "1.3.0", HEAD + "## 1.2.0 -- unreleased\n\nx\n")
        self.assertIn("add a fragment", changelog.problems(pkg)[0])
        add(pkg, "lane-a.added.md", "Something new.\n")
        self.assertEqual(changelog.problems(pkg), [])
        # Older than the changelog, or a module that disagrees.
        pkg = make_package(self.py, "older", "1.1.0", HEAD + "## 1.2.0 -- unreleased\n\nx\n")
        self.assertIn("older than", changelog.problems(pkg)[0])
        pkg = make_package(self.py, "module", "1.2.0", HEAD + "## 1.2.0\n", module_version="1.1.0")
        self.assertIn("__version__", changelog.problems(pkg)[0])

    def test_fragment_names_and_content(self) -> None:
        pkg = make_package(self.py, "pkg", "1.0.0", HEAD + "## 1.0.0\n")
        add(pkg, "Lane.added.md", "x")
        add(pkg, "lane.improved.md", "x")
        add(pkg, "lane-b.fixed.md", "  \n")
        add(pkg, "lane-c.removed.md", "Gone.")
        found, wrong = changelog.fragments(pkg)
        self.assertEqual([f.path.name for f in found], ["lane-c.removed.md"])
        self.assertEqual(len(wrong), 3)
        self.assertEqual(len(changelog.problems(pkg)), 3)

    def test_release_into_a_new_section(self) -> None:
        pkg = make_package(self.py, "pkg", "1.3.0", HEAD + "## 1.2.0 -- unreleased\n\nOld text.\n")
        add(pkg, "zeta.fixed.md", "Fixed z.\n")
        add(pkg, "beta.added.md", "- Added b,\n  two lines.\n")
        add(pkg, "alpha.added.md", "Added a.\n")
        other = make_package(self.py, "other", "2.0.0", HEAD + "## 2.0.0\n")
        done = changelog.release("v2026.11.22", changelog.packages(self.py))
        self.assertEqual(len(done), 3)
        self.assertEqual(changelog.pending([pkg, other]), [])
        self.assertEqual((pkg / "CHANGELOG.md").read_text(encoding="utf-8"), HEAD + (
            "## 1.3.0 -- v2026.11.22\n\n### Added\n\nAdded a.\n\n- Added b,\n  two lines.\n\n"
            "### Fixed\n\nFixed z.\n\n## 1.2.0 -- unreleased\n\nOld text.\n"))
        self.assertEqual((other / "CHANGELOG.md").read_text(encoding="utf-8"), HEAD + "## 2.0.0\n")
        self.assertEqual(changelog.problems(pkg), [])

    def test_release_into_the_versions_own_section(self) -> None:
        text = HEAD + "## 1.2.0 -- unreleased\n\nEarlier text.\n\n\n## 1.1.0 -- unreleased\n\nOlder.\n"
        pkg = make_package(self.py, "pkg", "1.2.0", text)
        add(pkg, "lane.changed.md", "Changed c.\n")
        changelog.release("v2026.11.22", [pkg])
        self.assertEqual((pkg / "CHANGELOG.md").read_text(encoding="utf-8"), HEAD + (
            "## 1.2.0 -- unreleased\n\nEarlier text.\n\n### Changed\n\nChanged c.\n\n\n"
            "## 1.1.0 -- unreleased\n\nOlder.\n"))
        # At the end of the file too.
        pkg = make_package(self.py, "last", "1.0.0", HEAD + "## 1.0.0\n\nText.\n")
        add(pkg, "lane.removed.md", "Removed r.")
        changelog.release("v2026.11.22", [pkg])
        self.assertEqual((pkg / "CHANGELOG.md").read_text(encoding="utf-8"),
                         HEAD + "## 1.0.0\n\nText.\n\n### Removed\n\nRemoved r.\n")

    def test_a_wrong_fragment_writes_nothing(self) -> None:
        good = make_package(self.py, "good", "1.0.0", HEAD + "## 1.0.0\n")
        add(good, "lane.added.md", "Added.\n")
        bad = make_package(self.py, "bad", "1.0.0", HEAD + "## 1.0.0\n")
        add(bad, "lane.misc.md", "x")
        with self.assertRaises(ValueError):
            changelog.release("v2026.11.22", changelog.packages(self.py))
        self.assertEqual((good / "CHANGELOG.md").read_text(encoding="utf-8"), HEAD + "## 1.0.0\n")
        self.assertTrue((good / changelog.CHANGES / "lane.added.md").exists())

    def test_the_repository_agrees(self) -> None:
        for folder in changelog.packages():
            self.assertEqual(changelog.problems(folder), [], folder.name)


class RefusalTest(unittest.TestCase):
    """tools/ci.py plan refuses a pull request's CHANGELOG.md edit, but a release branch's."""

    def test_pull_request(self) -> None:
        refused = ci.changelog_refusal("pull_request", ["python/docx-agent/CHANGELOG.md", "x.py"], "lane-x")
        self.assertIn("python/docx-agent/CHANGELOG.md", refused or "")
        self.assertIsNone(ci.changelog_refusal("pull_request", ["python/docx-agent/CHANGELOG.md"],
                                               "release/v2026.11.22"))
        self.assertIsNone(ci.changelog_refusal("pull_request", ["python/docx-agent/changes/x.fixed.md"], "lane-x"))
        self.assertIsNone(ci.changelog_refusal("push", ["python/docx-agent/CHANGELOG.md"], None))
        # Another file of that name is not a package's changelog.
        self.assertIsNone(ci.changelog_refusal("pull_request", ["crates/x/CHANGELOG.md"], None))

    def test_main_fails_the_plan(self) -> None:
        self.assertEqual(ci.main(["plan", "--event", "pull_request", "--changed", "python/pptx-agent/CHANGELOG.md",
                                  "--head-ref", "lane-x"]), 1)

    def test_fragments_are_notes(self) -> None:
        self.assertEqual(ci.affected(["python/docx-agent/changes/lane.added.md"]), [])
        self.assertEqual(ci.built(["python/docx-agent/changes/lane.added.md"]), ["docx-agent"])
        self.assertFalse(ci.module_input("python/ooxml-core/changes/lane.added.md"))
        self.assertEqual(ci.built(["tools/changelog.py"]), list(ci.PACKAGES))

    def test_release_notes_refuse_waiting_fragments(self) -> None:
        self.assertTrue(release_notes.TAG_PATTERN.fullmatch("v2026.11.22.1"))
        self.assertFalse(release_notes.TAG_PATTERN.fullmatch("check-only"))


if __name__ == "__main__":
    unittest.main()
