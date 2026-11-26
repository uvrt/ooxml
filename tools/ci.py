"""What CI runs for the Python packages, in one place.

The workflows (.github/workflows/ci.yml, freethreading.yml, release.yml) call this script
rather than spell out each package's installs and test command, so the seven packages'
differences live here and nowhere else:

    python tools/ci.py plan [--event EVENT] [--changed FILE]...
        Which packages to test, as GitHub Actions matrices (written to $GITHUB_OUTPUT when it
        is set, printed otherwise), and whether rust.yml's jobs run (`rust`, and `rust_<job>`
        for its path-filtered jobs, RUST_JOBS).  On a pull request only the packages a change touches, and
        every package that depends on one of them (``uses``, or ``pr_uses``, below); a change
        to a package's notes (NOTES) builds it without running its suite; a change to the test
        workflows or to this script (RUN_ALL_PREFIXES) runs everything.  A push to main, the
        nightly run and a manual run take all, but for rust.yml's wheels: a push to main builds
        them only when its changes (the merge's, against main before it) would on a pull
        request (``wheels_affected``); the nightly run, a manual run and release.yml build them
        every time.
        Which Pythons each OS runs depends on the event (LEGS below): production runs
        CPython 3.15 (moving to 3.15t), development runs on macOS and macOS runners are
        scarce, so a pull request and a push to main run Linux and Windows on the newest
        Python and no macOS; the nightly and a manual run add every older Python on Linux
        and the newest on macOS.
        The free-threaded legs (`ft`, freethreading.yml) run on Linux only: nightly and manual
        runs, and a pull request that changes freethreading.yml.
    python tools/ci.py affected FILE...
        The packages those changed files select, one per line (for a quick look by hand).
    python tools/ci.py install PACKAGE [--free-threading]
        pip-installs the package (editable, with its test extras) and the siblings it needs,
        all from this checkout -- never from the old repositories.
    python tools/ci.py test PACKAGE [--event EVENT] [--free-threading] [--shard K/N] [--junit FILE]
        Runs the package's suite as its own repository's CI ran it; with --shard, only the
        test files of shard K of N (SHARDS below), with --junit, a JUnit report of it.
    python tools/ci.py shard PACKAGE K/N [--free-threading]
        The test files shard K of N runs, one per line.
    python tools/ci.py durations PACKAGE [--free-threading] JUNIT...
        Rewrites the package's GIL (or free-threaded) times in tools/ci_durations.json from
        the JUnit reports of a run's shards (the `junit-*` artifacts of python-tests.yml,
        ci.yml's run or freethreading.yml's), so the split stays even.
    python tools/ci.py gil-check PACKAGE
        On a free-threaded Python: importing the package, its modules and the extension
        modules its leg installs (``gil_imports``) leaves the GIL off.
    python tools/ci.py build-check PACKAGE
        The build job's checks: the version agrees with CHANGELOG.md and the changelog
        fragments (tools/changelog.py check), ruff, mypy where the package type checks.

Standard library only, Python 3.10+.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

import changelog

ROOT = Path(__file__).resolve().parent.parent
PY = ROOT / "python"

PYTHONS = ["3.10", "3.11", "3.12", "3.13", "3.14", "3.15"]

#: What a changed path changes besides the packages: the Rust core's Python module (MODULE)
#: and, within it, its bindings (BINDINGS).  ``uses`` names them as it names siblings.
MODULE = "ooxml-core"
BINDINGS = "ooxml-core bindings"
#: The oracle's own Python tests (crates/ooxml-oracle/pytest.ini: the harness's, python/tests,
#: and the recorded-Word tier, word/tests): run in docx-agent's environment (its tool host) by
#: docx-agent's suite (its ``also_tests``), so they run whenever docx-agent's tests do, and a
#: change to them or to what they read (ORACLE_TEST_PREFIXES) selects docx-agent's legs.
ORACLE_TESTS = "ooxml-oracle tests"
ORACLE_TEST_PREFIXES = ("crates/ooxml-oracle/python/", "crates/ooxml-oracle/word/", "crates/ooxml-oracle/pytest.ini",
                        "crates/ooxml-oracle/reference.toml", "crates/ooxml-oracle/host/")
#: The host goldens (crates/ooxml-oracle/python/host_goldens.py): the oracle's tests replay their
#: docx streams, xlsx-agent's suite their mixed ones (a workbook xlsx-agent holds), so a change to
#: them or to what records and replays them selects xlsx-agent's legs too.
HOST_GOLDENS = "host goldens"
HOST_GOLDENS_PREFIXES = ("crates/ooxml-oracle/host/", "crates/ooxml-oracle/python/host_goldens.py",
                         "crates/ooxml-oracle/python/host_features.py")

#: Each package: the monorepo packages its test run uses (itself included; a package's
#: tests run when any of them changes), its siblings to install (pip requirement paths,
#: relative to python/), its own extras, the oldest Python it supports, and how its suite
#: runs.  ``uses`` is the transitive closure of what is installed, so it is the dependency
#: graph the path filter follows.  ``pr_uses``, where given, replaces it on a pull request: the
#: package's own dependencies without the siblings only its tests install (a push to main
#: still runs the suite against every one of them).  ``"windows": False``: no Windows leg on
#: any event (_select).
PACKAGES: dict[str, dict] = {
    "ooxml-common": {
        # ooxml-edit is test only (tests/test_dates.py, test_numfmt.py read it); the font
        # bundle is pptx2svg's sub-distribution, for tests/test_fonts.py.
        "uses": ["ooxml-common", "ooxml-edit", "pptx2svg"],
        "siblings": ["pptx2svg/packages/pptx2svg-fonts", "ooxml-edit"],
        "extras": "dev",
        "min_python": "3.10",
        "pytest": [],
        "mypy": True,
        # One leg (ubuntu, the newest Python) runs the suite under coverage instead, with
        # this floor: the suite is meant to hold the package by itself, not through the
        # renderers' suites.
        "coverage": "87",
        # Windows's font folders (fonts/office.py, rasterdb.py): a Windows leg on every
        # event (LEGS, "smoke").
        "windows_smoke": True,
        # Its test-only sibling ooxml-edit installs lxml: lxml 7 on the free-threaded leg.
        "lxml7": True,
        "gil_imports": ["fontTools.ttLib", "lxml.etree", "lxml.html", "resvg_py"],
    },
    "ooxml-edit": {
        "uses": ["ooxml-common", "ooxml-edit"],
        "siblings": ["ooxml-common"],
        "extras": "dev",
        "min_python": "3.10",
        "pytest": [],
        "ft_pytest": ["-v", "-o", "faulthandler_timeout=300"],
        "mypy": True,
        "mypy_siblings": ["ooxml-common"],
        "lxml7": True,
        # The worker's pipes in binary mode (tools/subworker.py, msvcrt) and zlib-ng's
        # deflate (opc.py): a Windows leg on every event (LEGS, "smoke").
        "windows_smoke": True,
        "gil_imports": ["lxml.etree", "lxml.html"],
    },
    "docx2svg": {
        # A thin layer over the Rust core's layout and drawing (ooxml-core, built here by maturin).
        "uses": ["ooxml-common", "ooxml-core", "docx2svg"],
        # On a pull request: the bindings, not the Rust core's other crates (its tests hold the
        # layer to the core's own answers; a push to main runs them against every crate).
        "pr_uses": ["ooxml-common", BINDINGS, "docx2svg"],
        "siblings": ["ooxml-common", "ooxml-core"],
        "extras": "dev",
        "min_python": "3.10",
        "pytest": ["-n", "auto", "--durations=30"],
        # A pull request leaves out the tests marked long; main, nightly and manual runs
        # take them.
        "pr_pytest": ["-m", "not long"],
        # Parallel as on the GIL leg (serial, it was the slowest free-threaded leg).
        "ft_pytest": ["-n", "auto"],
        "mypy": True,
        "mypy_siblings": ["ooxml-common"],
        "fonts": True,
        # The committed fixture is regenerable and current.
        "fixture_check": True,
        # No Windows leg: ooxml-core's wheels are Linux only, and production runs Linux.
        "windows": False,
        "gil_imports": ["ooxml_core"],
    },
    "pptx2svg": {
        "uses": ["ooxml-common", "pptx2svg"],
        "siblings": ["ooxml-common"],
        "editable_siblings": ["pptx2svg/packages/pptx2svg-fonts"],
        "extras": "dev",
        "min_python": "3.10",
        "pytest": [],
        "mypy": False,
        "gil_imports": ["resvg_py", "pypdfium2", "fontTools.ttLib", "PIL.Image", "numpy"],
    },
    "docx-agent": {
        # ooxml-core (the Rust core, built here by maturin) serves its checks of ranges.
        "uses": ["ooxml-common", "ooxml-edit", "docx2svg", "ooxml-core", "docx-agent", ORACLE_TESTS],
        # The oracle's tests (ORACLE_TESTS), run after the suite on its first shard, GIL legs only.
        "also_tests": "crates/ooxml-oracle",
        "siblings": ["ooxml-common", "ooxml-edit", "docx2svg", "ooxml-core"],
        "extras": "dev,png",
        "min_python": "3.10",
        # A test past 5 minutes dumps every thread's stack (a hang is named, not a timeout).
        "pytest": ["-n", "auto", "--durations=30", "-o", "faulthandler_timeout=300"],
        "ft_pytest": ["-n", "auto", "-o", "faulthandler_timeout=300", "-v"],
        # The suite takes about 18 minutes on one Linux runner: split into this many jobs
        # on Linux (SHARDS below).
        "shards": 4,
        # No Windows leg, nightly either: production runs Linux only, and docx-agent has no
        # code of its own for Windows (its nightly Windows failures were in its tests'
        # deadlines and locks, R4 #8).
        "windows": False,
        "compile": ["src", "tests", "tools"],
        "mypy": False,
        "fonts": True,
        "lxml7": True,
        "gil_imports": ["lxml.etree", "lxml.html", "ooxml_core"],
    },
    "pptx-agent": {
        "uses": ["ooxml-common", "ooxml-edit", "pptx2svg", "pptx-agent"],
        "siblings": ["ooxml-common", "ooxml-edit", "pptx2svg[png]", "pptx2svg/packages/pptx2svg-fonts"],
        "extras": "dev,png",
        # pytest-xdist is installed for CI only, not in the dev extra, so a bare local
        # `pytest` stays serial.
        "pip_extra": ["pytest-xdist>=3"],
        "min_python": "3.10",
        "pytest": ["-n", "auto", "--durations=30"],
        "ft_pytest": [],
        "mypy": False,
        "lxml7": True,
        "gil_imports": ["lxml.etree", "lxml.html", "resvg_py", "PIL.Image"],
    },
    "xlsx-agent": {
        # HOST_GOLDENS: its suite replays the host goldens' mixed streams (a workbook beside a
        # Word document, tests/test_host_goldens_mixed.py).
        "uses": ["ooxml-common", "ooxml-edit", "pptx2svg", "pptx-agent", "docx2svg", "ooxml-core", "docx-agent",
                 "xlsx-agent", HOST_GOLDENS],
        # On a pull request: its dependencies and the Python bindings of ooxml-core, not the
        # Rust core's other crates nor the test-only siblings (main runs those).
        "pr_uses": ["ooxml-common", "ooxml-edit", BINDINGS, "xlsx-agent", HOST_GOLDENS],
        # Test only: pptx2svg and pptx-agent (a deck filled from a workbook's table),
        # docx2svg and docx-agent (a workbook's rich-text cell in a Word document), and
        # docx-agent's ooxml-core.
        "siblings": ["ooxml-common", "ooxml-edit", "pptx2svg", "pptx-agent", "docx2svg", "ooxml-core",
                     "docx-agent"],
        "extras": "test",
        "min_python": "3.11",
        # pytest-xdist for CI only, as pptx-agent: the suite builds ooxml-core and runs
        # serially otherwise, the slowest leg after docx-agent was split.
        "pip_extra": ["pytest-xdist>=3"],
        "pytest": ["-n", "auto"],
        "ft_pytest": ["-n", "auto"],
        "compile": ["src", "tests"],
        "mypy": True,
        "mypy_siblings": ["ooxml-common", "ooxml-edit"],
        "lxml7": True,
        "gil_imports": ["lxml.etree", "lxml.html", "ooxml_core"],
    },
}

IMPORT_NAMES = {name: name.replace("-", "_") for name in PACKAGES}

#: The distributions with extension modules that a leg may install, and their top-level
#: module.  A package's ``gil_imports`` (gil-check imports them) are exactly the modules of
#: those its leg installs -- its own dependencies, its extras' and its siblings' (pip
#: requirements in pyproject.toml); tools/test_ci.py derives that set from the pyproject
#: files and compares, so a dependency dropped or added updates the list or fails the plan
#: job.  The tools (ruff, mypy, pytest) are not the packages' modules and are not listed.  A
#: new extension dependency goes here.
EXTENSION_DISTRIBUTIONS = {
    "fonttools": "fontTools", "lxml": "lxml", "numpy": "numpy", "ooxml-core": "ooxml_core",
    "pillow": "PIL", "pymupdf": "pymupdf", "pypdfium2": "pypdfium2", "resvg-py": "resvg_py",
}

#: A change to one of these runs every job of the test workflows: those workflows and this
#: script.  The other files under .github/ and tools/ (gates, release, the release tools) run
#: none of them: ci.yml's plan job runs tools/'s unit tests on every pull request.
RUN_ALL_PREFIXES = tuple(f".github/workflows/{name}.yml" for name in
                         ("ci", "python-tests", "freethreading", "rust", "rust-wheels")) + (
                     ".github/actions/", "tools/ci.py", "tools/ci_durations.json")

#: A package's notes, which no test reads: a change to them, or to its changelog fragments
#: (python/<package>/changes/, tools/changelog.py), selects the package's build job
#: (build-check compares CHANGELOG.md and the fragments with the version), not its suite nor
#: rust.yml's jobs.
NOTES = ("CHANGELOG.md", "ROADMAP.md", "CONTRIBUTING.md")

#: The workspace's own files: a change to one rebuilds every crate (and fuzz/, whose crates
#: inherit the workspace's dependencies).
RUST_WORKSPACE = ("Cargo.toml", "Cargo.lock", ".cargo/")

#: What the Python module ooxml-core is built from: the workspace's files, python/ooxml-core
#: (its tests aside) and the source of crates/python and every crate it depends on, which is
#: every crate but NOT_MODULE_CRATES (tools/test_ci.py checks those against Cargo.toml).  A new
#: crate counts as part of the module until it is listed here.  A crate's tests, benches and
#: examples are not built into it.  BINDING_PREFIXES: the module's Python bindings.
NOT_MODULE_CRATES = ("ooxml-oracle",)
CRATE_TEST_FOLDERS = ("tests", "benches", "examples")
BINDING_PREFIXES = ("python/ooxml-core/", "crates/python/", "crates/ooxml-python-session/")

#: rust.yml's `rust` job (fmt, clippy, cargo test) runs on a change to the workspace or to the
#: packages whose files the crates' tests read or the differential tests compare with
#: (RUST_REFERENCE).  The lints, the dependency policy, the fuzz targets and xtask's code
#: generation run rust.yml only.
RUST_PREFIXES = RUST_WORKSPACE + ("crates/", "python/ooxml-core/", "rustfmt.toml", "clippy.toml", "deny.toml",
                                  "DEPENDENCIES.md", "fuzz/", "xtask/")
RUST_REFERENCE = ("ooxml-common", "ooxml-edit", "docx2svg", "docx-agent")


def _is_notes(path: str) -> bool:
    parts = path.split("/")
    return parts[0] == "python" and (len(parts) == 3 and parts[2] in NOTES
                                     or len(parts) == 4 and parts[2] == changelog.CHANGES)


def module_input(path: str) -> bool:
    """Whether ``path`` is something the Python module ooxml-core is built from."""
    if path.startswith(RUST_WORKSPACE):
        return True
    parts = path.split("/")
    if path.startswith("python/ooxml-core/"):
        return parts[2] != "tests" and not _is_notes(path)
    return (len(parts) > 2 and parts[0] == "crates" and parts[1] not in NOT_MODULE_CRATES
            and parts[2] not in CRATE_TEST_FOLDERS and not path.endswith(".md"))


def _units(path: str) -> set[str]:
    """The packages and units (MODULE, BINDINGS) a change to ``path`` changes."""
    units: set[str] = set()
    if module_input(path):
        units.add(MODULE)
        if path.startswith(BINDING_PREFIXES + RUST_WORKSPACE):
            units.add(BINDINGS)
    if path.startswith(ORACLE_TEST_PREFIXES):
        units.add(ORACLE_TESTS)
    if path.startswith(HOST_GOLDENS_PREFIXES):
        units.add(HOST_GOLDENS)
    parts = path.split("/")
    if len(parts) > 2 and parts[0] == "python" and parts[1] in PACKAGES and not _is_notes(path):
        units.add(parts[1])
    return units


def rust_affected(changed: list[str]) -> bool:
    for path in changed:
        if path.startswith(RUN_ALL_PREFIXES + RUST_PREFIXES):
            return True
        if _units(path) & set(RUST_REFERENCE):
            return True
    return False


def rust_module_affected(changed: list[str]) -> bool:
    """rust.yml's `differential` and `rust-check`: the module, the packages they compare with
    or make their documents with (RUST_REFERENCE), or their own tests changed."""
    return any(path.startswith(RUN_ALL_PREFIXES + ("python/ooxml-core/tests/",))
               or _units(path) & {MODULE, *RUST_REFERENCE} for path in changed)


def wheels_affected(changed: list[str]) -> bool:
    """rust-wheels.yml: the bindings (the stable ABI, the free-threaded build), the workspace's
    and the module's crates' manifests and build scripts (a native dependency, manylinux);
    the crates' other code is built by `rust`, `differential` and `rust-check` already."""
    return any(path.startswith(RUN_ALL_PREFIXES) or BINDINGS in _units(path)
               or (module_input(path) and path.endswith(("/Cargo.toml", "/build.rs"))) for path in changed)


