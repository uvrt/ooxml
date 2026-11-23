"""Tests of tools/gates.py: the known differences (every entry loads, none names its case by
number, an entry covers its case however the run numbers it, and a document condition holds by
the document, not by its name) and the modes ``auto`` picks from a branch's changes.  Standard
library only;
ci.yml's plan job runs them (`python3 -m unittest discover -s tools -p 'test_*.py'`)."""

from __future__ import annotations

import argparse
import contextlib
import gzip
import io
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import MappingProxyType
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gates  # noqa: E402
import gates_fixtures  # noqa: E402
import gates_hunks  # noqa: E402
import gates_jobs  # noqa: E402
import gates_store  # noqa: E402

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
            path.write_text('[[difference]]\nmode = "structure"\ncase = 3\nword = "n/a"\ncause = "x"\n')
            with self.assertRaises(SystemExit):
                gates.known_entries(path)

    def test_every_entry_has_a_word_verdict(self) -> None:
        verdicts = {entry["word"] for entry in gates.known_entries()}
        self.assertLessEqual(verdicts, set(gates.VERDICTS))

    def test_a_verdict_is_required_and_checked(self) -> None:
        observation = "crates/ooxml-oracle/word/observations/e4-word.json#fields"
        refused = ['', 'word = "python"\n', 'word = "rust"\n', 'word = "rust"\nobservation = "no/such.json#x"\n',
                   f'word = "n/a"\nobservation = "{observation}"\n']
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "known.toml"
            for verdict in refused:
                path.write_text(f'[[difference]]\nmode = "structure"\n{verdict}cause = "x"\n')
                with self.assertRaises(SystemExit, msg=verdict):
                    gates.known_entries(path)
            for verdict in ('word = "n/a"\n', 'word = "unmeasured"\n',
                            f'word = "rust"\nobservation = "{observation}"\n'):
                path.write_text(f'[[difference]]\nmode = "structure"\n{verdict}cause = "x"\n')
                self.assertEqual(len(gates.known_entries(path)), 1)

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
                path.write_text(f'[[difference]]\nmode = "objects"\ndocument = {condition}\nword = "n/a"\ncause = "x"\n')
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

    def test_the_frozen_reference_packages_run_nothing(self) -> None:
        # the reference is installed from crates/ooxml-oracle/reference.toml's commit
        self.assertEqual(self.modes("python/docx-agent/src/docx_agent/rust/edits.py",
                                    "python/ooxml-edit/src/ooxml_edit/tools/served.py",
                                    "python/docx-agent/pyproject.toml"), (set(), False))

    def test_the_reads_case_makers_run_the_reads(self) -> None:
        for name in gates.READ_CASE_MAKERS:
            self.assertEqual(self.modes(f"crates/ooxml-oracle/python/{name}"), (set(), True), name)
            self.assertTrue((gates.HARNESS / name).is_file(), name)

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
                     "crates/ooxml-oracle/src/edits.rs", "crates/ooxml-oracle/fixtures/generated/x.docx",
                     "crates/ooxml-oracle/corpus/make_keep_probe/keep-1.docx",
                     "crates/ooxml-oracle/reference.toml", "crates/ooxml-oracle/python/reference_packages.py",
                     "crates/ooxml-oracle/python/word_edits_diff.py", "crates/ooxml-oracle/known-differences.toml",
                     "crates/ooxml-oracle/python/reference_guard.py",
                     "crates/ooxml-oracle/python/no_core/ooxml_core/__init__.py",
                     "tools/gates.py", ".github/workflows/gates.yml", "Cargo.lock", "a/new/place.txt"):
            self.assertEqual(self.modes(path), (set(gates.EDIT_MODES), True), path)

    def test_the_checkouts_python_runs_in_no_gate(self) -> None:
        # the reference is installed from its frozen commit, and the corpus is frozen
        for path in ("python/docx2svg/src/docx2svg/layout.py", "python/docx2svg/tools/make_lists.py",
                     "python/ooxml-common/src/ooxml_common/fonts/office.py", "python/docx2svg/pyproject.toml",
                     "python/docx-agent/src/docx_agent/document.py"):
            self.assertEqual(self.modes(path), (set(), False), path)

    def test_the_corpus_is_frozen_in_the_checkout(self) -> None:
        found = gates.documents(gates.DEFAULT_TREES["corpus"], 1, (1, 1))
        self.assertEqual(len(found), 240)
        self.assertTrue(all(f.is_relative_to(gates.CORPUS) for f in found))

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
        self.assertEqual(set(gates.READ_MODES.values()) | {"word-edits"}, set(gates.GATE_BINARIES))

    def test_the_case_makers_are_not_keyed(self) -> None:
        # reference_cache.py leaves them out of the reference's key (only stdlib at import)
        sys.path.insert(0, str(gates.HARNESS))
        import reference_cache

        self.assertTrue(set(gates.CASE_MAKERS) <= set(reference_cache.NOT_REFERENCE))


