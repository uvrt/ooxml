"""docx-agent's documents owned by the Rust session (phase P2; lane P2c4: the session is the
only owner).

A Word document open in a toolbox session is held by the process's Rust session
(``ooxml_core.Session``, the one :mod:`.reads` reads): its versions, its steps, its ids and
its saved bytes are the session's.  :class:`RustOwner` is the format's
:attr:`~ooxml_edit.tools.session.DocumentFormat.owner` (the protocol is
:mod:`ooxml_edit.tools.owned`'s):

- a tool with a Rust editing handler runs in the session (:mod:`.edits`), and the Python
  ``Document`` is not read;
- every other tool runs on the Python view, made current from the session's head when it is
  read (:func:`.view.follow`: the changed parts, the ids and renames the session gave), and
  the parts it changed are committed through ``Session.bridge`` as the step of that tool and
  caller (``bridge_calls{tool}``);
- undo and redo are the session's (``Session.undo``/``redo``); the save is the session's bytes;
- a changing call's checks read the session's validation (:func:`owned_validation`) unless
  the view is current anyway or a layout is compared.
"""

from __future__ import annotations

import json
import weakref
from collections.abc import Mapping
from typing import Any

from . import reads
from .view import follow


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


class RustOwner:
    """The Rust session as the owner of docx-agent's documents in a toolbox session; its
    handle per document is the :class:`~.reads.Mirror` the reads use."""

    def open(self, document: Any) -> "reads.Mirror | None":
        mirror = reads.mirror_of(document)
        if mirror is None:
            return None             # no session (ooxml_core without one, or reads set to none)
        with mirror.lock:
            mirror.owned = True
            mirror.view_head = mirror.opened = mirror.session.head(mirror.doc)
            mirror.live = weakref.ref(document)
        # One set of bytes per version: the view's are the session's (owned_bytes).
        document.held_by(owned_bytes)
        return mirror

    def head(self, mirror: "reads.Mirror") -> Any:
        return mirror.session.head(mirror.doc)

    def parts_between(self, mirror: "reads.Mirror", before: Any, after: Any) -> list[str]:
        return after.parts_changed_since(before)

    def touched(self, mirror: "reads.Mirror", before: Any, after: Any) -> list[dict[str, Any]]:
        return json.loads(after.touched_since(before))

    def follow(self, mirror: "reads.Mirror", document: Any, head: Any, number: int, last: int) -> None:
        with mirror.lock:
            then = mirror.view_head
            parts = dict(head.changed_since(then))
            given, renamed = head.ids_since(then)
            follow(document, parts, given, renamed, number, last)
            mirror.view_head = head

    def bridge(self, mirror: "reads.Mirror", tool: str, caller: Mapping[str, Any] | None,
               parts: dict[str, bytes | None]) -> Any:
        from ..layout import laid_mark, same_state

        live = mirror.live()
        # What the step laid out of its last state is keyed by the view's own bytes then:
        # found under the session's bytes of the version too (only where it laid out).
        laid = live is not None and laid_mark(live) != mirror.laid
        own = live.package.to_bytes() if laid else None
        with mirror.lock:
            made = mirror.session.bridge(mirror.doc, tool, parts, context=context_of(caller))
            mirror.view_head = made
        if own is not None:
            same_state(live, own, bytes(made.save()))
        if live is not None:
            mirror.laid = laid_mark(live)
        return made

    def undo(self, mirror: "reads.Mirror") -> Any:
        return self._step(mirror, mirror.session.undo)

    def redo(self, mirror: "reads.Mirror") -> Any:
        return self._step(mirror, mirror.session.redo)

    @staticmethod
    def _step(mirror: "reads.Mirror", step: Any) -> Any:
        with mirror.lock:
            done, _version = step(mirror.doc, 1)
            return mirror.session.head(mirror.doc) if done else None

    def reader(self, mirror: "reads.Mirror", head: Any, number: int) -> Any:
        """What makes the Python reader of ``head`` (the document's version ``number``) from
        its bytes: a ``Document`` of them numbered ``number``, with the ids the session gave
        and the renames it made since the document was opened, read through the session's
        copy of the document (its reads find ``head`` by ``number``).  (No ``draft``: an owned
        document's calls are not prepared on drafts, ooxml-edit's ``OwnedEntry``.)"""
        from ..edit.ids import copy_ledger
        from ..layout import reader_copy

        opened = mirror.opened

        def ids_of(live: Any) -> Any:
            def ids(copy: Any) -> None:
                given, renamed = head.ids_since(opened)
                follow(copy, {}, given, renamed, number, number)
                if live.history.version == number:
                    # The view is this version: what its ids were (deleted ones too) is the copy's.
                    copy._ledger = copy_ledger(live._ledger)
                    copy._aliases.update(live._aliases)

            return ids

        def make(data: bytes) -> Any:
            live = mirror.live()
            return reader_copy(live, data, ids_of(live))

        return make

    def keep(self, mirror: "reads.Mirror", number: int, head: Any) -> None:
        from .check import owner_committed

        with mirror.lock:
            mirror.keep(number, head)
        live = mirror.live()
        if live is not None:
            owner_committed(live, number)

    def save(self, mirror: "reads.Mirror") -> bytes:
        return bytes(mirror.session.save(mirror.doc))

    def close(self, mirror: "reads.Mirror") -> None:
        live = mirror.live() if mirror.live is not None else None
        if live is not None:
            live.held_by(None)
        reads._close(mirror.session, mirror.doc)


def owned_bytes(document: Any) -> bytes | None:
    """The bytes of the version a view of a document the session owns is, as the session
    writes them -- the view's ``to_bytes`` (``Document.held_by``), so every layout and copy of
    a version is keyed by them; ``None`` for any other document, and once a Python step
    changed the view (its own ``to_bytes``)."""
    mirror = document.__dict__.get("_rust_reads")
    if not isinstance(mirror, reads.Mirror) or not mirror.owned or mirror.view_head is None \
            or document.history.batch_changed:
        return None             # (a Python step that changed the view: ahead of the session)
    return bytes(mirror.view_head.save())


def owned_validation(entry: Any) -> dict[str, Any] | None:
    """The validation delta of a document the session owns whose view is not current (after
    a call the session served): the session's, against the problems it opened with; ``None``
    where the Python checks answer (the view is current, so reading it costs nothing)."""
    mirror = getattr(entry, "handle", None)
    if not isinstance(mirror, reads.Mirror) or entry.view_current:
        return None
    found = mirror.session.validation(mirror.doc)
    if found is None:
        return None
    new, fixed, baseline = found
    return {"new": list(new), "fixed": list(fixed), "baseline": baseline}


OWNER = RustOwner()
