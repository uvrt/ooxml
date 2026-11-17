"""What CI runs for the Python packages, in one place.

The workflows (.github/workflows/ci.yml, freethreading.yml, release.yml) call this script
rather than spell out each package's installs and test command, so the seven packages'
differences live here and nowhere else:

    python tools/ci.py plan [--event EVENT] [--changed FILE]...
        Which packages to test, as GitHub Actions matrices (written to $GITHUB_OUTPUT when it
        is set, printed otherwise), and whether rust.yml's jobs run (`rust`, and `rust_<job>`
        for its path-filtered jobs, RUST_JOBS).  On a pull request only the packages a change touches, and
        every package that depends on one of them (USES below); a change to the workflows or
        to tools/ runs everything.  A push to main, the nightly run and a manual run take all.
        Which Pythons each OS runs depends on the event (LEGS below): production runs
        CPython 3.15 (moving to 3.15t), development runs on macOS and macOS runners are
        scarce, so a pull request and a push to main run Linux and Windows on the newest
        Python and no macOS; the nightly and a manual run add every older Python on Linux
        and the newest on macOS.
        The free-threaded legs (`ft`, freethreading.yml) run on Linux only, every event.
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
    python tools/ci.py fuzz-seeds DIR
        rust.yml's fuzz job's seed corpora from this checkout's documents (FUZZ_SEED_SUFFIXES
        under python/): DIR/package (each package, for zip_read and opc_open) and DIR/xml
        (each XML part, for xml_roundtrip) and DIR/metafile (the metafiles in ooxml-oracle's
        recorded case bundle, for metafile_draw), every distinct file once, none over 1 MiB.

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
        # ooxml-core (the Rust core, built here by maturin) serves its checks of ranges.
        "uses": ["ooxml-common", "ooxml-edit", "docx2svg", "ooxml-core", "docx-agent"],
        "siblings": ["ooxml-common", "ooxml-edit", "docx2svg[png]", "ooxml-core"],
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
#: RUST_MODULE_PREFIXES are what the Python module ooxml-core is built from, so a change there
#: also runs the suites of the packages that install it (affected); the lints, the dependency
#: policy, the fuzz targets and xtask's code generation run rust.yml only.
RUST_MODULE_PREFIXES = ("crates/", "Cargo.toml", "Cargo.lock", ".cargo/", "python/ooxml-core/")
RUST_PREFIXES = RUST_MODULE_PREFIXES + ("rustfmt.toml", "clippy.toml", "deny.toml", "DEPENDENCIES.md", "fuzz/",
                                        "xtask/")
RUST_REFERENCE = ("ooxml-common", "ooxml-edit", "docx2svg", "docx-agent")

#: The workspace's own files: a change to one rebuilds every crate (and fuzz/, whose crates
#: inherit the workspace's dependencies), so it runs every job of RUST_JOBS.
RUST_WORKSPACE = ("Cargo.toml", "Cargo.lock", ".cargo/")

#: The architecture's crates a job of RUST_JOBS builds: the crate it runs and every
#: workspace crate that one depends on (tools/test_ci.py checks them against Cargo.toml).
FUZZ_CRATES = ("ooxml-base", "ooxml-xml", "ooxml-zip", "ooxml-opc")
LOOM_CRATES = ("ooxml-store",)
WORD_MODEL_CRATES = ("ooxml-oracle", "ooxml-base", "ooxml-xml", "ooxml-zip", "ooxml-opc", "ooxml-derive",
                     "ooxml-fonts", "ooxml-text", "ooxml-word-model", "ooxml-colour", "ooxml-schema",
                     "ooxml-drawingml", "ooxml-scene", "ooxml-metafile", "ooxml-word-layout", "ooxml-numfmt",
                     "ooxml-chart", "ooxml-render", "ooxml-tools", "ooxml-tools-derive", "ooxml-tools-word",
                      "ooxml-raster", "ooxml-style-names", "ooxml-word-pages", "ooxml-session", "ooxml-store")


def _crates(names: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(f"crates/{name}/" for name in names)


#: rust.yml's jobs that run on a pull request only when it changes their own paths (or
#: RUN_ALL_PREFIXES or RUST_WORKSPACE), and the other events that run them.  rust.yml's
#: other jobs (fmt, clippy, tests, differential, wheels) follow `rust` above.
#: - fuzz: fuzz/'s targets, 5 minutes each; nightly and manual runs, not a push to main.
#: - loom: ooxml-store's commit path, model-checked.
#: - oracles: tools/rust_oracles.py -- the XML writer and the package save against ooxml-edit.
#: - word_model: the Word model against docx2svg's layout input
#:   (crates/ooxml-oracle/python/word_model_diff.py); xtask generates its typed views.
#: - deny: cargo-deny (deny.toml) when a Cargo file or the dependency policy changes.
FULL_EVENTS = ("push", "schedule", "workflow_dispatch")
RUST_JOBS: dict[str, dict] = {
    "fuzz": {"paths": ("fuzz/", *_crates(FUZZ_CRATES)), "events": ("schedule", "workflow_dispatch")},
    "loom": {"paths": _crates(LOOM_CRATES), "events": FULL_EVENTS},
    "oracles": {"paths": ("python/ooxml-edit/", "python/ooxml-common/", *_crates(FUZZ_CRATES)),
                "events": FULL_EVENTS},
    "word_model": {"paths": ("xtask/", "python/docx2svg/", "python/ooxml-common/", "python/pptx2svg/", *_crates(WORD_MODEL_CRATES)),
                   "events": FULL_EVENTS},
    "deny": {"paths": ("deny.toml", "DEPENDENCIES.md"), "suffixes": ("Cargo.toml", "Cargo.lock"),
             "events": FULL_EVENTS},
}


def rust_affected(changed: list[str]) -> bool:
    for path in changed:
        if path.startswith(RUN_ALL_PREFIXES + RUST_PREFIXES):
            return True
        parts = path.split("/")
        if len(parts) > 2 and parts[0] == "python" and parts[1] in RUST_REFERENCE:
            return True
    return False


def rust_job_runs(job: str, event: str, changed: list[str] | None) -> bool:
    """Whether rust.yml's ``job`` (RUST_JOBS) runs on ``event``, a pull request changing ``changed``."""
    spec = RUST_JOBS[job]
    if event != "pull_request":
        return event in spec["events"]
    if changed is None:
        return True
    for path in changed:
        if path.startswith(RUN_ALL_PREFIXES + RUST_WORKSPACE + spec["paths"]):
            return True
        if path.endswith(spec.get("suffixes", ())):
            return True
    return False