def _crates(names: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(f"crates/{name}/" for name in names)

#: The architecture's crates a job of RUST_JOBS builds: the crate it runs and every
#: workspace crate that one depends on (tools/test_ci.py checks them against Cargo.toml).
FUZZ_CRATES = ("ooxml-base", "ooxml-xml", "ooxml-zip", "ooxml-opc")
LOOM_CRATES = ("ooxml-store",)
MODEL_CRATES = ("ooxml-oracle", "ooxml-base", "ooxml-xml", "ooxml-zip", "ooxml-opc", "ooxml-derive",
                     "ooxml-fonts", "ooxml-text", "ooxml-word-model", "ooxml-colour", "ooxml-schema",
                     "ooxml-drawingml", "ooxml-scene", "ooxml-metafile", "ooxml-word-layout", "ooxml-numfmt",
                     "ooxml-chart", "ooxml-render", "ooxml-tools", "ooxml-tools-derive", "ooxml-tools-word",
                      "ooxml-raster", "ooxml-style-names", "ooxml-word-pages", "ooxml-session", "ooxml-session-word", "ooxml-host", "ooxml-store",
                      "ooxml-word-edit", "ooxml-markdown", "ooxml-sheets")


#: rust.yml's jobs that run on a pull request only when it changes their own paths (or
#: RUN_ALL_PREFIXES or RUST_WORKSPACE), and the events that run them.  rust.yml's other jobs
#: (fmt, clippy, tests, differential, wheels) follow `rust`, `rust_module` and `rust_wheels`.
#: - fuzz: fuzz/'s targets, 4 minutes each; nightly and manual runs only.  On a pull request
#:   that changes its paths the `rust` job checks that the targets still build (fuzz_check).
#: - loom: ooxml-store's commit path, model-checked.
#: - oracles: tools/rust_oracles.py -- the XML writer and the package save against ooxml-edit.
#: - models: the DrawingML model and colours and the charts against ooxml-common's
#:   (crates/ooxml-oracle/python/drawingml_diff.py, chart_diff.py), and the preset table.
#: - deny: cargo-deny (deny.toml) when a Cargo file or the dependency policy changes.
FULL_EVENTS = ("push", "schedule", "workflow_dispatch")
PR_EVENTS = ("pull_request", *FULL_EVENTS)
RUST_JOBS: dict[str, dict] = {
    "fuzz": {"paths": ("fuzz/", *_crates(FUZZ_CRATES)), "events": ("schedule", "workflow_dispatch")},
    "loom": {"paths": _crates(LOOM_CRATES), "events": PR_EVENTS},
    "oracles": {"paths": ("python/ooxml-edit/", "python/ooxml-common/", "tools/rust_oracles.py",
                          *_crates(FUZZ_CRATES)),
                "events": PR_EVENTS},
    "models": {"paths": ("xtask/", "python/ooxml-common/", "python/pptx2svg/", *_crates(MODEL_CRATES)),
               "events": PR_EVENTS},
    "deny": {"paths": ("deny.toml", "DEPENDENCIES.md"), "suffixes": ("Cargo.toml", "Cargo.lock"),
             "events": PR_EVENTS},
}


def _job_paths_changed(job: str, changed: list[str] | None) -> bool:
    spec = RUST_JOBS[job]
    if changed is None:
        return True
    for path in changed:
        if _is_notes(path):
            continue
        if path.startswith(RUN_ALL_PREFIXES + RUST_WORKSPACE + spec["paths"]):
            return True
        if path.endswith(spec.get("suffixes", ())):
            return True
    return False


def rust_job_runs(job: str, event: str, changed: list[str] | None) -> bool:
    """Whether rust.yml's ``job`` (RUST_JOBS) runs on ``event``, a pull request changing ``changed``."""
    if event not in RUST_JOBS[job]["events"]:
        return False
    return event != "pull_request" or _job_paths_changed(job, changed)


def affected(changed: list[str]) -> list[str]:
    """The packages whose tests a pull request changing ``changed`` (paths from the root)
    selects: those whose ``pr_uses`` (or ``uses``) a changed path changes."""
    if any(path.startswith(RUN_ALL_PREFIXES) for path in changed):
        return list(PACKAGES)
    touched = set().union(*map(_units, changed)) if changed else set()
    return [name for name, spec in PACKAGES.items() if touched & set(spec.get("pr_uses", spec["uses"]))]


def built(changed: list[str]) -> list[str]:
    """The packages whose build job a pull request runs: the tested ones, and those whose
    notes alone changed (build-check reads CHANGELOG.md); every one when the check itself
    (tools/changelog.py) changed."""
    if "tools/changelog.py" in changed:
        return list(PACKAGES)
    names = set(affected(changed))
    names |= {path.split("/")[1] for path in changed if _is_notes(path) and path.split("/")[1] in PACKAGES}
    return [name for name in PACKAGES if name in names]


def _pythons(name: str) -> list[str]:
    low = PACKAGES[name]["min_python"]
    return [v for v in PYTHONS if tuple(map(int, v.split("."))) >= tuple(map(int, low.split(".")))]


#: Which of a package's Pythons each OS runs, per event: "all" (from the package's
#: min_python to the newest), "newest", "smoke" (the newest, for the packages marked
#: windows_smoke only) or None (no legs).  Production runs CPython 3.15 on Linux and is
#: moving to 3.15t (the `ft` legs, Linux, every event); development and local test runs
#: happen on macOS, whose runners are few and shared by every run.  So a pull request and a
#: push to main run Linux on the newest Python, Windows only for the packages with code of
#: their own there (windows_smoke: a few minutes each), and no macOS; the nightly run and a
#: manual run (FULL_LEGS, also any event not named here) run every Python on Linux, so the
#: older ones requires-python still allows keep working, and the newest on Windows and
#: macOS for every package.
LEGS: dict[str, dict[str, str | None]] = {
    "pull_request": {"linux": "newest", "windows": "smoke", "macos": None},
    "push": {"linux": "newest", "windows": "smoke", "macos": None},
}
FULL_LEGS: dict[str, str | None] = {"linux": "all", "windows": "newest", "macos": "newest"}
RUNNERS = {"linux": "ubuntu-latest", "windows": "windows-latest", "macos": "macos-latest"}


def _select(name: str, versions: list[str], which: str | None, os_: str = "linux") -> list[str]:
    """The Pythons ``name`` runs on ``os_`` for ``which`` (LEGS); none on an OS the
    package opts out of (PACKAGES' ``"windows": False``)."""
    if which is None or PACKAGES[name].get(os_, True) is False:
        return []
    if which == "all":
        return versions
    if which == "newest":
        return versions[-1:]
    if which == "smoke":
        return versions[-1:] if PACKAGES[name].get("windows_smoke") else []
    raise ValueError(which)


#: The packages whose install builds the Rust core (ooxml-core, by maturin): their legs
#: carry `core`, and python-tests.yml restores the Rust build cache for them.
def _builds_core(name: str) -> bool:
    return "ooxml-core" in PACKAGES[name]["siblings"]


def _sharded(leg: dict[str, str | bool | int]) -> list[dict[str, str | bool | int]]:
    """``leg``, or one copy per shard (``shard``, ``shards``) for a package split on Linux."""
    count = PACKAGES[str(leg["package"])].get("shards", 1)
    if count == 1 or leg["os"] != RUNNERS["linux"]:
        return [leg]
    return [{**leg, "shard": k, "shards": count} for k in range(1, count + 1)]


def plan(event: str, changed: list[str] | None) -> dict[str, str]:
    every = event != "pull_request" or changed is None     # every package and every job
    # rust.yml's wheels: a push to main filters as a pull request does (release.yml builds them).
    wheels = event not in ("pull_request", "push") or changed is None or wheels_affected(changed)
    names = list(PACKAGES) if every else affected(changed)
    builds = list(PACKAGES) if every else built(changed)
    legs = LEGS.get(event, FULL_LEGS)
    matrices: dict[str, list[dict[str, str | bool | int]]] = {os_: [] for os_ in RUNNERS}
    for name in names:
        versions = _pythons(name)
        for os_, runner in RUNNERS.items():
            for v in _select(name, versions, legs[os_], os_):
                leg: dict[str, str | bool | int] = {"package": name, "os": runner, "python": v}
                if os_ == "linux" and v == versions[-1]:
                    # The coverage leg (PACKAGES' "coverage"): Linux, the newest Python,
                    # which every event runs.
                    leg["coverage"] = True
                if _builds_core(name):
                    leg["core"] = True
                matrices[os_].extend(_sharded(leg))
    ft = [{"package": n, "os": RUNNERS["linux"], "python": "3.15t", **({"core": True} if _builds_core(n) else {})}
          for n in names]
    out = {
        "packages": names,
        "any": bool(names),
        "rust": every or rust_affected(changed),
        # rust.yml's differential and rust-check; its wheels.
        "rust_module": every or rust_module_affected(changed),
        "rust_wheels": wheels,
        # rust.yml's path-filtered jobs (RUST_JOBS): rust_fuzz, rust_loom, ...
        **{f"rust_{job}": rust_job_runs(job, event, changed) for job in RUST_JOBS},
        # A pull request that changes the fuzz targets' paths: the `rust` job builds them.
        "rust_fuzz_check": event == "pull_request" and _job_paths_changed("fuzz", changed),
        **{os_: {"include": legs_} for os_, legs_ in matrices.items()},
        # The free-threaded legs (freethreading.yml): Linux only; a pull request runs them
        # only when it changes that workflow (its `paths`).
        "ft": {"include": [shard for leg in ft for shard in _sharded(leg)]},
        # Whether ci.yml's Windows and macOS calls run at all: an empty matrix is an error
        # in Actions.
        "any_windows": bool(matrices["windows"]),
        "any_macos": bool(matrices["macos"]),
        "any_build": bool(builds),
        "build": {"include": [{"package": n} for n in builds]},
    }
    return {k: json.dumps(v) if not isinstance(v, bool) else str(v).lower() for k, v in out.items()}


def changelog_refusal(event: str, changed: list[str] | None, head_ref: str | None) -> str | None:
    """Why a pull request may not change ``changed``, or None: only a release branch
    (tools/changelog.py, RELEASE_BRANCH) writes a package's CHANGELOG.md; any other pull
    request adds a fragment, python/<package>/changes/<topic>.<kind>.md."""
    if event != "pull_request" or changed is None or changelog.is_release_branch(head_ref):
        return None
    edited = changelog.direct_edits(changed)
    if not edited:
        return None
    return (f"{', '.join(edited)}: CHANGELOG.md is written at release time only (tools/changelog.py). "
            f"Put the entry in python/<package>/{changelog.CHANGES}/<topic>.<kind>.md instead, "
            f"kind one of {', '.join(changelog.KINDS)}.")


def _pip(*args: str) -> None:
    cmd = [sys.executable, "-m", "pip", "install", *args]
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, check=True)


