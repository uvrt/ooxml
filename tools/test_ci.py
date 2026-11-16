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
            self.assertEqual(_legs(out, "linux")[name], [versions[0], versions[-1]])
            self.assertEqual(_legs(out, "windows")[name], [versions[-1]])
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
            versions = ci._pythons(name)
            self.assertEqual(_legs(out, "linux")[name], versions)
            self.assertEqual(_legs(out, "windows")[name], [versions[0], versions[-1]])
        self.assertEqual(out["macos"]["include"], [])
        self.assertIs(out["any_macos"], False)

    def test_nightly_and_manual(self) -> None:
        for event in ("schedule", "workflow_dispatch"):
            out = _plan(event)
            self.assertIs(out["any_macos"], True)
            for name in ci.PACKAGES:
                versions = ci._pythons(name)
                self.assertEqual(_legs(out, "linux")[name], versions, event)
                self.assertEqual(_legs(out, "windows")[name], versions, event)
                self.assertEqual(_legs(out, "macos")[name], [versions[-1]], event)

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


if __name__ == "__main__":
    unittest.main()
