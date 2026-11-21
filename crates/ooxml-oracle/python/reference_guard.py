"""Keeps the Rust core out of the gates' reference: docx-agent in Python, never Rust.

The differential gates compare docx-agent in Python (the reference, ``DOCX_AGENT_RUST=0``) with
the Rust core's oracle binaries.  docx-agent serves calls through the Rust core wherever its
module ``ooxml_core`` is loaded, ``DOCX_AGENT_RUST=0`` or not (only ``use_session(None)`` stops
it), so a reference process that loaded it would compare Rust with Rust and pass silently --
and the reference cache's key (reference_cache.py) does not cover the Rust core's code.  The
gates' environment does not install it (.github/actions/gates-reference); this makes that a
guarantee wherever the harness runs:

* :func:`guard` -- each harness calls it before it imports docx-agent -- fails when
  ``ooxml_core`` is loaded already, and puts ``no_core/`` first on this process's path and on
  ``PYTHONPATH``.  Every worker process the reference starts gets that path (spawn workers
  from this process's, subprocess workers from ``PYTHONPATH``), so in each of them, too,
  ``import ooxml_core`` finds the stand-in in ``no_core/``, which fails as an environment
  without the Rust core does: docx-agent's Rust paths are not taken, and none can serve.
* :func:`check` -- the harness's report calls it at the end of each run -- fails the run when
  ``ooxml_core`` is loaded in this process anyway, naming the modules.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

#: The Rust core's Python module.
CORE = "ooxml_core"
#: The folder holding the stand-in for :data:`CORE`.
NO_CORE = str(Path(__file__).resolve().parent / "no_core")


def loaded() -> list[str]:
    """The :data:`CORE` modules loaded in this process."""
    return sorted(name for name in sys.modules if name == CORE or name.startswith(CORE + "."))


def check() -> None:
    """Fails (SystemExit) when :data:`CORE` is loaded in this process, naming its modules."""
    found = loaded()
    if found:
        raise SystemExit(f"the gates' reference loaded the Rust core ({', '.join(found)}): docx-agent would serve "
                         "through it, Rust compared with Rust (crates/ooxml-oracle/python/reference_guard.py)")


def guard() -> None:
    """Keeps :data:`CORE` out of this process and the worker processes it starts from now on
    (see the module's docstring); fails when it is loaded already."""
    check()
    if NO_CORE not in sys.path:
        sys.path.insert(0, NO_CORE)
    paths = [p for p in os.environ.get("PYTHONPATH", "").split(os.pathsep) if p and p != NO_CORE]
    os.environ["PYTHONPATH"] = os.pathsep.join([NO_CORE, *paths])
