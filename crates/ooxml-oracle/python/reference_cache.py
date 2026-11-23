"""docx-agent's results kept between runs: the differential gates' reference cache.

docx-agent is the reference the Rust core is compared with, and it is frozen (pure Python,
deleted at P4e), so a case's expected result is the same in every run until something it
depends on changes.  A result is kept under a key made of everything it depends on:

* the **chain**: the code the reference runs.  Frozen (reference_packages.py: the gates
  install docx-agent and its packages from the commit crates/ooxml-oracle/reference.toml
  names), that commit, the harness modules but NOT_REFERENCE, the Python version and every
  installed distribution (pip excepted: the workflow upgrades it, and the reference never
  imports it) -- so a kept result stays valid whatever the checkout's ``python/`` holds.  Not
  frozen (a run by hand), instead of the commit every git-tracked file of each package folder
  a harness module was imported from (``python/<package>/src``: its modules and the data they
  read);
* the **environment**: the variables the reference code names (any quoted upper-case name
  in its Python files -- the harness's and the reference packages', installed or not -- that
  is set) and the font files docx2svg measures with (paths, sizes);
* the **case**: its document's bytes, its spec as both sides are given it, and the bytes of
  a ``reference_file`` it names.

The chain is derived, not listed: it is read from ``sys.modules`` once the harness has been
imported, and :meth:`Store.close` fails when a module the run imported later lies in the
repository outside it (the installed reference lies outside the repository, its commit
keyed instead).

NOT_REFERENCE are the harness modules that make the cases, run the Rust side and compare the
results: the harnesses' drivers (word_edits_diff.py, and word_reads_diff.py, whose run and
report it shares), the case makers (every word_*_cases.py), word_edits_compare.py and what
records the cases and answers for goldens (goldens.py) and runs the Rust side (oracle_run.py).  The
case they make is in each result's key, and a comparison runs again on every run, kept result
or not, so a change to them (a new case, a comparison rule, the Rust side's run) leaves every
kept result valid: only the new cases are computed.  The reference's own code -- what runs
docx-agent on a case and reads its result (word_edits_reference.py, word_tool_reference.py),
this module, and what chooses and guards the reference's packages (reference_packages.py,
reference_guard.py) -- stays in the chain, and a change to it computes every case again.
:meth:`Store.compute` holds NOT_REFERENCE to that: it watches (``sys.monitoring``) for any of
their functions running while docx-agent computes a result, and :meth:`Store.close` then
fails, naming it -- that code belongs in the reference (word_edits_reference.py,
word_tool_reference.py).

A package's modules are keyed with all of their package's tracked files rather than with the
modules imported alone: the reference imports nearly every one of them (docx-agent's
``rust`` modules too: tools/_base.py imports rust/edits.py, whose ``owned_entry`` runs for
every edit with Rust off), many of them only once a case runs, so the folder is the smallest
set known before the run.

What is kept is what the comparison reads, with each changed part's text replaced by its
digest (:func:`compact`); the harness compares a kept result with the Rust core's in that
form and runs docx-agent again for a case whose comparison finds a difference, so the log
says as much as it does without the cache.  A result is kept only when it reads back from
JSON exactly as it was computed.

A store is a folder of ``*.jsonl.gz`` files (one entry a line: ``{"key", "value"}``); a run
writes the entries it computed to its own file (``--cache-new``) and :func:`merge` adds
those to a store.  One store holds every mode's results (fixtures and corpus), so a run
keeps in memory only the entries of its own cases (:meth:`Store.read`): the whole store held
as Python objects made every full collection of the cyclic garbage collector walk it, case
after case, and a structure gate job ran three times as long with the store as without it
(lane PF1).
"""

from __future__ import annotations

import gzip
import hashlib
import importlib.metadata
import json
import os
import re
import subprocess
import sys
from collections.abc import Callable
from pathlib import Path

from reference_packages import check_frozen, installed_sources

ROOT = Path(__file__).resolve().parents[3]
HARNESS = Path(__file__).resolve().parent
#: Installed but not run by the reference (and upgraded on every run by the workflow).
NOT_REFERENCE_DISTRIBUTIONS = {"pip"}
#: The harness modules that make the cases, run the Rust side and compare the results, not
#: part of the chain (see the module's docstring): the drivers, word_edits_compare.py and every
#: case maker (word_*_cases.py, found by name so that a new one is left out too).
DRIVERS = ("word_edits_diff.py", "word_reads_diff.py", "word_edits_compare.py")
#: What records a run's cases and the Rust side's answers for goldens (the drivers' --record).
RECORDING = ("goldens.py", "oracle_run.py")
NOT_REFERENCE = (*DRIVERS, *RECORDING, *sorted(path.name for path in HARNESS.glob("word_*_cases.py")))
DIGEST = "sha256:"
#: The name the merged store's file has (other files in a store are merged into it).
MERGED = "reference.jsonl.gz"


