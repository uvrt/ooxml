"""Tests of tools/ci.py plan: which legs each event runs.  Standard library only; ci.yml's
plan job runs them (`python3 -m unittest discover -s tools -p 'test_*.py'`)."""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ci  # noqa: E402


def _plan(event: str, changed: list[str] | None = None) -> dict:
    return {k: json.loads(v) for k, v in ci.plan(event, changed).items()}


def _legs(out: dict, os_: str) -> dict[str, list[str]]:
    """Each package's Pythons on ``os_`` (a sharded suite's Python once, not per shard)."""
    legs: dict[str, list[str]] = {}
    for leg in out[os_]["include"]:
        if leg.get("shard", 1) == 1:
            legs.setdefault(leg["package"], []).append(leg["python"])
    return legs


SMOKE = [name for name, spec in ci.PACKAGES.items() if spec.get("windows_smoke")]


class PlanTest(unittest.TestCase):
    def test_pull_request(self) -> None:
        out = _plan("pull_request", ["tools/ci.py"])
        self.assertEqual(out["packages"], list(ci.PACKAGES))
        for name in ci.PACKAGES:
            versions = ci._pythons(name)
            self.assertEqual(_legs(out, "linux")[name], ["3.15"])
            self.assertEqual(versions[-1], "3.15")
        # Windows: only the packages with code of their own there.
        self.assertEqual(_legs(out, "windows"), {name: ["3.15"] for name in SMOKE})
        self.assertEqual(SMOKE, ["ooxml-common", "ooxml-edit"])
        self.assertIs(out["any_windows"], True)
        self.assertEqual(out["macos"]["include"], [])
        self.assertIs(out["any_macos"], False)

    def test_pull_request_without_a_windows_package(self) -> None:
        out = _plan("pull_request", ["python/docx-agent/src/docx_agent/x.py"])
        self.assertEqual(out["windows"]["include"], [])
        self.assertIs(out["any_windows"], False)

    def test_pull_request_affected_only(self) -> None:
        out = _plan("pull_request", ["python/pptx-agent/src/pptx_agent/x.py"])
        # xlsx-agent's tests install pptx-agent, but a pull request follows its pr_uses.
        self.assertEqual(out["packages"], ["pptx-agent"])
        self.assertEqual(set(_legs(out, "linux")), {"pptx-agent"})
        self.assertIs(out["rust"], False)

    def test_xlsx_agent_on_a_pull_request(self) -> None:
        cases = {
            "python/xlsx-agent/src/xlsx_agent/x.py": True,
            "python/ooxml-common/src/ooxml_common/x.py": True,
            "python/ooxml-edit/src/ooxml_edit/x.py": True,
            "python/ooxml-core/pyproject.toml": True,
            "crates/python/src/lib.rs": True,
            "crates/ooxml-python-session/src/lib.rs": True,
            "Cargo.lock": True,
            "crates/ooxml-word-edit/src/lib.rs": False,
            "python/docx-agent/src/docx_agent/x.py": False,
            "python/pptx-agent/src/pptx_agent/x.py": False,
            "python/pptx2svg/src/pptx2svg/x.py": False,
            "python/docx2svg/src/docx2svg/x.py": False,
        }
        for path, runs in cases.items():
            self.assertEqual("xlsx-agent" in ci.affected([path]), runs, path)
        # Main still runs it against every sibling it installs.
        self.assertIn("xlsx-agent", _plan("push")["packages"])

    def test_notes_build_only(self) -> None:
        out = _plan("pull_request", ["python/docx-agent/CHANGELOG.md", "python/ooxml-common/ROADMAP.md"])
        self.assertEqual(out["packages"], [])
        self.assertIs(out["any"], False)
        self.assertEqual([b["package"] for b in out["build"]["include"]], ["ooxml-common", "docx-agent"])
        self.assertIs(out["any_build"], True)
        for key in ("rust", "rust_module", "rust_wheels", *(f"rust_{job}" for job in ci.RUST_JOBS)):
            self.assertIs(out[key], False, key)
        # Notes with code: the suites and their builds as before.
        out = _plan("pull_request", ["python/docx-agent/CHANGELOG.md", "python/docx-agent/src/docx_agent/x.py"])
        self.assertEqual(out["packages"], ["docx-agent"])
        self.assertEqual([b["package"] for b in out["build"]["include"]], ["docx-agent"])
        # A package's README and docs are read by its tests: they select the suite.
        self.assertEqual(ci.affected(["python/docx-agent/README.md"]), ["docx-agent"])
        self.assertEqual(ci.affected(["python/docx-agent/docs/common-tasks.md"]), ["docx-agent"])

    def test_the_oracles_tests_select_docx_agents_legs(self) -> None:
        """The oracle's Python tests (the harness's, the recorded-Word tier) run in docx-agent's
        legs, so a change to them, or to what they read, selects those legs alone."""
        for path in ("crates/ooxml-oracle/word/observations/e4-word.json",
                     "crates/ooxml-oracle/word/probes/e4_probe.py", "crates/ooxml-oracle/word/tests/recorded.py",
                     "crates/ooxml-oracle/python/tests/test_reference_guard.py",
                     "crates/ooxml-oracle/python/word_edits_diff.py", "crates/ooxml-oracle/reference.toml",
                     "crates/ooxml-oracle/pytest.ini"):
            self.assertEqual(ci.affected([path]), ["docx-agent"], path)
        self.assertEqual(ci.affected(["crates/ooxml-oracle/known-differences.toml"]), [])
        self.assertEqual(ci.PACKAGES["docx-agent"]["also_tests"], "crates/ooxml-oracle")
        self.assertTrue((ci.ROOT / "crates" / "ooxml-oracle" / "pytest.ini").is_file())

    def test_docs_and_other_tools_run_nothing(self) -> None:
        for changed in (["docs/architecture/README.md"], ["README.md"], [".github/workflows/gates.yml"],
                        ["tools/gates.py", "tools/test_gates.py"], [".github/workflows/release.yml"]):
            out = _plan("pull_request", changed)
            self.assertIs(out["any"], False, changed)
            self.assertIs(out["any_build"], False, changed)
            for key in ("rust", "rust_module", "rust_wheels", "rust_fuzz_check",
                        *(f"rust_{job}" for job in ci.RUST_JOBS)):
                self.assertIs(out[key], False, (changed, key))

    def test_test_workflows_run_everything(self) -> None:
        for path in ci.RUN_ALL_PREFIXES:
            out = _plan("pull_request", [path + ("x.yml" if path.endswith("/") else "")])
            self.assertEqual(out["packages"], list(ci.PACKAGES), path)
            for key in ("rust", "rust_module", "rust_wheels", "rust_fuzz_check"):
                self.assertIs(out[key], True, (path, key))
            self.assertEqual({job for job in ci.RUST_JOBS if out[f"rust_{job}"]}, set(ci.RUST_JOBS) - {"fuzz"})

    def test_push(self) -> None:
        out = _plan("push")
        for name in ci.PACKAGES:
            self.assertEqual(_legs(out, "linux")[name], ["3.15"])
        self.assertEqual(_legs(out, "windows"), {name: ["3.15"] for name in SMOKE})
        self.assertEqual(out["macos"]["include"], [])
        self.assertIs(out["any_macos"], False)

    def test_nightly_and_manual(self) -> None:
        for event in ("schedule", "workflow_dispatch"):
            out = _plan(event)
            self.assertIs(out["any_macos"], True)
            self.assertIs(out["any_windows"], True)
            for name in ci.PACKAGES:
                versions = ci._pythons(name)
                self.assertEqual(versions[0], ci.PACKAGES[name]["min_python"])
                self.assertEqual(_legs(out, "linux")[name], versions, event)
                if ci.PACKAGES[name].get("windows", True):
                    self.assertEqual(_legs(out, "windows")[name], ["3.15"], event)
                else:
                    self.assertNotIn(name, _legs(out, "windows"), event)
                self.assertEqual(_legs(out, "macos")[name], ["3.15"], event)
            # production is Linux only: docx-agent has no Windows leg (R4 #8)
            self.assertNotIn("docx-agent", _legs(out, "windows"))

    def test_free_threaded_legs_linux_only(self) -> None:
        for event in ("pull_request", "push", "schedule", "workflow_dispatch"):
            out = _plan(event, ["tools/ci.py"] if event == "pull_request" else None)
            self.assertEqual([leg["package"] for leg in out["ft"]["include"] if leg.get("shard", 1) == 1],
                             list(ci.PACKAGES), event)
            for leg in out["ft"]["include"]:
                self.assertEqual((leg["os"], leg["python"]), ("ubuntu-latest", "3.15t"))
            for os_ in ci.RUNNERS:
                self.assertFalse([leg for leg in out[os_]["include"] if leg["python"].endswith("t")])

    def test_free_threaded_legs_follow_the_path_filter(self) -> None:
        out = _plan("pull_request", ["python/pptx-agent/src/pptx_agent/x.py"])
        self.assertEqual([leg["package"] for leg in out["ft"]["include"]], ["pptx-agent"])

    def test_one_coverage_leg_per_package_every_event(self) -> None:
        for event in ("pull_request", "push", "schedule"):
            out = _plan(event, ["tools/ci.py"] if event == "pull_request" else None)
            covered = [leg for os_ in ci.RUNNERS for leg in out[os_]["include"]
                       if leg.get("coverage") and leg.get("shard", 1) == 1]
            self.assertEqual(sorted(leg["package"] for leg in covered), sorted(ci.PACKAGES), event)
            for leg in covered:
                self.assertEqual(leg["os"], "ubuntu-latest")
                self.assertEqual(leg["python"], ci._pythons(leg["package"])[-1])