def _req(path: str) -> str:
    """A pip requirement for ``path`` (under python/, maybe with [extras])."""
    base, _, extras = path.partition("[")
    return str(PY / base) + (f"[{extras}" if extras else "")


def install(name: str, free_threading: bool) -> None:
    spec = PACKAGES[name]
    _pip("--upgrade", "pip")
    if free_threading and spec.get("lxml7"):
        # lxml 6 turns the GIL back on when imported on 3.15t; lxml 7 (a pre-release for
        # now) does not, so it goes in first and the other installs find it there.
        _pip("--pre", "lxml>=7.0.0b1")
    args = [_req(s) for s in spec["siblings"]]
    for s in spec.get("editable_siblings", []):
        args += ["-e", _req(s)]
    args += ["-e", _req(f"{name}[{spec['extras']}]")]
    args += spec.get("pip_extra", [])
    # One resolve over all of them: every sibling requirement is met from this checkout.
    _pip(*args)


def test(name: str, event: str, free_threading: bool, coverage: bool = False,
         shard: tuple[int, int] | None = None, junit: Path | None = None) -> int:
    spec = PACKAGES[name]
    cwd = PY / name
    first = shard is None or shard[0] == 1      # the whole-package checks: once per suite
    if "compile" in spec and not free_threading and first:
        # A docstring's invalid escape is a SyntaxWarning, printed once and easy to miss.
        subprocess.run(
            [sys.executable, "-W", "error::SyntaxWarning", "-W", "error::DeprecationWarning",
             "-m", "compileall", "-q", "-f", *spec["compile"]],
            cwd=cwd, check=True,
        )
    if free_threading:
        args = ["-W", "error::RuntimeWarning", "-m", "pytest", "-q", "-W", "error::RuntimeWarning"]
        if spec.get("lxml7"):
            args += ["-W", "error::pytest.PytestUnraisableExceptionWarning"]
        args += spec.get("ft_pytest", [])
    else:
        args = ["-m", "pytest", "-q", *spec["pytest"]]
        if event == "pull_request":
            args += spec.get("pr_pytest", [])
    if junit is not None:
        # xunit1: each test case carries its file, which `durations` sums by.
        args += [f"--junitxml={junit.resolve()}", "-o", "junit_family=xunit1"]
    if shard is not None:
        args += shard_files(name, *shard, free_threading=free_threading)
    env = dict(os.environ)
    if free_threading:
        env["PYTHON_GIL"] = "0"
    if coverage and spec.get("coverage"):
        _pip("coverage>=7.6")
        env["COVERAGE_CORE"] = "sysmon"
        args = ["-m", "coverage", "run", *args]
    cmd = [sys.executable, *args]
    print("+ (in python/%s)" % name, " ".join(cmd), flush=True)
    code = subprocess.run(cmd, cwd=cwd, env=env).returncode
    if code:
        return code
    if coverage and spec.get("coverage"):
        code = subprocess.run([sys.executable, "-m", "coverage", "report", f"--fail-under={spec['coverage']}",
                               "--sort=cover"], cwd=cwd).returncode
    if not code and spec.get("fixture_check") and not free_threading and first:
        code = fixture_check(cwd)
    if not code and spec.get("also_tests") and not free_threading and first:
        code = also_test(ROOT / spec["also_tests"], spec["pytest"])
    return code


