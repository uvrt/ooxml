"""Where the harnesses import the gates' reference packages from: the frozen reference, or the checkout.

The reference is docx-agent in Python with the packages it runs (FROZEN).  Its Python is frozen
at P4e: ``crates/ooxml-oracle/reference.toml`` names the commit (``sha``) whose packages the gates
install -- .github/actions/gates-reference and local-gates.sh install them from ``git archive`` of
that commit -- and both set :data:`ENV` to that commit.  With :data:`ENV` set, :func:`use_sources`
leaves the checkout's copies of the FROZEN packages off ``sys.path``, so a harness imports the
installed ones, whatever the checkout's ``python/`` holds (P4e deletes most of it).  Without it
(the Rust workflow's differentials, a run by hand) each package's ``python/<package>/src`` comes
first, as before the freeze.

:func:`check_frozen` (the reference cache's chain calls it) fails a frozen run that imported a
FROZEN package from the checkout anyway (a ``PYTHONPATH`` left over, say), and one whose
:data:`ENV` is not the commit reference.toml names.
"""

from __future__ import annotations

import importlib.util
import os
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
#: The variable naming the frozen reference's commit (set by the gates' install).
ENV = "GATES_REFERENCE_SHA"
#: The file naming the commit the reference is frozen at.
REFERENCE_TOML = Path(__file__).resolve().parents[1] / "reference.toml"
#: The reference's packages (folders below ``python/``) and their modules.
FROZEN = {"docx-agent": "docx_agent", "docx2svg": "docx2svg", "ooxml-edit": "ooxml_edit",
          "ooxml-common": "ooxml_common"}


def pinned_sha(path: Path = REFERENCE_TOML) -> str | None:
    """The commit reference.toml freezes the reference at, or None without the file."""
    if not path.is_file():
        return None
    sha = tomllib.loads(path.read_text()).get("sha")
    if not isinstance(sha, str) or len(sha) != 40 or any(c not in "0123456789abcdef" for c in sha):
        raise SystemExit(f"{path}: sha must be a full commit id (40 lower-case hex digits): {sha!r}")
    return sha


def frozen_sha() -> str | None:
    """The frozen reference's commit when this run uses it (:data:`ENV` set), else None."""
    sha = os.environ.get(ENV) or None
    if sha is not None and sha != pinned_sha():
        raise SystemExit(f"{ENV}={sha} is not the commit {REFERENCE_TOML.relative_to(ROOT)} names "
                         f"({pinned_sha()}): install that reference (crates/ooxml-oracle/python/reference_packages.py)")
    return sha


def use_sources(*packages: str) -> None:
    """Puts ``python/<package>/src`` of each of ``packages`` first on ``sys.path``, but not a
    FROZEN one's while the run uses the frozen reference (its installed copy is imported)."""
    frozen = frozen_sha() is not None
    for package in packages:
        if frozen and package in FROZEN:
            continue
        source = str(ROOT / "python" / package / "src")
        if source not in sys.path:
            sys.path.insert(0, source)


def checkout_imports() -> list[str]:
    """The FROZEN packages' modules this process imported from the checkout."""
    roots = [ROOT / "python" / package for package in FROZEN]
    found = []
    for name, module in list(sys.modules.items()):
        file = getattr(module, "__file__", None)
        if name.split(".")[0] in FROZEN.values() and file and any(Path(file).resolve().is_relative_to(r)
                                                                   for r in roots):
            found.append(name)
    return sorted(found)


def check_frozen() -> str | None:
    """The frozen reference's commit (None when the run does not use it); fails when a FROZEN
    package was imported from the checkout in a frozen run."""
    sha = frozen_sha()
    if sha is not None:
        found = checkout_imports()
        if found:
            raise SystemExit(f"the frozen reference ({ENV}={sha}) imported {found[:5]} from the checkout, not "
                             "the installed reference (crates/ooxml-oracle/python/reference_packages.py)")
    return sha


def installed_sources() -> list[Path]:
    """The Python files of the FROZEN packages as installed, in a run that uses the frozen
    reference (none otherwise: the checkout's are the chain's own files)."""
    if frozen_sha() is None:
        return []
    files: list[Path] = []
    for module in FROZEN.values():
        spec = importlib.util.find_spec(module)
        if spec is not None and spec.submodule_search_locations:
            for folder in spec.submodule_search_locations:
                files.extend(sorted(Path(folder).rglob("*.py")))
    return files