class AutoFixtures(unittest.TestCase):
    """--fixtures auto: the fixtures a branch's changes can affect, and the canaries."""

    def test_the_feature_map_is_current(self) -> None:
        # after adding or changing a fixture: python tools/gates_fixtures.py write
        self.assertEqual(gates_fixtures.scan(), gates_fixtures.load())

    def test_the_canaries_are_fixtures(self) -> None:
        known = gates_fixtures.load()
        for name in gates_fixtures.CANARIES:
            self.assertIn(name, known)

    def test_a_tables_change_runs_the_table_fixtures(self) -> None:
        known = gates_fixtures.load()
        tables = {name for name, held in known.items() if "tables" in held}
        for path in ("crates/ooxml-word-edit/src/tables/merge.rs", "crates/ooxml-tools-word/src/tables/edit.rs"):
            chosen = gates.auto_fixtures([path])
            self.assertEqual(set(chosen), tables | set(gates_fixtures.CANARIES), path)
            self.assertLess(len(chosen), len(known))

    def test_a_notes_change_leaves_the_others_out(self) -> None:
        chosen = gates.auto_fixtures(["crates/ooxml-tools-word/src/structure/notes.rs"])
        self.assertIn("generated/own-tracked-note.docx", chosen)
        self.assertNotIn("wordto/sample-10pages.docx", chosen)

    def test_a_foundation_or_an_unnamed_path_runs_every_fixture(self) -> None:
        for path in ("crates/ooxml-xml/src/writer.rs", "crates/ooxml-opc/src/package.rs",
                     "crates/ooxml-store/src/lib.rs", "crates/ooxml-session/src/lib.rs",
                     "crates/ooxml-oracle/python/word_edits_diff.py", "crates/ooxml-oracle/known-differences.toml",
                     "crates/ooxml-word-edit/src/editor.rs"):
            self.assertEqual(gates.auto_fixtures([path]), gates_fixtures.ALL, path)
        self.assertEqual(gates.auto_fixtures([]), gates_fixtures.ALL)

    def test_a_changed_fixture_runs(self) -> None:
        name = "generated/mode14.docx"
        chosen = gates.auto_fixtures([f"crates/ooxml-oracle/fixtures/{name}"])
        self.assertIn(name, chosen)
        self.assertEqual(gates.auto_fixtures(["crates/ooxml-oracle/fixtures/generated/new-one.docx"]),
                         gates_fixtures.ALL)

    def test_paths_the_gates_do_not_run_add_nothing(self) -> None:
        chosen = gates.auto_fixtures(["crates/ooxml-word-edit/src/tables/merge.rs", "README.md", "tools/ci.py"])
        self.assertEqual(chosen, gates.auto_fixtures(["crates/ooxml-word-edit/src/tables/merge.rs"]))

    def test_documents_keep_the_selection(self) -> None:
        only = list(gates_fixtures.CANARIES)
        files = gates.documents(gates.FIXTURES, 1, (1, 1), only)
        self.assertEqual(sorted(str(f.relative_to(gates.FIXTURES)) for f in files), sorted(only))