def _sha(*chunks: bytes) -> str:
    h = hashlib.sha256()
    for chunk in chunks:
        h.update(len(chunk).to_bytes(8, "little"))
        h.update(chunk)
    return h.hexdigest()


def _repository_modules() -> list[Path]:
    """The files of the modules imported so far that lie in the repository."""
    out = []
    for module in list(sys.modules.values()):
        file = getattr(module, "__file__", None)
        if file and Path(file).resolve().is_relative_to(ROOT):
            out.append(Path(file).resolve())
    return out


def _tracked(folder: Path) -> list[Path]:
    listed = subprocess.run(["git", "ls-files", "-z", "--", str(folder.relative_to(ROOT))], cwd=ROOT,
                            capture_output=True, check=True).stdout.decode()
    return [ROOT / name for name in listed.split("\0") if name]


def _not_reference() -> set[Path]:
    return {HARNESS / name for name in NOT_REFERENCE}


def reference_files() -> list[Path]:
    """The repository files the reference runs: each harness module imported but
    NOT_REFERENCE, and every tracked file of the package folders (the ``sys.path`` entries)
    the other imported modules came from."""
    entries = sorted({Path(p).resolve() for p in sys.path if p and Path(p).resolve().is_relative_to(ROOT)},
                     key=lambda p: len(p.parts), reverse=True)
    files: set[Path] = set()
    packages: set[Path] = set()
    for module in _repository_modules():
        entry = next((e for e in entries if module.is_relative_to(e)), None)
        if entry is None or entry == HARNESS:
            if module not in _not_reference():
                files.add(module)
        else:
            packages.add(entry)
    for entry in packages:
        files.update(_tracked(entry))
    return sorted(files)


def chain() -> tuple[str, list[Path]]:
    """The chain's digest and the files it covers (see the module's docstring)."""
    frozen = check_frozen()
    files = reference_files()
    distributions = sorted({f"{d.metadata['Name']}=={d.version}" for d in importlib.metadata.distributions()
                            if (d.metadata["Name"] or "").lower() not in NOT_REFERENCE_DISTRIBUTIONS})
    chunks = [sys.version.encode(), "\n".join(distributions).encode()]
    if frozen is not None:
        chunks.insert(0, f"frozen reference {frozen}".encode())
    for file in files:
        chunks += [str(file.relative_to(ROOT)).encode(), file.read_bytes() if file.exists() else b"(deleted)"]
    return _sha(*chunks), files


def environment(files: list[Path]) -> str:
    """The digest of the variables the reference code names and the fonts it can measure."""
    names: set[str] = set()
    for file in [*files, *installed_sources()]:
        if file.suffix == ".py" and file.exists():
            names.update(re.findall(r"""["']([A-Z][A-Z0-9_]{2,})["']""", file.read_text("utf-8", "replace")))
    variables = {name: os.environ[name] for name in sorted(names) if name in os.environ}
    from docx2svg.fonts import default_font_dirs
    from ooxml_common.fonts.office import user_font_dirs

    fonts = []
    for folder in [*user_font_dirs(), *default_font_dirs()]:
        for path in sorted(Path(folder).rglob("*")) if Path(folder).is_dir() else ():
            if path.is_file():
                fonts.append(f"{path}\t{path.stat().st_size}")
    return _sha(json.dumps(variables, sort_keys=True).encode(), "\n".join(fonts).encode())


def compact(result):
    """``result`` with each changed part's text replaced by its digest (both sides alike)."""
    if isinstance(result, dict) and isinstance(result.get("parts"), dict):
        result["parts"] = {name: DIGEST + hashlib.sha256(text.encode()).hexdigest()
                           if isinstance(text, str) else text for name, text in result["parts"].items()}
    return result


