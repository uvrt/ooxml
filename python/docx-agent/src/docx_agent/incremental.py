"""The edit path's passes over the whole document, made proportional to the change (0.1.116).

An edit, an undo or a redo leaves a version whose body's bytes differ from the version
before in one stretch of top-level blocks (``tools/touched.py`` finds it from the bytes:
the blocks before the first byte that differs and after the last are the same blocks).
The passes that read every block -- the ids ledger's note (:class:`docx_agent.edit.ids.IdScan`),
the touched heading sections (:mod:`docx_agent.tools.touched`), the Rust mirror's list
labels and table records (:mod:`docx_agent.rust.mirror`) -- keep what they read per block
and read again only the blocks in that stretch.

``DOCX_AGENT_INCREMENTAL=0`` in the environment runs every pass whole, as docx-agent 0.1.115
did (the switch :mod:`docx_agent.revalidate` reads too).  ``DOCX_AGENT_FULL_VALIDATE=1``
runs the whole pass beside each one and raises :class:`Mismatch` where they differ
(docx-agent's own tests set it): the whole pass stays the definition.
"""

from __future__ import annotations

import os
from collections import Counter

#: What each pass did, counted: ``<pass>: incremental``, ``<pass>: whole (<why>)``.
STATS: Counter = Counter()


class Mismatch(AssertionError):
    """``DOCX_AGENT_FULL_VALIDATE``: a pass made per change differs from the whole pass."""


def enabled() -> bool:
    """Whether the passes are made per change (``DOCX_AGENT_INCREMENTAL`` is not ``0``)."""
    return os.environ.get("DOCX_AGENT_INCREMENTAL", "1").strip() != "0"


def verifying() -> bool:
    """Whether each pass is compared with the whole one (``DOCX_AGENT_FULL_VALIDATE=1``)."""
    return os.environ.get("DOCX_AGENT_FULL_VALIDATE", "").strip() == "1"


def check(name: str, made: object, whole: object) -> None:
    """Raise :class:`Mismatch` where ``made`` (per change) is not ``whole``."""
    if made != whole:
        raise Mismatch(f"{name}: the pass made per change differs from the whole pass")