def also_test(folder: Path, pytest_args: list[str]) -> int:
    """The suite of ``folder`` (its own pytest.ini), in this environment: a package's
    ``also_tests``."""
    cmd = [sys.executable, "-m", "pytest", "-q", *(a for a in pytest_args if not a.startswith("--durations"))]
    print(f"+ (in {folder.relative_to(ROOT)})", " ".join(cmd), flush=True)
    return subprocess.run(cmd, cwd=folder).returncode


#: Each sharded package's recorded time per test file, in seconds summed over its tests,
#: on the GIL legs ("gil") and the free-threaded ones ("free-threaded"), split each on its
#: own (some files run only there: tests/test_block_edits_ft.py); `durations` writes them
#: from a run's JUnit reports.  A file not in it counts as the
#: median of those that are, so a new test file lands somewhere sensible until the next
#: refresh; a stale entry only makes the split less even, never drops a test.
DURATIONS = ROOT / "tools" / "ci_durations.json"


def test_files(name: str) -> list[str]:
    """The package's test files, relative to its folder: what pytest collects from tests/."""
    folder = PY / name
    return sorted(p.relative_to(folder).as_posix() for p in (folder / "tests").rglob("test_*.py"))


def _kind(free_threading: bool) -> str:
    return "free-threaded" if free_threading else "gil"