def affected(changed: list[str]) -> list[str]:
    """The packages whose tests a change to ``changed`` (paths from the root) selects."""
    touched: set[str] = set()
    for path in changed:
        if path.startswith(RUN_ALL_PREFIXES):
            return list(PACKAGES)
        if path.startswith(RUST_MODULE_PREFIXES):
            touched.add("ooxml-core")       # the Rust core: the packages that install it
        parts = path.split("/")
        if len(parts) > 2 and parts[0] == "python" and parts[1] in PACKAGES:
            touched.add(parts[1])
    return [name for name, spec in PACKAGES.items() if touched & set(spec["uses"])]


def _pythons(name: str) -> list[str]:
    low = PACKAGES[name]["min_python"]
    return [v for v in PYTHONS if tuple(map(int, v.split("."))) >= tuple(map(int, low.split(".")))]


#: Which of a package's Pythons each OS runs, per event: "all" (from the package's
#: min_python to the newest), "newest" or None (no legs).  Production runs CPython 3.15 and
#: is moving to 3.15t (the `ft` legs, Linux, every event); development and local test runs
#: happen on macOS, whose runners are few and shared by every run.  So a pull request and a
#: push to main run Linux and Windows on the newest Python and no macOS; the nightly run
#: and a manual run (FULL_LEGS, also any event not named here) run every Python on Linux,
#: so the older ones requires-python still allows keep working, and the newest on Windows
#: and macOS.
LEGS: dict[str, dict[str, str | None]] = {
    "pull_request": {"linux": "newest", "windows": "newest", "macos": None},
    "push": {"linux": "newest", "windows": "newest", "macos": None},
}
FULL_LEGS: dict[str, str | None] = {"linux": "all", "windows": "newest", "macos": "newest"}
RUNNERS = {"linux": "ubuntu-latest", "windows": "windows-latest", "macos": "macos-latest"}


