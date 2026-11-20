"""Tests of tools/gates.py: the known differences (every entry loads, none names its case by
number, an entry covers its case however the run numbers it, and a document condition holds by
the document, not by its name) and the modes ``auto`` picks from a branch's changes.  Standard
library only;
ci.yml's plan job runs them (`python3 -m unittest discover -s tools -p 'test_*.py'`)."""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gates  # noqa: E402
import gates_fixtures  # noqa: E402
import gates_hunks  # noqa: E402
import gates_jobs  # noqa: E402

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
        for path in ("crates/ooxml-word-edit/src/tables/merge.rs", "python/docx-agent/src/docx_agent/edit/tables.py"):
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
                     "crates/ooxml-word-edit/src/editor.rs", "python/docx-agent/src/docx_agent/layout.py"):
            self.assertEqual(gates.auto_fixtures([path]), gates_fixtures.ALL, path)
        self.assertEqual(gates.auto_fixtures([]), gates_fixtures.ALL)

    def test_a_changed_fixture_runs(self) -> None:
        name = "generated/mode14.docx"
        chosen = gates.auto_fixtures([f"python/docx-agent/tests/fixtures/{name}"])
        self.assertIn(name, chosen)
        self.assertEqual(gates.auto_fixtures(["python/docx-agent/tests/fixtures/generated/new-one.docx"]),
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


if __name__ == "__main__":
    unittest.main()