def _durations(name: str, free_threading: bool) -> dict[str, float]:
    if not DURATIONS.is_file():
        return {}
    return json.loads(DURATIONS.read_text(encoding="utf-8")).get(name, {}).get(_kind(free_threading), {})


def split(files: list[str], durations: dict[str, float], count: int) -> list[list[str]]:
    """``files`` in ``count`` shards of about equal recorded time: the longest file first,
    each to the shard with the least time so far (the lowest-numbered on a tie).  The
    same inputs give the same split on every runner."""
    known = sorted(durations.values())
    default = known[len(known) // 2] if known else 1.0
    weight = {f: durations.get(f, default) for f in files}
    shards: list[list[str]] = [[] for _ in range(count)]
    totals = [0.0] * count
    for f in sorted(files, key=lambda f: (-weight[f], f)):
        k = min(range(count), key=lambda i: (totals[i], i))
        shards[k].append(f)
        totals[k] += weight[f]
    return [sorted(s) for s in shards]


def shard_files(name: str, index: int, count: int, free_threading: bool = False) -> list[str]:
    """The test files shard ``index`` (from 1) of ``count`` runs."""
    if not 1 <= index <= count:
        raise ValueError(f"shard {index} of {count}")
    return split(test_files(name), _durations(name, free_threading), count)[index - 1]


def durations_from_junit(reports: list[Path]) -> dict[str, float]:
    """Seconds per test file, summed over the test cases of JUnit (xunit1) reports."""
    import xml.etree.ElementTree as ET

    out: dict[str, float] = {}
    for report in reports:
        for case in ET.parse(report).getroot().iter("testcase"):
            f = case.get("file")
            if f:
                out[f] = out.get(f, 0.0) + float(case.get("time") or 0.0)
    return {f: round(t, 1) for f, t in sorted(out.items())}


def write_durations(name: str, reports: list[Path], free_threading: bool = False) -> dict[str, float]:
    found = durations_from_junit(reports)
    data = json.loads(DURATIONS.read_text(encoding="utf-8")) if DURATIONS.is_file() else {}
    data.setdefault(name, {})[_kind(free_threading)] = found
    DURATIONS.write_text(json.dumps(data, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    return found


def _shard_arg(text: str) -> tuple[int, int]:
    index, _, count = text.partition("/")
    return int(index), int(count)


def fixture_check(cwd: Path) -> int:
    """docx2svg: tools/make_layout_sweep.py regenerates tests/fixtures/ to the committed bytes.

    The archive's bytes are deflate's; CPython 3.14+ on Windows deflates with zlib-ng, whose
    bytes differ from zlib's for the same parts, so there the .docx files are left out of the
    diff and tests/test_parse.py's part-by-part comparison is the check.
    """
    import zlib

    subprocess.run([sys.executable, "tools/make_layout_sweep.py"], cwd=cwd, check=True)
    pathspec = ["tests/fixtures/"]
    if hasattr(zlib, "ZLIBNG_VERSION"):
        print("deflate is zlib-ng: tests/fixtures/*.docx compared part by part (tests/test_parse.py)")
        pathspec.append(":(exclude)tests/fixtures/*.docx")
    code = subprocess.run(["git", "diff", "--exit-code", "--", *pathspec], cwd=cwd).returncode
    if code:
        print("tests/fixtures/ is stale: rerun tools/make_layout_sweep.py")
    return code


def gil_check(name: str) -> int:
    spec = PACKAGES[name]
    code = f"""
import importlib, pkgutil, sys
assert hasattr(sys, "_is_gil_enabled"), "not a free-threaded build"
import {IMPORT_NAMES[name]} as package
for module in pkgutil.walk_packages(package.__path__, package.__name__ + "."):
    if not module.name.endswith("__main__"):
        importlib.import_module(module.name)
import {", ".join(spec["gil_imports"])}  # the extension modules its leg installs
if {bool(spec.get('lxml7'))}:
    import lxml.etree
    assert lxml.etree.LXML_VERSION >= (7,), lxml.etree.LXML_VERSION
print(sys.version)
sys.exit("the GIL is enabled" if sys._is_gil_enabled() else 0)
"""
    return subprocess.run([sys.executable, "-W", "error::RuntimeWarning", "-c", code]).returncode


def build_check(name: str) -> None:
    spec = PACKAGES[name]
    cwd = PY / name
    subprocess.run([sys.executable, str(ROOT / "tools" / "changelog.py"), "check", name], check=True)
    subprocess.run(["ruff", "check", "."], cwd=cwd, check=True)
    if spec["mypy"]:
        siblings = spec.get("mypy_siblings", [])
        if siblings:
            _pip(*[_req(s) for s in siblings])
        subprocess.run(["mypy"], cwd=cwd, check=True)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("plan")
    p.add_argument("--event", default=os.environ.get("GITHUB_EVENT_NAME", "pull_request"))
    p.add_argument("--changed", action="append",
                   help="a changed path (repeat); default on a pull request and a push: git diff HEAD^1 HEAD")
    p.add_argument("--head-ref", default=os.environ.get("GITHUB_HEAD_REF"),
                   help="a pull request's branch (default $GITHUB_HEAD_REF): only a release branch writes CHANGELOG.md")
    a = sub.add_parser("affected")
    a.add_argument("files", nargs="+")
    sh = sub.add_parser("shard")
    sh.add_argument("package", choices=list(PACKAGES))
    sh.add_argument("shard", type=_shard_arg, help="K/N")
    sh.add_argument("--free-threading", action="store_true")
    du = sub.add_parser("durations")
    du.add_argument("package", choices=list(PACKAGES))
    du.add_argument("--free-threading", action="store_true", help="the reports are of the free-threaded legs")
    du.add_argument("reports", nargs="+", type=Path)
    for command in ("install", "test", "gil-check", "build-check"):
        s = sub.add_parser(command)
        s.add_argument("package", choices=list(PACKAGES))
        if command in ("install", "test"):
            s.add_argument("--free-threading", action="store_true")
        if command == "test":
            s.add_argument("--event", default=os.environ.get("GITHUB_EVENT_NAME", "push"))
            s.add_argument("--coverage", action="store_true", help="the coverage leg, where the package has one")
            s.add_argument("--shard", type=_shard_arg, help="K/N: only shard K of N's test files")
            s.add_argument("--junit", type=Path, help="write a JUnit (xunit1) report here")
    args = parser.parse_args(argv)

    if args.command == "affected":
        print("\n".join(affected(args.files)))
        return 0
    if args.command == "plan":
        changed = args.changed
        if changed is None and args.event in ("pull_request", "push"):
            # A pull request's checkout is its merge commit, and a push to main is a pull
            # request's merge (main takes merges only): the first parent is the base.
            diff = subprocess.run(["git", "diff", "--name-only", "HEAD^1", "HEAD"], cwd=ROOT,
                                  capture_output=True, text=True, check=True)
            changed = [line for line in diff.stdout.splitlines() if line]
            print(f"{len(changed)} changed files", file=sys.stderr)
        refused = changelog_refusal(args.event, changed, args.head_ref)
        if refused:
            print(refused, file=sys.stderr)
            return 1
        result = plan(args.event, changed)
        target = os.environ.get("GITHUB_OUTPUT")
        if target:
            with open(target, "a", encoding="utf-8") as fh:
                for k, v in result.items():
                    fh.write(f"{k}={v}\n")
        for k, v in result.items():
            print(f"{k}={v}")
        return 0
    if args.command == "install":
        install(args.package, args.free_threading)
        return 0
    if args.command == "test":
        return test(args.package, args.event, args.free_threading, args.coverage, args.shard, args.junit)
    if args.command == "shard":
        print("\n".join(shard_files(args.package, *args.shard, free_threading=args.free_threading)))
        return 0
    if args.command == "durations":
        found = write_durations(args.package, args.reports, args.free_threading)
        print(f"{len(found)} files, {sum(found.values()):.0f} s", file=sys.stderr)
        return 0
    if args.command == "gil-check":
        return gil_check(args.package)
    if args.command == "build-check":
        build_check(args.package)
        return 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