class ChangedLines(unittest.TestCase):
    """--fixtures auto reads the changed lines of the files every branch touches
    (tools/gates_hunks.py), in a scratch repository: base and head commits of a few files."""

    def select(self, files: dict[str, tuple[str, str]]) -> list[str] | str:
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)

            def git(*args: str) -> str:
                return subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args], cwd=root,
                                      capture_output=True, text=True, check=True).stdout.strip()

            git("init", "-q")
            for side in (0, 1):
                for path, texts in files.items():
                    (root / path).parent.mkdir(parents=True, exist_ok=True)
                    (root / path).write_text(texts[side])
                git("add", "-A")
                git("commit", "-q", "-m", str(side))
            diff = gates_hunks.Diff("HEAD~1", "HEAD", root=root)
            return gates_fixtures.select(list(files), diff=diff)

    def features(self, *names: str) -> set[str]:
        return {n for n, held in gates_fixtures.load().items() if set(names) & set(held)} | set(gates_fixtures.CANARIES)

    def test_a_use_line_in_lib_rs_adds_nothing(self) -> None:
        lib = "crates/ooxml-word-edit/src/lib.rs"
        base = "mod a;\npub use a::{B, C};\n\npub fn f() -> u8 {\n    1\n}\n"
        self.assertEqual(set(self.select({lib: (base, base.replace("{B, C}", "{B, C, D}").replace("mod a;", "mod a;\nmod e;"))})),
                         set(gates_fixtures.CANARIES))
        self.assertEqual(self.select({lib: (base, base.replace("    1", "    2"))}), gates_fixtures.ALL)

    def test_a_comment_adds_nothing(self) -> None:
        editor = "crates/ooxml-word-edit/src/editor.rs"
        base = "// one\npub fn f() -> u8 {\n    1\n}\n"
        self.assertEqual(set(self.select({editor: (base, base.replace("// one", "// two"))})),
                         set(gates_fixtures.CANARIES))

    def test_a_registration_runs_its_tool(self) -> None:
        registry = "crates/ooxml-session/src/registry.rs"
        base = ("fn r() {\n        registry.register(word_edit::<ooxml_tools_word::WordDelete>(ooxml_tools_word::word_delete));\n"
                "}\n")
        head = base.replace("}\n", "        registry.register(word_edit::<ooxml_tools_word::WordComments>(\n"
                                   "            ooxml_tools_word::word_comments,\n        ));\n}\n")
        self.assertEqual(set(self.select({registry: (base, head)})), self.features("comments"))
        self.assertEqual(self.select({registry: (base, base.replace("fn r() {", "fn r() {\n    let x = 1;"))}),
                         gates_fixtures.ALL)

    def test_a_known_difference_runs_its_fixture(self) -> None:
        known = "crates/ooxml-oracle/known-differences.toml"
        entry = '[[difference]]\nmode = "structure"\nfile = "generated/mode14.docx"\ncause = "x"\n'
        self.assertEqual(set(self.select({known: ("", entry)})),
                         set(gates_fixtures.CANARIES) | {"generated/mode14.docx"})
        self.assertEqual(self.select({known: ("", entry.replace('file = "generated/mode14.docx"\n', ""))}),
                         gates_fixtures.ALL)
        self.assertEqual(set(self.select({known: ("", entry.replace('mode =', 'gate = "corpus"\nmode ='))})),
                         set(gates_fixtures.CANARIES))

    def test_a_case_maker_runs_the_tools_whose_cases_changed(self) -> None:
        cases = "crates/ooxml-oracle/python/word_structure_cases.py"
        base = ('@CASES\ndef note_cases(document):\n    out = [call("word_notes", action="insert")]\n    return out\n\n\n'
                "def _words(document):\n    return []\n")
        self.assertEqual(set(self.select({cases: (base, base.replace('"insert"', '"delete"'))})),
                         self.features("notes"))
        self.assertEqual(self.select({cases: (base, base.replace("return []", "return [1]"))}), gates_fixtures.ALL)

    def test_a_served_entry_runs_its_tool(self) -> None:
        served = "python/docx-agent/src/docx_agent/rust/edits.py"
        base = 'SERVED = {\n    "word_delete": _delete,\n}\n'
        head = base.replace("}", '    "word_format_table": _format_table,\n}')
        self.assertEqual(set(self.select({served: (base, head)})), self.features("tables", "styles"))
        self.assertEqual(self.select({served: (base, head.replace("SERVED = {", "X = 2\nSERVED = {"))}),
                         gates_fixtures.ALL)


