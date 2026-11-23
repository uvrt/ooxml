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
        add(pkg, "lane-c.removed.md", "Gone.\n\nInstead: `other`.")
        add(pkg, "lane-d.removed.md", "Gone, nothing said.")
        found, wrong = changelog.fragments(pkg)
        self.assertEqual([f.path.name for f in found], ["lane-c.removed.md"])
        self.assertEqual(len(wrong), 4)
        self.assertTrue(any("lane-d.removed.md" in w and "Instead:" in w for w in wrong), wrong)
        self.assertEqual(len(changelog.problems(pkg)), 4)

    def test_release_into_a_new_section(self) -> None:
        pkg = make_package(self.py, "pkg", "1.3.0", HEAD + "## 1.2.0 -- unreleased\n\nOld text.\n")
        add(pkg, "zeta.fixed.md", "Fixed z.\n")
        add(pkg, "beta.added.md", "- Added b,\n  two lines.\n")
        add(pkg, "alpha.added.md", "Added a.\n")
        other = make_package(self.py, "other", "2.0.0", HEAD + "## 2.0.0\n")
        done = changelog.release("v2026.11.22", changelog.packages(self.py), {"pkg": "1.2.0"}, locks=[])
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
        # 1.2.0 was raised by hand past 1.1.0's next version (1.1.1), so it is kept.
        changelog.release("v2026.11.22", [pkg], {"pkg": "1.1.0"}, locks=[])
        self.assertEqual((pkg / "CHANGELOG.md").read_text(encoding="utf-8"), HEAD + (
            "## 1.2.0 -- unreleased\n\nEarlier text.\n\n### Changed\n\nChanged c.\n\n\n"
            "## 1.1.0 -- unreleased\n\nOlder.\n"))
        # At the end of the file too.
        pkg = make_package(self.py, "last", "1.0.0", HEAD + "## 1.0.0\n\nText.\n")
        add(pkg, "lane.removed.md", "Removed r. Instead: s.")
        changelog.release("v2026.11.22", [pkg], {"last": "0.9.0"}, locks=[])
        self.assertEqual((pkg / "CHANGELOG.md").read_text(encoding="utf-8"),
                         HEAD + "## 1.0.0\n\nText.\n\n### Removed\n\nRemoved r. Instead: s.\n")

    def test_a_wrong_fragment_writes_nothing(self) -> None:
        good = make_package(self.py, "good", "1.0.0", HEAD + "## 1.0.0\n")
        add(good, "lane.added.md", "Added.\n")
        bad = make_package(self.py, "bad", "1.0.0", HEAD + "## 1.0.0\n")
        add(bad, "lane.misc.md", "x")
        with self.assertRaises(ValueError):
            changelog.release("v2026.11.22", changelog.packages(self.py), locks=[])
        self.assertEqual((good / "CHANGELOG.md").read_text(encoding="utf-8"), HEAD + "## 1.0.0\n")
        self.assertTrue((good / changelog.CHANGES / "lane.added.md").exists())

    def test_next_version(self) -> None:
        self.assertEqual(changelog.next_version("0.1.140", {"fixed", "added", "changed"}), "0.1.141")
        self.assertEqual(changelog.next_version("0.1.140", {"fixed", "removed"}), "0.2.0")
        self.assertEqual(changelog.next_version("1.4.2", {"removed"}), "2.0.0")
        self.assertEqual(changelog.next_version("1.4.2", {"added"}), "1.4.3")
        self.assertEqual(changelog.next_version("0.3", {"fixed"}), "0.3.1")

    def test_a_pull_request_needs_no_raised_version(self) -> None:
        pkg = make_package(self.py, "pkg", "0.4.0", HEAD + "## 0.4.0 -- v2026.11.22\n\nx\n")
        add(pkg, "lane-a.fixed.md", "Fixed.\n")
        self.assertEqual(changelog.problems(pkg), [])

    def test_release_sets_versions_and_floors(self) -> None:
        low = make_package(self.py, "low", "0.4.0", HEAD + "## 0.4.0\n")
        add(low, "a.removed.md", "Gone. Instead: x.\n")
        mid = make_package(self.py, "mid", "1.0.0", HEAD + "## 1.0.0\n")
        add(mid, "b.fixed.md", "Fixed.\n")
        (mid / "pyproject.toml").write_text(
            '[project]\nname = "mid"\nversion = "1.0.0"\ndependencies = ["low>=0.3.0", "lowly>=0.1", "x>=1"]\n'
            '[project.optional-dependencies]\npng = ["low[png]>=0.3.5"]\ndev = ["low"]\n', encoding="utf-8")
        same = make_package(self.py, "same", "2.0.0", HEAD + "## 2.0.0\n")
        (same / "pyproject.toml").write_text('[project]\nname = "same"\nversion = "2.0.0"\n'
                                             'dependencies = ["low>=0.3.0"]\n', encoding="utf-8")
        changelog.release("v2026.11.24", changelog.packages(self.py), locks=[])
        self.assertEqual(changelog.project_version(low), "0.5.0")
        self.assertIn('__version__ = "0.5.0"', (low / "src/low/__init__.py").read_text(encoding="utf-8"))
        self.assertIn('version = "9.9"', (low / "pyproject.toml").read_text(encoding="utf-8"))
        self.assertTrue((low / "CHANGELOG.md").read_text(encoding="utf-8").startswith(HEAD + "## 0.5.0 -- v2026.11.24"))
        text = (mid / "pyproject.toml").read_text(encoding="utf-8")
        self.assertIn('version = "1.0.1"', text)
        self.assertIn('["low>=0.5.0", "lowly>=0.1", "x>=1"]', text)
        self.assertIn('["low[png]>=0.5.0"]', text)
        self.assertIn('["low"]', text)
        # A package with nothing to release keeps its version and its floors.
        self.assertIn('["low>=0.3.0"]', (same / "pyproject.toml").read_text(encoding="utf-8"))
        self.assertEqual(changelog.project_version(same), "2.0.0")
        for folder in (low, mid, same):
            self.assertEqual(changelog.problems(folder), [], folder.name)

    def test_release_versions_from_the_last_release(self) -> None:
        pkg = make_package(self.py, "pkg", "0.1.25", HEAD + "## 0.1.24\n")
        add(pkg, "a.fixed.md", "Fixed.\n")
        # Raised by hand since 0.1.24 (before versions were set at release): kept.
        self.assertEqual(changelog.release_versions([pkg], {"pkg": "0.1.24"}), {pkg: ("0.1.25", "0.1.25")})
        self.assertEqual(changelog.release_versions([pkg]), {pkg: ("0.1.25", "0.1.26")})
        add(pkg, "b.removed.md", "Gone. Instead: c.\n")
        self.assertEqual(changelog.release_versions([pkg], {"pkg": "0.1.24"}), {pkg: ("0.1.25", "0.2.0")})

    def test_a_crates_version_and_its_lock(self) -> None:
        folder = self.py / "core"
        (folder / "crate").mkdir(parents=True)
        (folder / "pyproject.toml").write_text('[project]\nname = "core"\ndynamic = ["version"]\n\n'
                                               '[tool.maturin]\nmanifest-path = "crate/Cargo.toml"\n', encoding="utf-8")
        (folder / "crate" / "Cargo.toml").write_text('[package]\nname = "core-py"\nversion = "0.10.0"\n\n'
                                                     '[dependencies]\nx = { version = "0.10.0" }\n', encoding="utf-8")
        (folder / "CHANGELOG.md").write_text(HEAD + "## 0.10.0\n", encoding="utf-8")
        lock = self.py / "Cargo.lock"
        lock.write_text('[[package]]\nname = "core"\nversion = "0.10.0"\n\n'
                        '[[package]]\nname = "core-py"\nversion = "0.10.0"\n', encoding="utf-8")
        add(folder, "a.added.md", "New.\n")
        changelog.release("v2026.11.24", [folder], locks=[lock])
        self.assertEqual(changelog.project_version(folder), "0.10.1")
        self.assertIn('x = { version = "0.10.0" }', (folder / "crate" / "Cargo.toml").read_text(encoding="utf-8"))
        self.assertEqual(lock.read_text(encoding="utf-8"), '[[package]]\nname = "core"\nversion = "0.10.0"\n\n'
                         '[[package]]\nname = "core-py"\nversion = "0.10.1"\n')

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


