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
    legs: dict[str, list[str]] = {}
    for leg in out[os_]["include"]:
        legs.setdefault(leg["package"], []).append(leg["python"])
    return legs


class PlanTest(unittest.TestCase):
    def test_pull_request(self) -> None:
        out = _plan("pull_request", ["tools/ci.py"])
        self.assertEqual(out["packages"], list(ci.PACKAGES))
        for name in ci.PACKAGES:
            versions = ci._pythons(name)
            self.assertEqual(_legs(out, "linux")[name], ["3.15"])
            self.assertEqual(_legs(out, "windows")[name], ["3.15"])
            self.assertEqual(versions[-1], "3.15")
        self.assertEqual(out["macos"]["include"], [])
        self.assertIs(out["any_macos"], False)

    def test_pull_request_affected_only(self) -> None:
        out = _plan("pull_request", ["python/pptx-agent/src/pptx_agent/x.py"])
        self.assertEqual(out["packages"], ["pptx-agent", "xlsx-agent"])
        self.assertEqual(set(_legs(out, "linux")), {"pptx-agent", "xlsx-agent"})
        self.assertIs(out["rust"], False)

    def test_push(self) -> None:
        out = _plan("push")
        for name in ci.PACKAGES:
            self.assertEqual(_legs(out, "linux")[name], ["3.15"])
            self.assertEqual(_legs(out, "windows")[name], ["3.15"])
        self.assertEqual(out["macos"]["include"], [])
        self.assertIs(out["any_macos"], False)

    def test_nightly_and_manual(self) -> None:
        for event in ("schedule", "workflow_dispatch"):
            out = _plan(event)
            self.assertIs(out["any_macos"], True)
            for name in ci.PACKAGES:
                versions = ci._pythons(name)
                self.assertEqual(versions[0], ci.PACKAGES[name]["min_python"])
                self.assertEqual(_legs(out, "linux")[name], versions, event)
                self.assertEqual(_legs(out, "windows")[name], ["3.15"], event)
                self.assertEqual(_legs(out, "macos")[name], ["3.15"], event)

    def test_free_threaded_legs_linux_only(self) -> None:
        for event in ("pull_request", "push", "schedule", "workflow_dispatch"):
            out = _plan(event, ["tools/ci.py"] if event == "pull_request" else None)
            self.assertEqual(out["ft"]["include"],
                             [{"package": n, "os": "ubuntu-latest", "python": "3.15t"} for n in ci.PACKAGES], event)
            for os_ in ci.RUNNERS:
                self.assertFalse([leg for leg in out[os_]["include"] if leg["python"].endswith("t")])

    def test_free_threaded_legs_follow_the_path_filter(self) -> None:
        out = _plan("pull_request", ["python/pptx-agent/src/pptx_agent/x.py"])
        self.assertEqual([leg["package"] for leg in out["ft"]["include"]], ["pptx-agent", "xlsx-agent"])

    def test_one_coverage_leg_per_package_every_event(self) -> None:
        for event in ("pull_request", "push", "schedule"):
            out = _plan(event, ["tools/ci.py"] if event == "pull_request" else None)
            covered = [leg for os_ in ci.RUNNERS for leg in out[os_]["include"] if leg.get("coverage")]
            self.assertEqual(sorted(leg["package"] for leg in covered), sorted(ci.PACKAGES), event)
            for leg in covered:
                self.assertEqual(leg["os"], "ubuntu-latest")
                self.assertEqual(leg["python"], ci._pythons(leg["package"])[-1])


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
        self.assertEqual(ci.affected(["crates/ooxml-store/src/lib.rs"]), ["docx-agent", "xlsx-agent"])

    def _jobs(self, event: str, changed: list[str] | None = None) -> set[str]:
        out = _plan(event, changed)
        return {job for job in ci.RUST_JOBS if out[f"rust_{job}"]}

    def test_rust_jobs_follow_their_paths(self) -> None:
        cases = {
            "python/pptx-agent/src/pptx_agent/x.py": set(),
            "crates/word/src/lib.rs": set(),
            "fuzz/fuzz_targets/opc_open.rs": {"fuzz"},
            "fuzz/Cargo.toml": {"fuzz", "deny"},
            "crates/ooxml-store/src/lib.rs": {"loom", "word_model"},
            "crates/ooxml-store/Cargo.toml": {"loom", "deny", "word_model"},
            "crates/ooxml-opc/src/lib.rs": {"fuzz", "oracles", "word_model"},
            "crates/ooxml-text/src/lib.rs": {"word_model"},
            "crates/ooxml-oracle/python/word_model_diff.py": {"word_model"},
            "xtask/src/codegen.rs": {"word_model"},
            "python/docx2svg/src/docx2svg/model.py": {"word_model"},
            "python/ooxml-edit/src/ooxml_edit/opc.py": {"oracles"},
            "python/ooxml-common/src/ooxml_common/x.py": {"oracles", "word_model"},
            "DEPENDENCIES.md": {"deny"},
            "Cargo.lock": set(ci.RUST_JOBS),
            "tools/rust_oracles.py": set(ci.RUST_JOBS),
            ".github/workflows/rust.yml": set(ci.RUST_JOBS),
        }
        for path, jobs in cases.items():
            self.assertEqual(self._jobs("pull_request", [path]), jobs, path)

    def test_rust_jobs_per_event(self) -> None:
        self.assertEqual(self._jobs("push"), set(ci.RUST_JOBS) - {"fuzz"})
        for event in ("schedule", "workflow_dispatch"):
            self.assertEqual(self._jobs(event), set(ci.RUST_JOBS), event)

    @unittest.skipIf(sys.version_info < (3, 11), "tomllib")
    def test_crate_lists_cover_their_dependencies(self) -> None:
        for job_crate, listed in (("ooxml-opc", ci.FUZZ_CRATES), ("ooxml-store", ci.LOOM_CRATES),
                                  ("ooxml-oracle", ci.WORD_MODEL_CRATES)):
            self.assertEqual(_workspace_deps(job_crate), set(listed), job_crate)


if __name__ == "__main__":
    unittest.main()