def _read(path: Path):
    with gzip.open(path, "rt", encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                yield json.loads(line)


def kept_entries(folder: Path | None, wanted: set[str]) -> dict[str, dict]:
    """The results of the store in ``folder`` under the keys ``wanted``: each other entry is
    let go as soon as it is read (:meth:`Store.read`)."""
    found: dict[str, dict] = {}
    for path in sorted(folder.glob("*.jsonl.gz")) if folder and folder.is_dir() else ():
        for entry in _read(path):
            if entry["key"] in wanted:
                found[entry["key"]] = entry["value"]
    return found


def _write(path: Path, entries: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with gzip.open(path, "wt", encoding="utf-8", compresslevel=6) as handle:
        for key, value in entries.items():
            handle.write(json.dumps({"key": key, "value": value}) + "\n")


class _Watch:
    """Which functions of the NOT_REFERENCE modules start while :meth:`watching`: a
    ``sys.monitoring`` tool on PY_START that turns itself off for each code object once
    seen, so a run pays for it about once per function."""

    def __init__(self) -> None:
        monitoring = getattr(sys, "monitoring", None)
        if monitoring is None:
            raise SystemExit("reference cache: needs sys.monitoring (Python 3.12 or later) to check that no case "
                             "maker runs inside the reference (reference_cache.py)")
        self.monitoring = monitoring
        self.files = {str(path) for path in _not_reference()}
        self.seen: set[str] = set()
        free = [tool for tool in range(6) if monitoring.get_tool(tool) is None]
        if not free:
            raise SystemExit("reference cache: no free sys.monitoring tool id")
        self.tool = free[-1]
        monitoring.use_tool_id(self.tool, "reference cache")
        monitoring.register_callback(self.tool, monitoring.events.PY_START, self._started)

    def _started(self, code, offset):  # noqa: ARG002 -- sys.monitoring's callback signature
        if os.path.realpath(code.co_filename) in self.files:
            self.seen.add(f"{Path(code.co_filename).name}: {code.co_qualname}")
        return self.monitoring.DISABLE

    def watching(self, run: Callable, *args):
        self.monitoring.set_events(self.tool, self.monitoring.events.PY_START)
        try:
            return run(*args)
        finally:
            self.monitoring.set_events(self.tool, 0)

    def close(self) -> None:
        self.monitoring.register_callback(self.tool, self.monitoring.events.PY_START, None)
        self.monitoring.free_tool_id(self.tool)


class Store:
    """The kept results a harness run reads (``folder``) and the ones it adds (``new``)."""

    def __init__(self, folder: str | None, new: str | None, same):
        self.watch = _Watch()
        self.chain, self.files = chain()
        self.prefix = _sha(self.chain.encode(), environment(self.files).encode())
        self.new_path = Path(new) if new else None
        self.same = same            # (a, b) -> True when a and b compare equal (the harness's differences)
        self.folder = Path(folder) if folder else None
        self.kept: dict[str, dict] = {}
        self.new: dict[str, dict] = {}
        self.hits = 0

    def read(self, wanted: set[str]) -> None:
        """Keeps the store's results under the keys ``wanted`` (the run's cases', :meth:`key`)
        and no others: what :meth:`get` finds.  Every other entry is let go as it is read, so
        the run holds its own results only, whatever the store's size (the module's
        docstring)."""
        self.kept.update(kept_entries(self.folder, wanted))

    def key(self, data: bytes, case: dict) -> str:
        other = case.get("reference_file")
        return _sha(self.prefix.encode(), hashlib.sha256(data).digest(), json.dumps(case, sort_keys=True).encode(),
                    Path(other).read_bytes() if other else b"")

    def get(self, key: str) -> dict | None:
        """The kept result under ``key`` (a case's, :meth:`key`), where :meth:`read` kept one."""
        found = self.kept.get(key)
        if found is not None:
            self.hits += 1
        return found

    def compute(self, python_case: Callable[..., dict], data: bytes, case: dict, *rest) -> dict:
        """``python_case(data, case, *rest)``, docx-agent's result, kept; checked to run no
        NOT_REFERENCE code (:meth:`close` fails if it did)."""
        result = self.watch.watching(python_case, data, case, *rest)
        self.keep(data, case, result)
        return result

    def keep(self, data: bytes, case: dict, result: dict) -> None:
        try:
            again = json.loads(json.dumps(result))
        except (TypeError, ValueError):
            return              # not JSON: computed every run
        if self.same(result, again):
            self.new[self.key(data, case)] = compact(again)

    def close(self) -> None:
        """Checks that the run imported nothing the chain does not cover and that no
        NOT_REFERENCE code ran inside the reference, then writes the entries computed."""
        self.watch.close()
        covered = set(self.files) | _not_reference()
        outside = sorted(str(f.relative_to(ROOT)) for f in _repository_modules() if f not in covered)
        if outside:
            raise SystemExit(f"reference cache: the run imported {outside}, which its key does not cover "
                             "(crates/ooxml-oracle/python/reference_cache.py)")
        if self.watch.seen:
            raise SystemExit(f"reference cache: docx-agent's results ran {sorted(self.watch.seen)}, which the key "
                             "leaves out (NOT_REFERENCE): move that code into the reference "
                             "(crates/ooxml-oracle/python/reference_cache.py)")
        print(f"reference cache: {self.hits} kept results used, {len(self.new)} computed and kept"
              f"{'' if self.new_path else ' (not written)'}")
        if self.new_path:
            _write(self.new_path, self.new)


def merge(store: str, files: list[str]) -> int:
    """Adds ``files``' entries to the store in ``store`` (one file, :data:`MERGED`); the
    number of entries added."""
    folder = Path(store)
    entries: dict[str, dict] = {}
    for path in sorted(folder.glob("*.jsonl.gz")) if folder.is_dir() else ():
        entries.update((e["key"], e["value"]) for e in _read(path))
    before = len(entries)
    for path in files:
        entries.update((e["key"], e["value"]) for e in _read(Path(path)))
    for path in folder.glob("*.jsonl.gz") if folder.is_dir() else ():
        path.unlink()
    _write(folder / MERGED, entries)
    return len(entries) - before


if __name__ == "__main__":
    # The chain's digest as the edits harness computes it: once its modules are imported
    # (the workflow's cache key; tools/gates.py cache-key).
    import word_edits_diff  # noqa: F401

    print(chain()[0])
