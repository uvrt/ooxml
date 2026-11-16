"""What CI runs for the Python packages, in one place.

The workflows (.github/workflows/ci.yml, rust.yml, release.yml) call this script
rather than spell out each package's installs and test command, so the seven packages'
differences live here and nowhere else:

    python tools/ci.py plan [--event EVENT] [--changed FILE]...
        Which packages to test, as GitHub Actions matrices (written to $GITHUB_OUTPUT when it
        is set, printed otherwise), and whether rust.yml's jobs run (`rust`).  On a pull request only the packages a change touches, and
        every package that depends on one of them (USES below); a change to the workflows or
        to tools/ runs everything.  A push to main, the nightly run and a manual run take all.
        Which Pythons each OS runs depends on the event (LEGS below): macOS runners are
        the scarce ones and development already runs on macOS, so a pull request and a
        push to main run no macOS leg, and the nightly and a manual run one per package.
    python tools/ci.py affected FILE...
        The packages those changed files select, one per line (for a quick look by hand).
    python tools/ci.py install PACKAGE
        pip-installs the package (editable, with its test extras) and the siblings it needs,
        all from this checkout -- never from the old repositories.
    python tools/ci.py test PACKAGE [--event EVENT]
        Runs the package's suite as its own repository's CI ran it.
    python tools/ci.py build-check PACKAGE
        The build job's checks: the version agrees with CHANGELOG.md, ruff, mypy where the
        package type checks.

Standard library only, Python 3.10+.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PY = ROOT / "python"

PYTHONS = ["3.10", "3.11", "3.12", "3.13", "3.14", "3.15"]

#: Each package: the monorepo packages its test run uses (itself included; a package's
#: tests run when any of them changes), its siblings to install (pip requirement paths,
#: relative to python/), its own extras, the oldest Python it supports, and how its suite
#: runs.  ``uses`` is the transitive closure of what is installed, so it is the dependency
#: graph the path filter follows.
PACKAGES: dict[str, dict] = {
    "ooxml-common": {
        # ooxml-edit is test only (tests/test_dates.py, test_numfmt.py read it); the font
        # bundle is pptx2svg's sub-distribution, for tests/test_fonts.py.
        "uses": ["ooxml-common", "ooxml-edit", "pptx2svg"],
        "siblings": ["pptx2svg/packages/pptx2svg-fonts", "ooxml-edit"],
        "extras": "dev",
        "min_python": "3.10",
        "pytest": [],
        "check_version": "tools/check_version.py",
        "mypy": True,
        # One leg (ubuntu, the newest Python) runs the suite under coverage instead, with
        # this floor: the suite is meant to hold the package by itself, not through the
        # renderers' suites.
        "coverage": "87",
    },
    "ooxml-edit": {
        "uses": ["ooxml-common", "ooxml-edit"],
        "siblings": ["ooxml-common"],
        "extras": "dev",
        "min_python": "3.10",
        "pytest": [],
        "check_version": ".github/check_version.py",
        "mypy": True,
        "mypy_siblings": ["ooxml-common"],
    },
    "docx2svg": {
        "uses": ["ooxml-common", "docx2svg"],
        "siblings": ["ooxml-common"],
        "extras": "dev",
        "min_python": "3.10",
        "pytest": ["-n", "auto", "--durations=30"],
        # A pull request leaves out the tests marked long; main, nightly and manual runs
        # take them.
        "pr_pytest": ["-m", "not long"],
        "check_version": "tools/check_version.py",
        "mypy": True,
        "mypy_siblings": ["ooxml-common"],
        "fonts": True,
        # The committed fixture is regenerable and current.
        "fixture_check": True,
    },
    "pptx2svg": {
        "uses": ["ooxml-common", "pptx2svg"],
        "siblings": ["ooxml-common"],
        "editable_siblings": ["pptx2svg/packages/pptx2svg-fonts"],
        "extras": "dev",
        "min_python": "3.10",
        "pytest": [],
        "check_version": "tools/check_version.py",
        "mypy": False,
    },
    "docx-agent": {
        # ooxml-core (the Rust core, built here by maturin) serves its checks of ranges.
        "uses": ["ooxml-common", "ooxml-edit", "docx2svg", "ooxml-core", "docx-agent"],
        "siblings": ["ooxml-common", "ooxml-edit", "docx2svg[png]", "ooxml-core"],
        "extras": "dev,png",
        "min_python": "3.10",
        "pytest": ["-n", "auto", "--durations=30"],
        "compile": ["src", "tests", "tools", "examples"],
        "check_version": "tools/check_version.py",
        "mypy": False,
        "fonts": True,
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
        "check_version": "tools/check_version.py",
        "mypy": False,
    },
    "xlsx-agent": {
        "uses": ["ooxml-common", "ooxml-edit", "pptx2svg", "pptx-agent", "docx2svg", "ooxml-core", "docx-agent",
                 "xlsx-agent"],
        # Test only: pptx2svg and pptx-agent (a deck filled from a workbook's table),
        # docx2svg and docx-agent (a workbook's rich-text cell in a Word document), and
        # docx-agent's ooxml-core.
        "siblings": ["ooxml-common", "ooxml-edit", "pptx2svg", "pptx-agent", "docx2svg", "ooxml-core",
                     "docx-agent"],
        "extras": "test",
        "min_python": "3.11",
        "pytest": [],
        "compile": ["src", "tests"],
        "check_version": "tools/check_version.py",
        "mypy": True,
        "mypy_siblings": ["ooxml-common", "ooxml-edit"],
    },
}

IMPORT_NAMES = {name: name.replace("-", "_") for name in PACKAGES}

#: A change under one of these runs every package's tests.
RUN_ALL_PREFIXES = (".github/", "tools/")

#: The Rust workspace and its Python module: a change here runs rust.yml.  So does a change
#: to the packages the differential tests compare with or build their documents through
#: (python/ooxml-core/tests: docx2svg is the reference, docx-agent makes the documents).
RUST_PREFIXES = ("crates/", "Cargo.toml", "Cargo.lock", ".cargo/", "rustfmt.toml", "python/ooxml-core/")
RUST_REFERENCE = ("ooxml-common", "ooxml-edit", "docx2svg", "docx-agent")


def rust_affected(changed: list[str]) -> bool:
    for path in changed:
        if path.startswith(RUN_ALL_PREFIXES + RUST_PREFIXES):
            return True
        parts = path.split("/")
        if len(parts) > 2 and parts[0] == "python" and parts[1] in RUST_REFERENCE:
            return True
    return False


def affected(changed: list[str]) -> list[str]:
    """The packages whose tests a change to ``changed`` (paths from the root) selects."""
    touched: set[str] = set()
    for path in changed:
        if path.startswith(RUN_ALL_PREFIXES):
            return list(PACKAGES)
        if path.startswith(RUST_PREFIXES):
            touched.add("ooxml-core")       # the Rust core: the packages that install it
        parts = path.split("/")
        if len(parts) > 2 and parts[0] == "python" and parts[1] in PACKAGES:
            touched.add(parts[1])
    return [name for name, spec in PACKAGES.items() if touched & set(spec["uses"])]


def _pythons(name: str) -> list[str]:
    low = PACKAGES[name]["min_python"]
    return [v for v in PYTHONS if tuple(map(int, v.split("."))) >= tuple(map(int, low.split(".")))]


#: Which of a package's Pythons each OS runs, per event: "all", "ends" (the oldest and the
#: newest), "newest" or None (no legs).  macOS runners are few and shared by every run,
#: and development and local test runs already happen on macOS, so a pull request and a
#: push to main run no macOS leg; the nightly run and a manual run (FULL_LEGS, also any
#: event not named here) take every Python on Linux and Windows and the newest on macOS.
LEGS: dict[str, dict[str, str | None]] = {
    "pull_request": {"linux": "ends", "windows": "newest", "macos": None},
    "push": {"linux": "all", "windows": "ends", "macos": None},
}
FULL_LEGS: dict[str, str | None] = {"linux": "all", "windows": "all", "macos": "newest"}
RUNNERS = {"linux": "ubuntu-latest", "windows": "windows-latest", "macos": "macos-latest"}


def _select(versions: list[str], which: str | None) -> list[str]:
    if which is None:
        return []
    if which == "all":
        return versions
    if which == "newest":
        return versions[-1:]
    if which == "ends":
        return sorted({versions[0], versions[-1]}, key=versions.index)
    raise ValueError(which)


def plan(event: str, changed: list[str] | None) -> dict[str, str]:
    full = event != "pull_request"
    names = list(PACKAGES) if full or changed is None else affected(changed)
    legs = LEGS.get(event, FULL_LEGS)
    matrices: dict[str, list[dict[str, str | bool]]] = {os_: [] for os_ in RUNNERS}
    for name in names:
        versions = _pythons(name)
        for os_, runner in RUNNERS.items():
            for v in _select(versions, legs[os_]):
                leg = {"package": name, "os": runner, "python": v}
                if os_ == "linux" and v == versions[-1]:
                    # The coverage leg (PACKAGES' "coverage"): Linux, the newest Python,
                    # which every event runs.
                    leg["coverage"] = True
                matrices[os_].append(leg)
    out = {
        "packages": names,
        "any": bool(names),
        "rust": full or changed is None or rust_affected(changed),
        **{os_: {"include": legs_} for os_, legs_ in matrices.items()},
        # Whether ci.yml's macOS call runs at all: an empty matrix is an error in Actions.
        "any_macos": bool(matrices["macos"]),
        "build": {"include": [{"package": n} for n in names]},
    }
    return {k: json.dumps(v) if not isinstance(v, bool) else str(v).lower() for k, v in out.items()}


def _pip(*args: str) -> None:
    cmd = [sys.executable, "-m", "pip", "install", *args]
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, check=True)


def _req(path: str) -> str:
    """A pip requirement for ``path`` (under python/, maybe with [extras])."""
    base, _, extras = path.partition("[")
    return str(PY / base) + (f"[{extras}" if extras else "")


def install(name: str) -> None:
    spec = PACKAGES[name]
    _pip("--upgrade", "pip")
    args = [_req(s) for s in spec["siblings"]]
    for s in spec.get("editable_siblings", []):
        args += ["-e", _req(s)]
    args += ["-e", _req(f"{name}[{spec['extras']}]")]
    args += spec.get("pip_extra", [])
    # One resolve over all of them: every sibling requirement is met from this checkout.
    _pip(*args)


def test(name: str, event: str, coverage: bool = False) -> int:
    spec = PACKAGES[name]
    cwd = PY / name
    if "compile" in spec:
        # A docstring's invalid escape is a SyntaxWarning, printed once and easy to miss.
        subprocess.run(
            [sys.executable, "-W", "error::SyntaxWarning", "-W", "error::DeprecationWarning",
             "-m", "compileall", "-q", "-f", *spec["compile"]],
            cwd=cwd, check=True,
        )
    args = ["-m", "pytest", "-q", *spec["pytest"]]
    if event == "pull_request":
        args += spec.get("pr_pytest", [])
    env = dict(os.environ)
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
    if not code and spec.get("fixture_check"):
        code = fixture_check(cwd)
    return code


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


def build_check(name: str) -> None:
    spec = PACKAGES[name]
    cwd = PY / name
    subprocess.run([sys.executable, spec["check_version"]], cwd=cwd, check=True)
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
    p.add_argument("--changed", action="append", help="a changed path (repeat); default: git diff HEAD^1 HEAD")
    a = sub.add_parser("affected")
    a.add_argument("files", nargs="+")
    for command in ("install", "test", "build-check"):
        s = sub.add_parser(command)
        s.add_argument("package", choices=list(PACKAGES))
        if command == "test":
            s.add_argument("--event", default=os.environ.get("GITHUB_EVENT_NAME", "push"))
            s.add_argument("--coverage", action="store_true", help="the coverage leg, where the package has one")
    args = parser.parse_args(argv)

    if args.command == "affected":
        print("\n".join(affected(args.files)))
        return 0
    if args.command == "plan":
        changed = args.changed
        if changed is None and args.event == "pull_request":
            # A pull request's checkout is its merge commit: the first parent is the base.
            diff = subprocess.run(["git", "diff", "--name-only", "HEAD^1", "HEAD"], cwd=ROOT,
                                  capture_output=True, text=True, check=True)
            changed = [line for line in diff.stdout.splitlines() if line]
            print(f"{len(changed)} changed files", file=sys.stderr)
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
        install(args.package)
        return 0
    if args.command == "test":
        return test(args.package, args.event, args.coverage)
    if args.command == "build-check":
        build_check(args.package)
        return 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
