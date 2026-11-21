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

import functools
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


def context_of(caller: Mapping[str, Any] | None, date: str | None = None, *, typography: bool = True) -> Any:
    """An ``ooxml_core.CallContext`` for ``caller``, its writes dated ``date`` (UTC to the
    second, the session clock's; ``None``: the Rust session's own clock) and spaced by their
    language unless ``typography`` is off (``docx_format(typography=False)``: as given)."""
    import ooxml_core

    return ooxml_core.CallContext(caller=caller_key(caller), date=date, typography=typography)


def host_ids(view: Any) -> Any:
    """An ``ooxml_core.HostIds`` of what ``view``'s id ledger holds beside the parts a Python
    tool changed in it: where its draws of new ids got to (an id it drew and did not write is
    given) and its renames (a block it stamped keeps its old id); ``None`` without a view."""
    import ooxml_core

    if view is None:
        return None
    pool = getattr(getattr(view, "_ledger", None), "pool", None)
    drawn = {seed: k for seed, k in pool.cursors.items() if k > 0} if pool is not None else {}
    return ooxml_core.HostIds(drawn=drawn, aliases=dict(getattr(view, "_aliases", None) or {}))


def options(**given: Any) -> Any:
    """An ``ooxml_core.CallOptions``: what one ``Session.call`` or ``Session.edit`` runs with
    beside its name and arguments (``context``, ``documents``, ``placer``, ``layout``,
    ``blobs``, ``written``, ``limit``)."""
    import ooxml_core

    return ooxml_core.CallOptions(**given)


def documents(**given: Any) -> Any:
    """An ``ooxml_core.CallDocuments``: what a call needs about the open documents it names
    (``at``: the versions it reads; ``names``: the host's names for them)."""
    import ooxml_core

    return ooxml_core.CallDocuments(**given)


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
        # The ids retired are the session's record (lane P2c10), from the state opened with on.
        _seed_retired(mirror, document._ledger)
        document.retired_ids = functools.partial(retired_ids, mirror)
        # The records of placed values are read by the session too (lane P2d1).
        document.owned_records = functools.partial(owned_records, mirror, weakref.ref(document))
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
        # the view's ledger beside the parts: where its draws of new ids got to (an id it drew
        # and did not write is given) and its renames (a block it stamped keeps its old id)
        ids = host_ids(live)
        with mirror.lock:
            made = mirror.session.bridge(mirror.doc, tool, parts, context=context_of(caller), ids=ids)
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
            # the session words a refusal of an id this version removed by the host's number
            mirror.session.number_version(mirror.doc, head, number)
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


def _seed_retired(mirror: "reads.Mirror", ledger: Any) -> None:
    """Give the session the ids the state the document was opened with retired (round 38):
    each with the version that removed it and who made it, as the state says them."""
    rows = [("block", value, version, ledger.earlier.get(version)) for value, version in ledger.gone.items()]
    rows += [("drawing", value, version, ledger.earlier.get(version))
             for value, version in ledger.drawings_gone.items()]
    if rows:
        mirror.session.seed_retired(mirror.doc, rows)


def retired_ids(mirror: "reads.Mirror") -> Any:
    """The document's retired ids as the session records them (``Document.retired_ids``),
    each session version numbered as the tool layer numbered it (:attr:`.reads.Mirror.numbers`;
    one not numbered yet -- its call still ending -- is left out until it is)."""
    from ..edit.ids import RetiredIds

    out = RetiredIds({}, {}, {}, {})
    numbers = mirror.numbers
    for kind, value, seq, caller, tool, version, said in mirror.session.retired(mirror.doc):
        if seq is not None:
            version = numbers.get(seq)
            if version is None:
                continue
            if tool:
                context = ", ".join(f"{key}={value_}" for key, value_ in json.loads(caller).items()) if caller else ""
                out.said[version] = f"{tool} ({context})" if context else tool
        else:
            out.earlier[version] = said
        (out.gone if kind == "block" else out.drawings_gone)[value] = version
    return out


def owned_records(mirror: "reads.Mirror", document: Any) -> dict[str, Any] | None:
    """The records of placed values of the version the view of a document the session owns
    is, as the session reads them (``Version.records``: the tables' and charts' records, the
    entries left out, the text facts and labels); ``None`` where the view is ahead of the
    session (a Python step changed it) or not the session's."""
    live = document()
    head = mirror.view_head
    if live is None or not mirror.owned or head is None or live.history.batch_changed:
        return None
    found = head.records()
    return json.loads(found) if found is not None else None


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
    a call the session served under the lock): the session's, against the problems it opened
    with; ``None`` where the Python checks answer (the view is current, so reading it costs
    nothing).  A call served without the lock carries its version's own (``edits.Made``)."""
    mirror = getattr(entry, "handle", None)
    if not isinstance(mirror, reads.Mirror) or entry.view_current:
        return None
    found = mirror.session.validation(mirror.doc)
    if found is None:
        return None
    new, fixed, baseline = found
    return {"new": list(new), "fixed": list(fixed), "baseline": baseline}


def session_validation(entry: Any) -> dict[str, Any] | None:
    """The save gate's comparison of a document the session owns (or of a read copy of one of
    its committed versions), at the entry's version: the session's, against the problems it
    opened with -- the format's :attr:`~ooxml_edit.tools.DocumentFormat.validation`, so
    neither ``check``'s ``validation`` nor saving makes the Python view current to validate
    it.  ``None`` for a document the session does not own, or a version it let go."""
    owned = reads._owned(entry)
    mirror = getattr(owned, "handle", None)
    if not isinstance(mirror, reads.Mirror):
        return None
    with mirror.lock:
        version = mirror.versions.get(entry.version)
    if version is None:
        return None
    found = mirror.session.validation(mirror.doc, at=version)
    if found is None:
        return None
    new, fixed, baseline = found
    return {"new": list(new), "fixed": list(fixed), "baseline": baseline}


OWNER = RustOwner()