class JobPlan(unittest.TestCase):
    """gates.py matrix: shards and bundles (tools/gates_jobs.py)."""

    SMALL = {"set_text": 60.0, "ops": 70.0, "cells": 5.0}

    def jobs(self, modes, estimates, *, gate="fixtures", warm=True, shards=None) -> list[tuple[str, str]]:
        warm = modes if warm is True else warm or []
        return [(j["modes"], j["shard"]) for j in gates_jobs.gate_jobs(gate, modes, warm=warm, shards=shards,
                                                                        estimates=estimates)]

    def test_cold_is_three_shards_a_mode(self) -> None:
        self.assertEqual(self.jobs(["set_text", "structure"], self.SMALL, warm=False),
                         [("set_text", "1/3"), ("set_text", "2/3"), ("set_text", "3/3"),
                          ("structure", "1/3"), ("structure", "2/3"), ("structure", "3/3")])

    def test_warm_bundles_the_small_modes(self) -> None:
        estimates = {**self.SMALL, "structure": 600.0}
        self.assertEqual(self.jobs(["set_text", "structure", "ops", "cells"], estimates),
                         [("set_text,ops,cells", "1/1"), ("structure", "1/1")])

    def test_a_slow_warm_mode_gets_shards(self) -> None:
        slow = 2.5 * gates_jobs.SHARD_TARGET
        self.assertEqual(self.jobs(["objects"], {"objects": slow}), [("objects", "1/3"), ("objects", "2/3"),
                                                                      ("objects", "3/3")])

    def test_a_mode_with_no_time_runs_alone(self) -> None:
        self.assertEqual(self.jobs(["set_text", "review"], self.SMALL), [("set_text", "1/1"), ("review", "1/1")])

    def test_a_full_bundle_starts_another(self) -> None:
        under = gates_jobs.BUNDLE_UNDER - 1
        modes = [f"m{i}" for i in range(gates_jobs.BUNDLE_LIMIT // under + 1)]
        jobs = self.jobs(modes, dict.fromkeys(modes, float(under)))
        self.assertEqual(len(jobs), 2)
        self.assertEqual(sorted(m for modes, _ in jobs for m in modes.split(",")), sorted(modes))

    def test_explicit_shards_win(self) -> None:
        self.assertEqual(self.jobs(["set_text", "ops"], self.SMALL, shards=2),
                         [("set_text", "1/2"), ("set_text", "2/2"), ("ops", "1/2"), ("ops", "2/2")])

    def test_the_reads_never_wait_for_the_cache(self) -> None:
        self.assertEqual(self.jobs(["ids", "find"], {"ids": 5.0, "find": 11.0}, gate="reads", warm=False),
                         [("ids,find", "1/1")])

    def test_a_time_scales_with_the_fixtures_picked(self) -> None:
        durations = {"fixtures": {"structure": {"seconds": 280.0, "documents": 28}}, "corpus": {"text": {"seconds": 9}}}
        self.assertEqual(gates_jobs.estimate(durations, "fixtures", "structure", 7), 70.0)
        self.assertEqual(gates_jobs.estimate(durations, "corpus", "text", None), 9.0)
        self.assertIsNone(gates_jobs.estimate(durations, "corpus", "style", None))

    def test_the_recorded_times_name_real_modes(self) -> None:
        durations = gates_jobs.load_durations()
        self.assertEqual(set(durations), {"fixtures", "corpus", "reads"})
        for gate, modes in durations.items():
            self.assertLessEqual(set(modes), set(gates.READ_MODES if gate == "reads" else gates.EDIT_MODES), gate)
            self.assertTrue(all(m["seconds"] >= 0 for m in modes.values()))

    def test_matrix_labels_and_names_are_unique(self) -> None:
        import contextlib
        import io
        import json

        for warm in ([], ["--warm"]):
            out = io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
                gates.main(["matrix", "--gates", "fixtures,corpus,reads", "--modes", "all", *warm])
            jobs = json.loads(out.getvalue())["include"]
            names = [(j["gate"], j["label"], j["shard"]) for j in jobs]
            self.assertEqual(len(names), len(set(names)))
            ran = sorted((j["gate"], m) for j in jobs for m in j["modes"].split(",") if j["shard"].startswith("1/"))
            self.assertEqual(ran, sorted([*((g, m) for g in ("fixtures", "corpus") for m in gates.EDIT_MODES),
                                          *(("reads", m) for m in gates.READ_MODES)]))
            self.assertTrue(all(j["per_tool"] == ("10" if j["gate"] == "corpus" else "0") for j in jobs))
            if warm:
                self.assertLess(len(jobs), 30)

    def summary(self, mode="set_text", shard="1/1", **over) -> dict:
        return {"gate": "fixtures", "mode": mode, "shard": shard, "every": 1, "per_tool": 0, "documents": 14,
                "cases": 100, "computed": 2, "kept": 98, "wall": 30.0, "crashed": False, **over}

    def test_warm_times_add_the_shards_up(self) -> None:
        found = gates_jobs.warm_durations([self.summary(shard="1/2"), self.summary(shard="2/2", wall=20.0)],
                                          {"fixtures": 0})
        self.assertEqual(found, {"fixtures": {"set_text": {"seconds": 50.0, "documents": 28}}})

    def test_cold_sampled_or_partial_runs_record_nothing(self) -> None:
        for summaries in ([self.summary(computed=50)], [self.summary(every=2)], [self.summary(per_tool=3)],
                          [self.summary(shard="1/2")], [self.summary(crashed=True)]):
            self.assertEqual(gates_jobs.warm_durations(summaries, {"fixtures": 0}), {}, summaries)

    def test_new_times_replace_old_ones(self) -> None:
        old = {"fixtures": {"ops": {"seconds": 1}, "cells": {"seconds": 2}}}
        new = {"fixtures": {"ops": {"seconds": 3}}, "reads": {"ids": {"seconds": 4}}}
        self.assertEqual(gates_jobs.merged(old, new), {"fixtures": {"cells": {"seconds": 2}, "ops": {"seconds": 3}},
                                                       "reads": {"ids": {"seconds": 4}}})


class HarnessCommand(unittest.TestCase):
    """gates.py diff: the harness command line each mode's harness parses."""

    def args(self, mode: str):
        import argparse

        return argparse.Namespace(mode=mode, bin="/bin", cache=None, per_tool=0)

    @staticmethod
    def harness_parser(name: str):
        """The harness's own argument parser, rebuilt from its main()'s add_argument calls
        (their literal arguments): importing the harness needs docx-agent, and these tests
        run on the standard library alone."""
        import argparse
        import ast

        tree = ast.parse((gates.HARNESS / name).read_text())
        main = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "main")
        parser = argparse.ArgumentParser(exit_on_error=False)
        for call in ast.walk(main):
            if isinstance(call, ast.Call) and getattr(call.func, "attr", "") == "add_argument":
                parser.add_argument(*(ast.literal_eval(a) for a in call.args),
                                    **{k.arg: ast.literal_eval(k.value) for k in call.keywords
                                       if k.arg not in ("type", "action") or isinstance(k.value, ast.Constant)})
        return parser

    def test_the_reads_harness_takes_its_files(self) -> None:
        files = [Path("a.docx"), Path("b/c.docx")]
        command, _ = gates.harness_command(self.args("ids"), Path("out"), Path("out/results.jsonl"), files)
        self.assertEqual(command[2:5], ["ids", "a.docx", "b/c.docx"])    # right after the check
        parsed = self.harness_parser("word_reads_diff.py").parse_args(command[2:])
        self.assertEqual((parsed.what, parsed.files, parsed.results), ("ids", ["a.docx", "b/c.docx"],
                                                                      "out/results.jsonl"))

    def test_the_edits_harness_takes_its_files_last(self) -> None:
        command, env = gates.harness_command(self.args("styled"), Path("out"), Path("r.jsonl"), [Path("a.docx")])
        self.assertEqual(command[2:], ["--styled", "--rust", "/bin/word-edits", "--results", "r.jsonl", "a.docx"])
        self.assertEqual(env, {})

    def test_the_edits_harness_takes_the_cache_and_writes_the_case_keys(self) -> None:
        args = self.args("structure")
        args.cache = "store"
        command, _ = gates.harness_command(args, Path("out"), Path("r.jsonl"), [Path("a.docx")])
        parsed = self.harness_parser("word_edits_diff.py").parse_args(command[2:])
        self.assertEqual((parsed.cache, parsed.cache_new, parsed.cache_used),
                         ("store", str(Path("out/reference-new.jsonl.gz")), str(Path("out") / gates.USED)))


class StoreCoverage(unittest.TestCase):
    """The store's coverage per mode (tools/gates_store.py): the plan runs warm only the modes it
    covers, and a merge keeps every mode the store held."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)

    def results(self, name: str, entries: dict) -> Path:
        path = self.tmp / name
        gates_store.write_results(path, entries)
        return path

    def shard(self, name: str, gate: str, mode: str, documents: dict, tree=None, per_tool=0) -> Path:
        path = self.tmp / name / gates_store.COVERAGE_NEW
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(gates_store.coverage(gate, mode, per_tool, documents, tree)))
        return path

    def stored(self, store: Path) -> dict:
        return dict(gates_store.read_results(store / gates_store.MERGED))

    def matrix(self, store: Path | None, modes: str, gates_: str = "fixtures", warm: bool = False) -> list[dict]:
        out = io.StringIO()
        args = (["--store", str(store)] if store else []) + (["--warm"] if warm else [])
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
            gates.main(["matrix", "--gates", gates_, "--modes", modes, "--fixtures", "all", *args])
        return json.loads(out.getvalue())["include"]

    def shards(self, jobs: list[dict]) -> dict[str, int]:
        return {mode: int(j["shard"].split("/")[1]) for j in jobs for mode in j["modes"].split(",")}

    def test_a_merge_keeps_the_modes_the_run_did_not_run(self) -> None:
        store = self.tmp / "store"
        gates_store.merge(store, [self.results("a.jsonl.gz", {"k1": {"v": 1}})],
                          [self.shard("a", "fixtures", "objects", {"x.docx": ["k1"]})])
        n = gates_store.merge(store, [self.results("b.jsonl.gz", {"k2": {"v": 2}})],
                              [self.shard("b", "fixtures", "structure", {"x.docx": ["k2"]})])
        self.assertEqual(n, {"added": 1, "dropped": 0, "recorded": 1})
        self.assertEqual(self.stored(store), {"k1": {"v": 1}, "k2": {"v": 2}})
        covered = gates_store.load(store)
        self.assertEqual(set(covered["modes"]), {"fixtures/objects", "fixtures/structure"})
        self.assertEqual(gates_store.warm_modes(covered, "fixtures", ["objects", "structure", "style"], 0,
                                                ["x.docx"]), {"objects", "structure"})

    def test_a_case_no_longer_made_is_dropped(self) -> None:
        store = self.tmp / "store"
        gates_store.merge(store, [self.results("a.jsonl.gz", {"k1": {}, "k2": {}, "k3": {}})],
                          [self.shard("a", "fixtures", "objects", {"x.docx": ["k1", "k2"]}),
                           self.shard("a2", "fixtures", "style", {"x.docx": ["k2"]})])
        n = gates_store.merge(store, [], [self.shard("b", "fixtures", "objects", {"x.docx": ["k1"]})])
        # k2 is still the style mode's; k3, kept but never listed (a crashed shard's), stays
        self.assertEqual((n["dropped"], set(self.stored(store))), (0, {"k1", "k2", "k3"}))
        n = gates_store.merge(store, [], [self.shard("c", "fixtures", "style", {"x.docx": []})])
        self.assertEqual((n["dropped"], set(self.stored(store))), (1, {"k1", "k3"}))

    def test_a_store_of_some_modes_plans_the_others_cold(self) -> None:
        store = self.tmp / "store"
        fixtures = [str(f.relative_to(gates.FIXTURES)) for f in gates.documents_of(gates.FIXTURES, None)]
        gates_store.merge(store, [], [self.shard(mode, "fixtures", mode, {name: [] for name in fixtures})
                                      for mode in ("objects", "style")])
        found = self.shards(self.matrix(store, "objects,style,structure"))
        self.assertEqual(found["structure"], gates_jobs.COLD_SHARDS)
        warm = self.shards(self.matrix(None, "objects,style", warm=True))   # as their recorded warm times size them
        self.assertEqual((found["objects"], found["style"]), (warm["objects"], warm["style"]))
        # a fixture the store has not covered: the mode is cold again
        covered = gates_store.load(store)
        del covered["modes"]["fixtures/objects"]["per_tool=0"][fixtures[0]]
        with gzip.open(store / gates_store.COVERAGE, "wt") as handle:
            json.dump(covered, handle)
        found = self.shards(self.matrix(store, "objects,style"))
        self.assertEqual((found["objects"], found["style"]), (gates_jobs.COLD_SHARDS, warm["style"]))

    def test_no_store_or_no_coverage_is_cold(self) -> None:
        self.assertEqual(set(self.shards(self.matrix(None, "objects,style")).values()), {gates_jobs.COLD_SHARDS})
        legacy = self.tmp / "legacy"
        self.results("legacy/reference.jsonl.gz", {"k": {}})
        self.assertEqual(set(self.shards(self.matrix(legacy, "objects")).values()), {gates_jobs.COLD_SHARDS})

    def diff(self, code: int) -> Path:
        """A corpus shard of one document whose harness exits with ``code``: its out folder."""
        tree, out = self.tmp / "corpus", self.tmp / f"out{code}"
        (tree / "d").mkdir(parents=True, exist_ok=True)
        for name in ("a.docx", "d/b.docx"):
            (tree / name).write_bytes(b"")

        def command(args, out_dir, results, files):
            used = json.dumps({str(tree / "d" / "b.docx"): ["k1"]})
            totals = json.dumps({"totals": {"cases": 1}})
            write = (f"import pathlib, sys; pathlib.Path({str(results)!r}).write_text({totals!r}); "
                     f"pathlib.Path({str(out_dir / gates.USED)!r}).write_text({used!r}); sys.exit({code})")
            return [sys.executable, "-c", write], {}

        args = argparse.Namespace(record=False, reference=False, out=str(out), tree=str(tree), gate="corpus", mode="structure", shard="2/2",
                                  every=1, per_tool=10, fixtures=None, base=None, cache="store")
        with mock.patch.object(gates, "harness_command", command), contextlib.redirect_stdout(io.StringIO()):
            gates.run_diff(args)
        return out

    def test_a_finished_shard_writes_its_coverage_and_a_crashed_one_none(self) -> None:
        shard = json.loads((self.diff(0) / gates_store.COVERAGE_NEW).read_text())
        self.assertEqual(shard, {"gate": "corpus", "mode": "structure", "per_tool": "per_tool=10",
                                 "documents": {"d/b.docx": ["k1"]}, "tree": ["a.docx", "d/b.docx"]})
        self.assertFalse((self.diff(3) / gates_store.COVERAGE_NEW).exists())

    def test_the_corpus_plans_from_the_tree_its_last_run_found(self) -> None:
        store = self.tmp / "store"
        tree = ["a.docx", "b.docx"]
        gates_store.merge(store, [], [self.shard("a", "corpus", "structure", {"a.docx": []}, tree, per_tool=10),
                                      self.shard("b", "corpus", "structure", {"b.docx": []}, tree, per_tool=10),
                                      self.shard("c", "corpus", "objects", {"a.docx": []}, tree, per_tool=10)])
        found = self.shards(self.matrix(store, "structure,objects", "corpus"))
        self.assertEqual((found["structure"], found["objects"]), (1, gates_jobs.COLD_SHARDS))


class Panics(unittest.TestCase):
    """A shard fails on any case where Rust panicked, and on a form not supported yet that the
    allow-list does not name; the allowed forms are reported."""

    def diff(self, totals: dict) -> tuple[int, dict, str]:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "out"
            line = json.dumps({"totals": totals})

            def command(args, out_dir, results, files):
                write = f"import pathlib; pathlib.Path({str(results)!r}).write_text({line!r})"
                return [sys.executable, "-c", write], {}

            args = argparse.Namespace(record=False, reference=False, out=str(out), tree=tmp, gate="corpus", mode="styled", shard="1/1",
                                      every=None, per_tool=None, fixtures=None, base=None, cache=None)
            printed = io.StringIO()
            with mock.patch.object(gates, "harness_command", command), \
                    mock.patch.object(gates, "documents", lambda *_: [Path(tmp) / "a.docx"]), \
                    contextlib.redirect_stdout(printed):
                code = gates.run_diff(args)
            return code, json.loads((out / "summary.json").read_text()), printed.getvalue()

    def test_a_panic_fails_the_shard(self) -> None:
        code, summary, printed = self.diff({"cases": 3, "failed": 0, "panics": 1, "not_supported_forms": {}})
        self.assertEqual((code, summary["panics"]), (1, 1))
        self.assertIn("Rust panicked", printed)

    def test_allowed_forms_are_reported_not_failing(self) -> None:
        forms = {"word_insert_markdown like": 2, "a picture in Markdown": 1}
        with mock.patch.object(gates, "allowed_forms", lambda: [{"form": "word_insert_markdown *", "cause": "x"},
                                                                {"form": "a picture in Markdown", "cause": "x"}]):
            code, summary, printed = self.diff({"cases": 3, "failed": 0, "panics": 0, "not_supported_forms": forms})
        self.assertEqual((code, summary["not_supported_forms"], summary["not_supported"]), (0, forms, 3))
        self.assertIn("not supported: 2 word_insert_markdown like", printed)

    def test_a_form_outside_the_allow_list_fails_the_shard(self) -> None:
        forms = {"word_insert_markdown like": 2, "a new form": 1}
        with mock.patch.object(gates, "allowed_forms", lambda: [{"form": "word_insert_markdown *", "cause": "x"}]):
            code, summary, printed = self.diff({"cases": 3, "failed": 0, "panics": 0, "not_supported_forms": forms})
        self.assertEqual((code, summary["unlisted_forms"]), (1, {"a new form": 1}))
        self.assertIn("'a new form'", printed)

    def test_the_allow_list_is_well_formed(self) -> None:
        entries = gates.allowed_forms()
        self.assertTrue(entries)
        self.assertTrue(all(e["form"] and e["cause"] for e in entries))

    def test_the_summary_adds_panics_and_forms_up(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            for k, (panics, forms) in enumerate([(1, {"like": 2}), (0, {"like": 1, "picture": 1})]):
                shard = Path(tmp) / f"diff-{k}"
                shard.mkdir()
                (shard / "summary.json").write_text(json.dumps({
                    "gate": "fixtures", "mode": "text", "documents": 1, "cases": 4, "differences": 0,
                    "known": 0, "not_supported": 3, "panics": panics, "not_supported_forms": forms, "wall": 1.0,
                    "crashed": False, "unknown": []}))
            printed = io.StringIO()
            with contextlib.redirect_stdout(printed):
                gates.run_summary(argparse.Namespace(dir=tmp))
        table = printed.getvalue()
        self.assertIn("| text | 2 | 2 | 8 | 0 | 0 | 6 | 1 | 1 |", table)
        self.assertIn("- text: 3 like", table)
        self.assertIn("- text: 1 picture", table)


class UnusedEntries(unittest.TestCase):
    """The summary lists the known differences no case of the run matched -- those it could
    have used (a mode it ran, in a gate the entry admits) -- and the forms not supported yet
    no refusal matched: the stale entries a full run shows."""

    ENTRIES = (
        {"mode": "text", "input": {"tool": "a"}, "cause": "used"},
        {"mode": "text", "gate": "fixtures", "file": "x.docx", "cause": "stale"},
        {"mode": "text", "gate": "corpus", "cause": "a gate the run did not run"},
        {"mode": "objects", "cause": "a mode the run did not run"},
    )
    ALLOWED = ({"form": "like *", "cause": "refused"}, {"form": "gone *", "cause": "never refused"})

    def shard(self, **over) -> dict:
        used = {gates.entry_key(self.ENTRIES[0]): 2}
        return {"gate": "fixtures", "mode": "text", "documents": 1, "cases": 4, "differences": 0, "known": 2,
                "not_supported": 1, "panics": 0, "not_supported_forms": {"like this": 1}, "wall": 1.0,
                "crashed": False, "unknown": [], "used": used, "whole": True, "every": 1, "per_tool": 0, **over}

    def test_an_entry_or_form_no_case_matched_is_listed(self) -> None:
        unused, idle = gates.unused_entries([self.shard(), self.shard(used={})], list(self.ENTRIES),
                                            list(self.ALLOWED))
        self.assertEqual([e["cause"] for e in unused], ["stale"])
        self.assertEqual([e["form"] for e in idle], ["gone *"])

    def test_the_summary_says_whether_the_list_is_a_hint(self) -> None:
        for over, said in (({}, "each entry below is stale"), ({"whole": False}, "a hint only"),
                           ({"per_tool": 3}, "a hint only")):
            with tempfile.TemporaryDirectory() as tmp:
                (Path(tmp) / "diff-0").mkdir()
                (Path(tmp) / "diff-0" / "summary.json").write_text(json.dumps(self.shard(**over)))
                printed = io.StringIO()
                with mock.patch.object(gates, "known_entries", lambda: list(self.ENTRIES)), \
                        mock.patch.object(gates, "allowed_forms", lambda: list(self.ALLOWED)), \
                        contextlib.redirect_stdout(printed):
                    gates.run_summary(argparse.Namespace(dir=tmp))
            table = printed.getvalue()
            self.assertIn("### Known differences no case matched", table)
            self.assertIn(said, table)
            self.assertIn('file "x.docx"', table)
            self.assertIn("- not supported: `gone *`", table)
            self.assertNotIn("used", table.split("no case matched")[1].split("not supported")[0])

    def test_a_shard_counts_the_entries_its_cases_matched(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "out"
            lines = [json.dumps({"file": str(Path(tmp) / "a.docx"), "case": 0, "input": {"tool": "a"}}),
                     json.dumps({"totals": {"cases": 1, "panics": 0, "not_supported_forms": {}}})]

            def command(args, out_dir, results, files):
                write = f"import pathlib; pathlib.Path({str(results)!r}).write_text({chr(10).join(lines)!r})"
                return [sys.executable, "-c", write], {}

            args = argparse.Namespace(record=False, reference=False, out=str(out), tree=tmp, gate="fixtures", mode="text",
                                      shard="1/1", every=1, per_tool=0, fixtures="all", base=None, cache=None)
            with mock.patch.object(gates, "harness_command", command), \
                    mock.patch.object(gates, "documents", lambda *_: [Path(tmp) / "a.docx"]), \
                    mock.patch.object(gates, "known_entries", lambda: list(self.ENTRIES)), \
                    contextlib.redirect_stdout(io.StringIO()):
                gates.run_diff(args)
            summary = json.loads((out / "summary.json").read_text())
        self.assertEqual(summary["used"], {gates.entry_key(self.ENTRIES[0]): 1})


class ReadsRefusals(unittest.TestCase):
    """The reads gate judges a case Rust refused as not supported yet by the allow-list, as
    the edit gates do: the harness's compared reply keeps the form (``details.not_supported``,
    where word_reads_diff.py's report looks for it), so the case is no difference."""

    REFUSED = MappingProxyType({
        "code": "refused", "message": "word_inspect of Accept All's cell ends is not supported yet",
        "details": {"not_supported": "word_inspect of Accept All's cell ends", "try": "x"}})

    @staticmethod
    def tools():
        # the harness's reply comparison: the standard library alone at import
        sys.path.insert(0, str(gates.HARNESS))
        import word_reads_tools

        return word_reads_tools

    def test_a_refusal_keeps_its_form(self) -> None:
        got = self.tools().comparable("word_inspect", {"error": dict(self.REFUSED)})
        self.assertEqual(got["error"]["details"], {"not_supported": "word_inspect of Accept All's cell ends"})

    def test_a_refusal_in_a_batch_keeps_its_form(self) -> None:
        reply = {"data": {"ops": [{"tool": "word_read", "doc": "d1", "error": dict(self.REFUSED)}]}}
        got = self.tools().comparable("read_batch", reply)
        self.assertEqual(got["data"]["ops"][0]["error"]["details"]["not_supported"],
                         "word_inspect of Accept All's cell ends")

    def test_another_error_is_its_code_and_message(self) -> None:
        error = {"code": "not_found", "message": "no paragraph or table 'p:1'", "details": {"field": "target"}}
        got = self.tools().comparable("word_inspect", {"error": error})
        self.assertEqual(got, {"error": {"code": "not_found", "message": "no paragraph or table 'p:1'"}})

    def test_the_reads_refusals_are_allowed(self) -> None:
        forms = {"word_read final view of Accept All's cell ends": 13, "word_inspect of Accept All's cell ends": 14,
                 "a theme whose colours docx-agent cannot read": 3}
        self.assertEqual(gates.unlisted_forms(forms, gates.allowed_forms()), {})

    def test_a_reads_refusal_not_on_the_list_is_a_difference(self) -> None:
        forms = {"word_inspect of Accept All's cell ends": 14, "word_inspect of a form no entry names": 2}
        self.assertEqual(gates.unlisted_forms(forms, gates.allowed_forms()),
                         {"word_inspect of a form no entry names": 2})


if __name__ == "__main__":
    unittest.main()
