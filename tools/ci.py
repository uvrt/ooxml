"""What CI runs for the Python packages, in one place.

The workflows (.github/workflows/ci.yml, freethreading.yml, release.yml) call this script
rather than spell out each package's installs and test command, so the seven packages'
differences live here and nowhere else:

    python tools/ci.py plan [--event EVENT] [--changed FILE]...
        Which packages to test, as GitHub Actions matrices (written to $GITHUB_OUTPUT when it
        is set, printed otherwise), and whether rust.yml's jobs run (`rust`).  On a pull request only the packages a change touches, and
        every package that depends on one of them (USES below); a change to the workflows or
        to tools/ runs everything.  A push to main, the nightly run and a manual run take all.
    python tools/ci.py affected FILE...
        The packages those changed files select, one per line (for a quick look by hand).
    python tools/ci.py install PACKAGE [--free-threading]
        pip-installs the package (editable, with its test extras) and the siblings it needs,
        all from this checkout -- never from the old repositories.
    python tools/ci.py test PACKAGE [--event EVENT] [--free-threading]
        Runs the package's suite as its own repository's CI ran it.
    python tools/ci.py gil-check PACKAGE
        On a free-threaded Python: importing the package, its modules and its extras' modules
        leaves the GIL off.
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
        # One leg (ubuntu, 3.12) runs the suite under coverage instead, with this floor: the
        # suite is meant to hold the package by itself, not through the renderers' suites.
        "coverage": "87",
        "gil_imports": "import fontTools.ttLib",
    },
    "ooxml-edit": {
        "uses": ["ooxml-common", "ooxml-edit"],
        "siblings": ["ooxml-common"],
        "extras": "dev",
        "min_python": "3.10",
        "pytest": [],
        "ft_pytest": ["-v", "-o", "faulthandler_timeout=300"],
        "check_version": ".github/check_version.py",
        "mypy": True,
        "mypy_siblings": ["ooxml-common"],
        "lxml7": True,
        "gil_imports": "import lxml.etree, lxml.html",
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
        "ft_pytest": [],
        "check_version": "tools/check_version.py",
        "mypy": True,
        "mypy_siblings": ["ooxml-common"],
        "fonts": True,
        # The committed fixture is regenerable and current.
        "fixture_check": True,
        "gil_imports": "import resvg_py, pypdfium2, fontTools.ttLib, numpy",
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
        "gil_imports": "import resvg_py, pypdfium2, fontTools.ttLib, PIL.Image, numpy",
    },
    "docx-agent": {
        "uses": ["ooxml-common", "ooxml-edit", "docx2svg", "docx-agent"],
        "siblings": ["ooxml-common", "ooxml-edit", "docx2svg[png]"],
        "extras": "dev,png",
        "min_python": "3.10",
        "pytest": ["-n", "auto", "--durations=30"],
        "ft_pytest": ["-n", "auto", "-o", "faulthandler_timeout=300", "-v"],
        "compile": ["src", "tests", "tools", "examples"],
        "check_version": "tools/check_version.py",
        "mypy": False,
        "fonts": True,
        "lxml7": True,
        "gil_imports": "import lxml.etree, lxml.html, docx2svg, resvg_py",
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
        "check_version": "tools/check_version.py",
        "mypy": False,
        "lxml7": True,
        "gil_imports": "import lxml.etree, lxml.html, pptx2svg, resvg_py, PIL.Image",
    },
    "xlsx-agent": {
        "uses": ["ooxml-common", "ooxml-edit", "pptx2svg", "pptx-agent", "docx2svg", "docx-agent", "xlsx-agent"],
        # Test only: pptx2svg and pptx-agent (a deck filled from a workbook's table),
        # docx2svg and docx-agent (a workbook's rich-text cell in a Word document).
        "siblings": ["ooxml-common", "ooxml-edit", "pptx2svg", "pptx-agent", "docx2svg", "docx-agent"],
        "extras": "test",
        "min_python": "3.11",
        "pytest": [],
        "ft_pytest": [],
        "compile": ["src", "tests"],
        "check_version": "tools/check_version.py",
        "mypy": True,
        "mypy_siblings": ["ooxml-common", "ooxml-edit"],
        "lxml7": True,
        "gil_imports": "import lxml.etree, lxml.html, pptx_agent, pptx2svg",
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
        parts = path.split("/")
        if len(parts) > 2 and parts[0] == "python" and parts[1] in PACKAGES:
            touched.add(parts[1])
    return [name for name, spec in PACKAGES.items() if touched & set(spec["uses"])]


def _pythons(name: str) -> list[str]:
    low = PACKAGES[name]["min_python"]
    return [v for v in PYTHONS if tuple(map(int, v.split("."))) >= tuple(map(int, low.split(".")))]


def plan(event: str, changed: list[str] | None) -> dict[str, str]:
    full = event != "pull_request"
    names = list(PACKAGES) if full or changed is None else affected(changed)
    linux, windows, macos, ft_linux, ft_macos = [], [], [], [], []
    for name in names:
        versions = _pythons(name)
        for v in versions:
            linux.append({"package": name, "os": "ubuntu-latest", "python": v})
            windows.append({"package": name, "os": "windows-latest", "python": v})
            # On a pull request macOS runs only the oldest and the newest Python: its
            # runners are the scarce ones.  Everything else takes the whole matrix.
            if full or v in (versions[0], versions[-1]):
                macos.append({"package": name, "os": "macos-latest", "python": v})
        ft_linux.append({"package": name, "os": "ubuntu-latest", "python": "3.15t"})
        if full:
            ft_macos.append({"package": name, "os": "macos-latest", "python": "3.15t"})
    out = {
        "packages": names,
        "any": bool(names),
        "rust": full or changed is None or rust_affected(changed),
        "linux": {"include": linux},
        "windows": {"include": windows},
        "macos": {"include": macos},
        "ft": {"include": ft_linux + ft_macos},
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


def test(name: str, event: str, free_threading: bool, coverage: bool = False) -> int:
    spec = PACKAGES[name]
    cwd = PY / name
    if "compile" in spec and not free_threading:
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
    if not code and spec.get("fixture_check") and not free_threading:
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


def gil_check(name: str) -> int:
    spec = PACKAGES[name]
    code = f"""
import importlib, pkgutil, sys
assert hasattr(sys, "_is_gil_enabled"), "not a free-threaded build"
import {IMPORT_NAMES[name]} as package
for module in pkgutil.walk_packages(package.__path__, package.__name__ + "."):
    if not module.name.endswith("__main__"):
        importlib.import_module(module.name)
{spec['gil_imports']}  # the extras' modules
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
    for command in ("install", "test", "gil-check", "build-check"):
        s = sub.add_parser(command)
        s.add_argument("package", choices=list(PACKAGES))
        if command in ("install", "test"):
            s.add_argument("--free-threading", action="store_true")
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
        install(args.package, args.free_threading)
        return 0
    if args.command == "test":
        return test(args.package, args.event, args.free_threading, args.coverage)
    if args.command == "gil-check":
        return gil_check(args.package)
    if args.command == "build-check":
        build_check(args.package)
        return 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