class ShardTest(unittest.TestCase):
    def test_sharded_legs(self) -> None:
        for event in ("pull_request", "push", "schedule"):
            out = _plan(event, ["tools/ci.py"] if event == "pull_request" else None)
            for key in ("linux", "ft"):
                for name, spec in ci.PACKAGES.items():
                    legs = [leg for leg in out[key]["include"] if leg["package"] == name]
                    count = spec.get("shards", 1)
                    for python in {leg["python"] for leg in legs}:
                        mine = [leg for leg in legs if leg["python"] == python]
                        if count == 1:
                            self.assertEqual(len(mine), 1)
                            self.assertNotIn("shard", mine[0])
                        else:
                            self.assertEqual([(leg["shard"], leg["shards"]) for leg in mine],
                                             [(k, count) for k in range(1, count + 1)], (event, key, name))
            # Windows and macOS legs are never split.
            for os_ in ("windows", "macos"):
                self.assertFalse([leg for leg in out[os_]["include"] if "shard" in leg])
        self.assertEqual(ci.PACKAGES["docx-agent"]["shards"], 4)

    def test_core_legs(self) -> None:
        out = _plan("push")
        for key in ("linux", "windows", "ft"):
            for leg in out[key]["include"]:
                self.assertEqual(leg.get("core", False), leg["package"] in ("docx2svg", "docx-agent", "xlsx-agent"), leg)

    def test_every_file_in_exactly_one_shard(self) -> None:
        for name, spec in ci.PACKAGES.items():
            count = spec.get("shards", 1)
            if count == 1:
                continue
            files = ci.test_files(name)
            self.assertGreater(len(files), count)
            for free_threading in (False, True):
                shards = [ci.shard_files(name, k, count, free_threading) for k in range(1, count + 1)]
                flat = [f for shard in shards for f in shard]
                self.assertEqual(sorted(flat), files)
                self.assertTrue(all(shards))

    def test_split_balances_and_is_stable(self) -> None:
        durations = {"a": 10.0, "b": 9.0, "c": 6.0, "d": 4.0, "e": 1.0}
        self.assertEqual(ci.split(sorted(durations), durations, 2), [["a", "d", "e"], ["b", "c"]])
        # An unrecorded file weighs the median of the recorded ones.
        self.assertEqual(ci.split(["a", "b", "new"], {"a": 5.0, "b": 1.0, "c": 3.0}, 2), [["a"], ["b", "new"]])
        # Nothing recorded: every file the same, round robin by name.
        self.assertEqual(ci.split(["w", "x", "y", "z"], {}, 2), [["w", "y"], ["x", "z"]])
        with self.assertRaises(ValueError):
            ci.shard_files("docx-agent", 5, 4)

    def test_durations_from_junit(self) -> None:
        import tempfile

        report = """<?xml version="1.0"?><testsuites><testsuite>
<testcase file="tests/test_a.py" name="x" time="1.25"/><testcase file="tests/test_a.py" name="y" time="2"/>
<testcase file="tests/test_b.py" name="z" time="0.5"/><testcase name="no file" time="9"/>
</testsuite></testsuites>"""
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "junit.xml"
            path.write_text(report)
            self.assertEqual(ci.durations_from_junit([path, path]), {"tests/test_a.py": 6.5, "tests/test_b.py": 1.0})

    def test_recorded_durations_name_the_packages_files(self) -> None:
        if not ci.DURATIONS.is_file():
            self.skipTest("no durations recorded")
        data = json.loads(ci.DURATIONS.read_text())
        for name, kinds in data.items():
            self.assertIn("shards", ci.PACKAGES[name], name)
            self.assertEqual(set(kinds), {"gil", "free-threaded"}, name)
            for durations in kinds.values():
                self.assertTrue(set(durations) & set(ci.test_files(name)), name)


