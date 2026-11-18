"""docx-agent's documents owned by the Rust session (phase P2, lane P2c3: the switch).

**Who owns what.**  A Word document open in a toolbox session is held by the process's Rust
session (``ooxml_core.Session``, the one :mod:`.reads` reads): its versions, its history and
its saved bytes are the session's (:class:`RustOwner`, the format's
:attr:`~ooxml_edit.tools.session.DocumentFormat.owner`).  The ``Document`` the tool layer
holds is the bridge's working copy (docs/architecture/README.md section 13), kept at the
session's head:

- a tool with a Rust editing handler runs there (:mod:`.edits`), and the parts it changed are
  written into the working copy as that call's step (``Document.take_parts``);
- every other tool -- and a call the Rust handler answers ``unported`` -- runs on the
  working copy, and the version it made is committed in the session through
  ``Session.bridge`` as the step of that tool and caller, counted in ``bridge_calls{tool}``
  (:meth:`RustOwner.committed`);
- an undo or redo is made in the session with the same caller, step for step with the
  working copy's (:meth:`RustOwner.followed`): a working-copy step is one session step;
- the saved bytes are the session's (:meth:`RustOwner.save`).

The working copy's history still decides an undo's refusal and its words (its rules are the
session's: a caller's own latest change only while it is the document's latest); the session
then makes the same steps.  Where the two ever disagree, the session is brought to the
working copy's state through the bridge (tool ``undo`` or ``redo``) and the disagreement is
logged and counted (:func:`disagreements`), never silent.
"""

from __future__ import annotations

import json
import logging
import threading
from collections import Counter
from collections.abc import Mapping
from typing import Any

from . import reads

log = logging.getLogger("docx_agent.rust")

_COUNTS_LOCK = threading.Lock()
_DISAGREED: Counter = Counter()


def disagreements() -> dict[str, int]:
    """Undos and redos the session made otherwise than the working copy, by direction,
    since the process started (each brought back through the bridge)."""
    with _COUNTS_LOCK:
        return dict(_DISAGREED)


def caller_key(caller: Mapping[str, Any] | None) -> str | None:
    """A call's context (:attr:`~ooxml_edit.tools.dispatch.Call.caller`) as the session's
    caller: its JSON, keys sorted; ``None`` for none (the session's own)."""
    if not caller:
        return None
    return json.dumps(dict(caller), sort_keys=True, default=str, separators=(",", ":"))


def context_of(caller: Mapping[str, Any] | None) -> Any:
    """An ``ooxml_core.CallContext`` for ``caller``."""
    import ooxml_core

    return ooxml_core.CallContext(caller=caller_key(caller))


def owned(document: Any) -> "reads.Mirror | None":
    """The Rust session's copy of ``document`` when the session owns it, else ``None``."""
    found = document.__dict__.get("_rust_reads")
    return found if found is not None and found.owned else None


class RustOwner:
    """The Rust session as the owner of docx-agent's documents in a toolbox session."""

    def hold(self, document: Any) -> "reads.Mirror | None":
        """The session's copy of ``document``, opened from its bytes and owned when it joins
        a toolbox session (the format's ``configure``); ``None`` without a session (the
        working copy is then the document)."""
        found = owned(document)
        if found is not None:
            return found
        made = reads.mirror_of(document)
        if made is None:
            return None
        with made.lock:
            if not made.owned:
                made.owned = True
                # Opened from this version: the session's head is it (else the first commit
                # hands over what differs).
                number = document.history.version
                if made.versions and next(reversed(made.versions)) == number:
                    made.synced = number
        return made

    def committed(self, entry: Any) -> None:
        """Commit the working copy's version in the session: the parts whose bytes differ
        from the session's, as the step of the call that made it (its tool and caller, by
        :attr:`~ooxml_edit.tools.session.DocumentEntry.owners`; ``library`` for a change made
        outside a call).  A version whose parts the session has already (an edit made
        there) is kept as the session's head."""
        document = entry.document
        mirror = self.hold(document)
        if mirror is None:
            return
        number = document.history.version
        with mirror.lock:
            if mirror.synced == number:
                return
            step = entry.owners.get(number)
            tool = step.tool if step is not None else "library"
            changed = _changed(mirror, document)
            if changed:
                made = mirror.session.bridge(mirror.doc, tool, changed,
                                             context=context_of(step.caller if step is not None else None))
            else:
                made = mirror.session.head(mirror.doc)
            mirror.keep(number, made)
            mirror.synced = number

    def followed(self, entry: Any, direction: str, count: int, caller: Mapping[str, Any] | None) -> None:
        """Make in the session the ``count`` undos or redos the working copy made."""
        mirror = owned(entry.document)
        if mirror is None:
            return
        document = entry.document
        number = document.history.version
        step = getattr(mirror.session, direction)
        with mirror.lock:
            try:
                done, _ = step(mirror.doc, count, context=context_of(caller))
            except ValueError as refused:
                done, why = 0, str(refused)
            else:
                why = f"{done} of {count} step(s)"
            # The parts the working copy's undo or redo changed: the session's own steps
            # changed them alike when it made as many.
            changed = _changed(mirror, document)
            if done != count:
                with _COUNTS_LOCK:
                    _DISAGREED[direction] += 1
                log.error("docx-agent: the Rust session's %s of %s differs from the working copy's "
                          "(%s); brought to it through the bridge", direction, mirror.doc, why)
                if changed:
                    mirror.session.bridge(mirror.doc, direction, changed, context=context_of(caller))
            mirror.keep(number, mirror.session.head(mirror.doc))
            mirror.synced = number

    def save(self, entry: Any) -> bytes:
        """The document's bytes: the session's head, once the working copy's version is
        committed there."""
        mirror = owned(entry.document)
        if mirror is None or mirror.synced != entry.document.history.version:
            self.committed(entry)
            mirror = owned(entry.document)
        if mirror is None:
            return entry.snapshot()
        return bytes(mirror.session.save(mirror.doc))


def _changed(mirror: "reads.Mirror", document: Any) -> dict[str, bytes | None]:
    """The working copy's parts whose bytes differ from the session's latest, and the
    parts it no longer has (``None``); the mirror then holds the working copy's."""
    now = reads._identities(document.package)
    changed: dict[str, bytes | None] = {}
    for part, (identity, data) in now.items():
        if mirror.parts.get(part) != identity:
            changed[part] = data() if callable(data) else data
    for part in mirror.parts.keys() - now.keys():
        changed[part] = None
    mirror.parts = {part: identity for part, (identity, _) in now.items()}
    return changed


OWNER = RustOwner()