def _select(versions: list[str], which: str | None) -> list[str]:
    if which is None:
        return []
    if which == "all":
        return versions
    if which == "newest":
        return versions[-1:]
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
        # rust.yml's path-filtered jobs (RUST_JOBS): rust_fuzz, rust_loom, ...
        **{f"rust_{job}": rust_job_runs(job, event, changed) for job in RUST_JOBS},
        **{os_: {"include": legs_} for os_, legs_ in matrices.items()},
        # The free-threaded legs (freethreading.yml): Linux only, on every event.
        "ft": {"include": [{"package": n, "os": RUNNERS["linux"], "python": "3.15t"} for n in names]},
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


#: The documents fuzz-seeds takes, and the parts of them it takes for the XML target (as
#: tools/rust_oracles.py does), up to libFuzzer's -max_len in rust.yml.
FUZZ_SEED_SUFFIXES = (".docx", ".docm", ".dotx", ".dotm", ".pptx", ".pptm", ".potx", ".xlsx", ".xlsm", ".xltx")
FUZZ_XML_SUFFIXES = (".xml", ".rels", ".vml")
FUZZ_MAX_LEN = 1 << 20
METAFILE_CASES = ROOT / "crates" / "ooxml-oracle" / "tests" / "data" / "metafile-cases.gz"


def fuzz_seeds(out: Path) -> dict[str, int]:
    import hashlib
    import zipfile

    counts = {"package": 0, "xml": 0, "metafile": 0}
    seen: set[str] = set()

    def keep(kind: str, data: bytes) -> None:
        digest = hashlib.sha256(data).hexdigest()
        if len(data) <= FUZZ_MAX_LEN and digest not in seen:
            seen.add(digest)
            (out / kind / digest[:20]).write_bytes(data)
            counts[kind] += 1

    for kind in counts:
        (out / kind).mkdir(parents=True, exist_ok=True)
    for path in sorted(PY.rglob("*")):
        if path.suffix.lower() not in FUZZ_SEED_SUFFIXES or not path.is_file() or ".venv" in path.parts:
            continue
        data = path.read_bytes()
        keep("package", data)
        try:
            with zipfile.ZipFile(path) as archive:
                for info in archive.infolist():
                    if info.filename.lower().endswith(FUZZ_XML_SUFFIXES) and info.file_size <= FUZZ_MAX_LEN:
                        keep("xml", archive.read(info))
        except (zipfile.BadZipFile, OSError, RuntimeError, NotImplementedError):
            continue  # a deliberately damaged document: a package seed only
    for data in metafile_cases():
        keep("metafile", data)
    return counts


def metafile_cases(path: Path = METAFILE_CASES) -> list[bytes]:
    """The metafiles in ooxml-oracle's recorded case bundle (the repository has no .emf or
    .wmf files; its tests build them): a gzip stream of length-prefixed cases, each a list
    of length-prefixed fields, field 0 the kind and field 1 a metafile's bytes."""
    import gzip

    def fields(data: bytes) -> list[bytes]:
        out, at = [], 0
        while at + 4 <= len(data):
            n = int.from_bytes(data[at:at + 4], "little")
            out.append(data[at + 4:at + 4 + n])
            at += 4 + n
        return out

    if not path.is_file():
        return []
    blob = gzip.decompress(path.read_bytes())
    found, at = [], 0
    while at + 4 <= len(blob):
        n = int.from_bytes(blob[at:at + 4], "little")
        case = fields(blob[at + 4:at + 4 + n])
        if len(case) > 1 and case[0] == b"metafile":
            found.append(case[1])
        at += 4 + n
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("plan")
    p.add_argument("--event", default=os.environ.get("GITHUB_EVENT_NAME", "pull_request"))
    p.add_argument("--changed", action="append", help="a changed path (repeat); default: git diff HEAD^1 HEAD")
    a = sub.add_parser("affected")
    a.add_argument("files", nargs="+")
    f = sub.add_parser("fuzz-seeds")
    f.add_argument("out", type=Path)
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
    if args.command == "fuzz-seeds":
        print(json.dumps(fuzz_seeds(args.out)))
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