def _workspace_deps(crate: str) -> set[str]:
    """``crate`` and the architecture's crates it depends on, transitively (Cargo.toml)."""
    import tomllib

    found, todo = set(), [crate]
    while todo:
        name = todo.pop()
        if name in found:
            continue
        found.add(name)
        manifest = tomllib.loads((ci.ROOT / "crates" / name / "Cargo.toml").read_text())
        for table in ("dependencies", "dev-dependencies", "build-dependencies"):
            todo += [dep for dep in manifest.get(table, {}) if (ci.ROOT / "crates" / dep).is_dir()]
    return found


class RustPlanTest(unittest.TestCase):
    def test_rust_prefixes(self) -> None:
        for path in ("clippy.toml", "deny.toml", "DEPENDENCIES.md", "fuzz/fuzz_targets/zip_read.rs",
                     "xtask/src/codegen.rs", "crates/ooxml-store/src/lib.rs"):
            out = _plan("pull_request", [path])
            self.assertIs(out["rust"], True, path)
        self.assertIs(_plan("pull_request", ["docs/architecture/README.md"])["rust"], False)
        # The Python module is not built from these: no package suite runs.
        self.assertEqual(ci.affected(["fuzz/fuzz_targets/zip_read.rs", "xtask/src/codegen.rs", "clippy.toml",
                                      "DEPENDENCIES.md", "deny.toml", "rustfmt.toml"]), [])
        self.assertEqual(ci.affected(["crates/ooxml-store/src/lib.rs"]), ["docx-agent"])

    def test_module_inputs(self) -> None:
        inputs = ("crates/ooxml-store/src/lib.rs", "crates/ooxml-store/Cargo.toml", "crates/word/src/lib.rs",
                  "crates/ooxml-text/build.rs", "crates/ooxml-text/data/x.bin", "python/ooxml-core/pyproject.toml",
                  "Cargo.lock", ".cargo/config.toml")
        others = ("crates/ooxml-store/tests/loom.rs", "crates/ooxml-word-edit/tests/integration/history.rs",
                  "crates/ooxml-word-layout/benches/x.rs", "crates/ooxml-xml/examples/xml_contract.rs",
                  "crates/ooxml-text/README.md", "crates/ooxml-oracle/src/lib.rs",
                  "python/ooxml-core/tests/test_layout.py", "python/ooxml-core/CHANGELOG.md", "fuzz/Cargo.toml",
                  "xtask/src/codegen.rs", "python/docx-agent/src/docx_agent/x.py")
        for path in inputs:
            self.assertIs(ci.module_input(path), True, path)
        for path in others:
            self.assertIs(ci.module_input(path), False, path)
        # docx-agent's shards: the module's code, not its crates' tests.
        self.assertEqual(ci.affected(["crates/ooxml-word-edit/tests/integration/history.rs"]), [])
        out = _plan("pull_request", ["crates/ooxml-word-edit/tests/integration/history.rs"])
        self.assertIs(out["rust"], True)
        self.assertIs(out["rust_module"], False)
        self.assertIs(out["rust_wheels"], False)

    def test_differential_and_wheels(self) -> None:
        cases = {  # path: (rust_module, rust_wheels)
            "crates/ooxml-word-edit/src/lib.rs": (True, False),
            "crates/ooxml-word-edit/Cargo.toml": (True, True),
            "crates/ooxml-fonts/build.rs": (True, True),
            "crates/python/src/lib.rs": (True, True),
            "python/ooxml-core/pyproject.toml": (True, True),
            "python/ooxml-core/tests/test_layout.py": (True, False),
            "Cargo.lock": (True, True),
            "python/docx-agent/src/docx_agent/x.py": (True, False),
            "python/docx2svg/src/docx2svg/x.py": (True, False),
            "python/pptx-agent/src/pptx_agent/x.py": (False, False),
            "crates/ooxml-oracle/src/lib.rs": (False, False),
            "xtask/src/codegen.rs": (False, False),
        }
        for path, (module, wheels) in cases.items():
            out = _plan("pull_request", [path])
            self.assertEqual((out["rust_module"], out["rust_wheels"]), (module, wheels), path)
        for event in ("push", "schedule", "workflow_dispatch"):
            out = _plan(event)
            self.assertEqual((out["rust_module"], out["rust_wheels"], out["rust_fuzz_check"]), (True, True, False))

    def test_fuzz_never_on_a_pull_request(self) -> None:
        for path in ("fuzz/fuzz_targets/opc_open.rs", "crates/ooxml-opc/src/lib.rs", "Cargo.lock",
                     ".github/workflows/rust.yml"):
            out = _plan("pull_request", [path])
            self.assertIs(out["rust_fuzz"], False, path)
            self.assertIs(out["rust_fuzz_check"], True, path)
        self.assertIs(_plan("pull_request", ["crates/ooxml-text/src/lib.rs"])["rust_fuzz_check"], False)

    def _jobs(self, event: str, changed: list[str] | None = None) -> set[str]:
        out = _plan(event, changed)
        return {job for job in ci.RUST_JOBS if out[f"rust_{job}"]}

    def test_rust_jobs_follow_their_paths(self) -> None:
        cases = {
            "python/pptx-agent/src/pptx_agent/x.py": set(),
            "crates/word/src/lib.rs": set(),
            "fuzz/fuzz_targets/opc_open.rs": set(),
            "fuzz/Cargo.toml": {"deny"},
            "crates/ooxml-store/src/lib.rs": {"loom", "models"},
            "crates/ooxml-store/Cargo.toml": {"loom", "deny", "models"},
            "crates/ooxml-opc/src/lib.rs": {"oracles", "models"},
            "crates/ooxml-text/src/lib.rs": {"models"},
            "crates/ooxml-oracle/python/drawingml_diff.py": {"models"},
            "xtask/src/codegen.rs": {"models"},
            "python/docx2svg/src/docx2svg/rust.py": set(),
            "python/ooxml-edit/src/ooxml_edit/opc.py": {"oracles"},
            "python/ooxml-common/src/ooxml_common/x.py": {"oracles", "models"},
            "DEPENDENCIES.md": {"deny"},
            "Cargo.lock": set(ci.RUST_JOBS) - {"fuzz"},
            "tools/rust_oracles.py": {"oracles"},
            "tools/gates.py": set(),
            ".github/workflows/rust.yml": set(ci.RUST_JOBS) - {"fuzz"},
        }
        for path, jobs in cases.items():
            self.assertEqual(self._jobs("pull_request", [path]), jobs, path)

    def test_rust_jobs_per_event(self) -> None:
        self.assertEqual(self._jobs("push"), set(ci.RUST_JOBS) - {"fuzz"})
        for event in ("schedule", "workflow_dispatch"):
            self.assertEqual(self._jobs(event), set(ci.RUST_JOBS), event)

    @unittest.skipIf(sys.version_info < (3, 11), "tomllib")
    def test_not_module_crates_are_outside_the_module(self) -> None:
        import tomllib

        folders = {tomllib.loads(path.read_text())["package"]["name"]: path.parent.name
                   for path in (ci.ROOT / "crates").glob("*/Cargo.toml")}
        found, todo = set(), ["python"]
        while todo:
            folder = todo.pop()
            if folder in found:
                continue
            found.add(folder)
            manifest = tomllib.loads((ci.ROOT / "crates" / folder / "Cargo.toml").read_text())
            for table in ("dependencies", "build-dependencies"):
                todo += [folders[dep] for dep in manifest.get(table, {}) if dep in folders]
        for folder in ci.NOT_MODULE_CRATES:
            self.assertTrue((ci.ROOT / "crates" / folder).is_dir(), folder)
            self.assertNotIn(folder, found)
        for prefix in ci.BINDING_PREFIXES:
            self.assertTrue((ci.ROOT / prefix).is_dir(), prefix)

    @unittest.skipIf(sys.version_info < (3, 11), "tomllib")
    def test_crate_lists_cover_their_dependencies(self) -> None:
        for job_crate, listed in (("ooxml-opc", ci.FUZZ_CRATES), ("ooxml-store", ci.LOOM_CRATES),
                                  ("ooxml-oracle", ci.MODEL_CRATES)):
            self.assertEqual(_workspace_deps(job_crate), set(listed), job_crate)


class MainPushTest(unittest.TestCase):
    """A push to main runs every package and job, but builds the wheels only when its changes
    would on a pull request; the nightly and a manual run build them every time."""

    def test_wheels_follow_the_change(self) -> None:
        crate = ["crates/ooxml-word-edit/src/set_text.rs"]
        out = _plan("push", crate)
        self.assertFalse(out["rust_wheels"])
        self.assertTrue(out["rust"] and out["rust_module"] and out["rust_models"])
        self.assertEqual(out["packages"], list(ci.PACKAGES))
        for changed in (["crates/python/src/lib.rs"], ["Cargo.lock"], ["crates/ooxml-word-edit/Cargo.toml"],
                        [".github/workflows/rust-wheels.yml"]):
            self.assertTrue(_plan("push", changed)["rust_wheels"], changed)
        for event in ("schedule", "workflow_dispatch"):
            self.assertTrue(_plan(event, crate)["rust_wheels"], event)
        self.assertTrue(_plan("push")["rust_wheels"])


if __name__ == "__main__":
    unittest.main()