class ReleaseNotesTest(unittest.TestCase):
    """tools/release_notes.py's sections above the versions table."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.py = Path(self.tmp.name)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_removed_entries(self) -> None:
        text = HEAD + ("## 0.3.0 -- v2026.11.24\n\n### Changed\n\nC.\n\n### Removed\n\nTool x is gone.\n"
                       "Instead: y.\n\n## 0.2.0 -- v2026.11.23\n\n### Removed\n\nOlder.\n")
        pkg = make_package(self.py, "pkg", "0.3.0", text)
        add(pkg, "w.removed.md", "Waiting z. Instead: q.\n")
        add(pkg, "w.fixed.md", "Fixed.\n")
        self.assertEqual(release_notes.removed_entries("v2026.11.24", [pkg]),
                         [("pkg", "Tool x is gone.\nInstead: y."), ("pkg", "Waiting z. Instead: q.")])
        self.assertEqual(release_notes.removed_entries("v2026.11.25", [pkg]), [("pkg", "Waiting z. Instead: q.")])

    def test_spec_changes(self) -> None:
        before = {"a": {"x", "y"}, "b": {"x"}, "gone": set()}
        after = {"a": {"x", "z"}, "b": {"x"}, "new": {"p"}}
        self.assertEqual(release_notes.spec_changes(before, after), [
            "- `gone`: withdrawn", "- `a`: argument `y` removed", "- `a`: argument `z` added", "- `new`: new tool"])
        self.assertEqual(release_notes.spec_changes(after, after), [])

    def test_refused_forms(self) -> None:
        path = self.py / "known.toml"
        path.write_text('[[not_supported]]\nform = "word_x *"\ncause = """x\'s forms not ported: a, b \\\n'
                        '(deferred, lane Q)"""\n\n[[difference]]\nmode = "m"\n', encoding="utf-8")
        self.assertEqual(release_notes.refused(path), [("word_x *", "x's forms not ported: a, b")])
        forms = release_notes.refused(release_notes.ROOT / release_notes.KNOWN)
        self.assertTrue(forms and all(form and cause for form, cause in forms))

    def test_the_specs_are_read_at_a_commit(self) -> None:
        specs = release_notes.specs_at("HEAD", release_notes.SPECS["docx-agent"])
        self.assertIn("describe", specs)
        self.assertIn("word_read", specs)
        self.assertIn("doc", specs["word_read"])
        # a Claude definition's input_schema too
        self.assertIn("doc", release_notes.specs_at("HEAD", release_notes.SPECS["pptx-agent"])["check"])


if __name__ == "__main__":
    unittest.main()
