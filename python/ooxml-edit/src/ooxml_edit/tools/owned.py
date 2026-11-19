"""Documents whose owner is a store outside the tool layer (lane P2c4): a format's
:attr:`~.session.DocumentFormat.owner` (a library's native session) keeps the document's
versions, its steps, its ids and its saved bytes; the tool layer keeps no working copy of
its own.

**What the owner is.**  An object with these methods, each given the handle
:meth:`open` returned (the owner's per-document state):

- ``open(document) -> handle | None``: hold ``document`` (as opened, its bytes and ids) from
  now on; ``None``: the owner does not take it (the tool layer then holds it itself);
- ``head(handle) -> head``: the document's latest version, a value whose ``number`` is the
  owner's own count of its versions (a new one for every version it makes);
- ``parts_between(handle, before, after) -> Iterable[str]``: the parts whose content differs
  between two heads, and ``touched(handle, before, after) -> list[dict]``: what changed
  between them in the format's own addresses (``DocumentFormat.touched``'s entries);
- ``follow(handle, document, head, number, last)``: bring ``document`` -- the view -- to
  ``head``, numbered ``number`` (``last``: the highest number given), its ids too;
- ``bridge(handle, tool, caller, parts) -> head``: commit the parts a Python tool changed in
  the view (bytes, ``None`` removes one) as the step of ``tool`` and ``caller``;
- ``undo(handle) -> head | None`` and ``redo(handle) -> head | None``: one step of the
  document's history, ``None`` when there is none (refused: ``ValueError``);
- ``keep(handle, number, head)``: the head is the document's version ``number`` (reads find it);
- ``reader(handle, head, number) -> make``: what makes a Python reader of the version ``head``
  (numbered ``number``) from its bytes, for the reads of a committed version (``head.save()``);
- ``save(handle) -> bytes`` and ``close(handle)``.

**The view.**  :attr:`OwnedEntry.document` is a Python document of the format, made current
lazily: only when something reads it -- a Python tool (whose change the owner commits through
``bridge``), a check only Python makes -- and only when the head moved since it was last
made current.  A call the owner serves itself (a Rust editing tool, :meth:`OwnedEntry.served`)
never reads it.

**Numbers.**  :class:`OwnedHistory` numbers the document's states as
:class:`~ooxml_edit.history.History` does (a step a new number, undo and redo back to the
number of the state they bring back), so every version-keyed thing of the tool layer (refs,
owners, caches) means what it meant.  The steps are the owner's: an undo the owner does not
make as the numbers say is an error, never smoothed over.

Must not: keep a copy of the document beside the owner's, or read the view on a served call.
"""

from __future__ import annotations

import contextvars
from contextlib import ExitStack, contextmanager
from dataclasses import dataclass, field
from typing import Any, Iterator

from ..history import Step
from .session import Committed, DocumentEntry, StepOwner

#: The steps kept for undo, as the owner keeps them (``ooxml_core.Session``'s depth).
DEPTH = 50

#: The entries (``id``) whose call's step is open in this context: only that call's reads of
#: the view open the view's step (a read made meanwhile by another call does not).
_IN_STEP: contextvars.ContextVar[frozenset[int]] = contextvars.ContextVar("ooxml_edit_owned_step",
                                                                           default=frozenset())


class OwnerDisagrees(RuntimeError):
    """The owner did what the tool layer's numbers say it would not (an undo it did not make,
    a step where none was expected): the two would say different things from now on."""


class OwnedHistory:
    """The numbers of an owned document's states and its undo and redo, which the owner
    makes (the protocol the tool layer reads of a :class:`~ooxml_edit.history.History`)."""

    compares = False

    def __init__(self, entry: "OwnedEntry", head: Any, version: int, last: int) -> None:
        self._entry = entry
        self._number = version
        self._last = last
        self._undo: list[int] = []
        self._redo: list[int] = []
        self._heads: dict[int, Any] = {version: head}

    @property
    def version(self) -> int:
        return self._number

    @property
    def last_version(self) -> int:
        return self._last

    @property
    def in_batch(self) -> bool:
        return self._entry.batch_depth > 0

    def head(self, number: int | None = None) -> Any:
        """The owner's version numbered ``number`` (now: the latest), ``None`` when let go."""
        return self._heads.get(self._number if number is None else number)

    def committed(self, head: Any, view: Any = None) -> int:
        """A step the owner made: its new head numbered, redo dropped; ``view``: the view
        that made it (a Python step), numbered as the head before the owner keeps it."""
        self._undo.append(self._number)
        if len(self._undo) > DEPTH:
            del self._undo[0]
        self._redo.clear()
        self._last += 1
        self._number = self._last
        if view is not None:
            view.history.follow(self._number, self._last)
        self._keep(head)
        return self._number

    def _keep(self, head: Any) -> None:
        self._heads[self._number] = head
        kept = set(self._undo) | set(self._redo) | {self._number}
        for number in [n for n in self._heads if n not in kept]:
            del self._heads[number]
        self._entry.owner.keep(self._entry.handle, self._number, head)

    def steps(self) -> list[Step]:
        versions = [*self._undo, self._number]
        return [Step(versions[k], versions[k + 1]) for k in range(len(self._undo))]

    def can_undo(self) -> bool:
        return bool(self._undo)

    def can_redo(self) -> bool:
        return bool(self._redo)

    @property
    def next_redo(self) -> int | None:
        return self._redo[-1] if self._redo else None

    def undo(self) -> bool:
        return self._move(self._entry.owner.undo, self._undo, self._redo)

    def redo(self) -> bool:
        return self._move(self._entry.owner.redo, self._redo, self._undo)

    def _move(self, step: Any, source: list[int], target: list[int]) -> bool:
        head = step(self._entry.handle)
        if head is None:
            if source:
                raise OwnerDisagrees(f"{self._entry.doc_id}: the owner has no step where the "
                                     f"history has {len(source)}")
            return False
        if not source:
            raise OwnerDisagrees(f"{self._entry.doc_id}: the owner made a step the history "
                                 "does not have")
        target.append(self._number)
        self._number = source.pop()
        self._keep(head)
        return True

    def mark(self) -> Any:
        return None

    def clear(self) -> None:
        self._undo.clear()
        self._redo.clear()
        self._heads = {self._number: self._heads.get(self._number)}

    def kept_bytes(self) -> int:
        return 0


class OwnedChange:
    """What an owned document's content did while :meth:`OwnedEntry.watch` was open: the
    parts between the heads before and after and what changed in them (asked of the owner),
    and -- where a Python tool changed the view -- the view's own
    :class:`~ooxml_edit.history.Change`."""

    def __init__(self, entry: "OwnedEntry") -> None:
        self._entry = entry
        self.start = entry.version
        self.end: int | None = None
        self.view: Any = None
        self._parts: frozenset[str] | None = None

    @property
    def changed(self) -> bool:
        return (self._entry.version if self.end is None else self.end) != self.start

    @property
    def parts(self) -> frozenset[str]:
        if self._parts is None:
            history = self._entry.history
            end = history.version if self.end is None else self.end
            before, after = history.head(self.start), history.head(end)
            if end == self.start:
                self._parts = frozenset()
            elif before is None or after is None:
                self._parts = frozenset(self.view.parts) if self.view is not None else frozenset()
            else:
                self._parts = frozenset(self._entry.owner.parts_between(
                    self._entry.handle, before, after))
        return self._parts

    def before(self, path: str) -> bytes | None:
        return self.view.before(path) if self.view is not None else None

    def after(self, path: str) -> bytes | None:
        return self.view.after(path) if self.view is not None else None


@dataclass(eq=False)
class OwnedEntry(DocumentEntry):
    """A document the format's owner holds (see the module docstring)."""

    owner: Any = field(default=None, repr=False)
    handle: Any = field(default=None, repr=False)
    batch_depth: int = field(default=0, repr=False)
    _view_number: int | None = field(default=None, repr=False)
    _owned: OwnedHistory | None = field(default=None, repr=False)
    _step: ExitStack | None = field(default=None, repr=False)
    _view_change: Any = field(default=None, repr=False)
    _watches: list[OwnedChange] = field(default_factory=list, repr=False)

    owned = True

    def own(self, owner: Any, handle: Any) -> None:
        """The document (the one this entry was made with) is ``owner``'s, which holds it
        as ``handle`` (what its ``open`` gave)."""
        document = self.held
        self.owner = owner
        self.handle = handle
        history = getattr(document, "history", None)
        version = getattr(history, "version", 0)
        last = getattr(history, "last_version", version)
        self._owned = OwnedHistory(self, owner.head(self.handle), version, last)
        self._view_number = version

    # -- the view ---------------------------------------------------------------------------

    @property
    def document(self) -> Any:
        """The view at the head, made current now if the head moved since it last was."""
        owned = self._owned
        if owned is not None and self._view_number != owned.version:
            with self.lock:     # (a read that came before the lock: no edit meanwhile)
                if self._view_number != owned.version:
                    self.owner.follow(self.handle, self.held, owned.head(), owned.version,
                                      owned.last_version)
                    self._view_number = owned.version
        if self.batch_depth and self._step is not None and self._view_change is None \
                and id(self) in _IN_STEP.get():
            self._open_view_step()
        return self.held

    @document.setter
    def document(self, value: Any) -> None:
        self.held = value

    @property
    def view_current(self) -> bool:
        """Whether the view is the head's (reading it costs nothing now)."""
        return self._owned is None or self._view_number == self._owned.version

    @property
    def history(self) -> Any:
        return self._owned if self._owned is not None else getattr(self.held, "history", None)

    @property
    def version(self) -> int:
        if self.pinned is not None:
            return self.pinned
        return self._owned.version if self._owned is not None else 0

    def bump(self) -> None:
        """(A step is counted by the owner's head.)"""

    def step_target(self, direction: str) -> Any:
        return getattr(self.history, direction)

    # -- a call's step ---------------------------------------------------------------------

    def _open_view_step(self) -> None:
        """The view's own batch and watch, opened the first time a call's step reads it: a
        Python tool's change is one step of the view's, made one step of the owner's."""
        view, stack = self.held, self._step
        assert stack is not None
        batch = getattr(view, "batch", None) or view.history.batch
        # (The watch first: it closes after the batch, which makes the step as it closes.)
        self._view_change = stack.enter_context(view.history.watch())
        stack.enter_context(batch())
        for watch in self._watches:
            watch.view = self._view_change

    @contextmanager
    def batch(self, owner: StepOwner | None = None) -> Iterator[None]:
        """One step around a call: what a Python tool changed in the view is committed by the
        owner as the step of ``owner``'s tool and caller; a served call's step is the
        owner's already."""
        outer = self.batch_depth == 0
        before = self.version
        self.batch_depth += 1
        try:
            if not outer:
                yield
            else:
                with ExitStack() as stack:
                    self._step = stack
                    token = _IN_STEP.set(_IN_STEP.get() | {id(self)})
                    try:
                        yield
                    finally:
                        _IN_STEP.reset(token)
                # The view's batch closed (its step made, as the format makes one) and its
                # watch with it: what it changed is the owner's to commit.
                self._step = None
                self._end_step(owner)
        finally:
            self.batch_depth -= 1
            if outer:
                self._step = None
                self._view_change = None
        if owner is not None and self.version != before:
            self.owners[self.version] = owner
            self.own_undos = [undo for undo in self.own_undos if undo.kind != "plain"]

    def _end_step(self, owner: StepOwner | None) -> None:
        change, owned = self._view_change, self._owned
        if change is None or not change.changed or not change.parts or owned is None:
            return
        if self.owner.head(self.handle).number != owned.head().number:
            raise OwnerDisagrees(f"{self.doc_id}: the owner's head moved during a Python step")
        package = self.held.package
        parts = {part: (package.read(part) if package.has_part(part) else None)
                 for part in change.parts}
        head = self.owner.bridge(self.handle, owner.tool if owner is not None else "library",
                                 dict(owner.caller) if owner is not None else None, parts)
        owned.committed(head, self.held)
        self._view_number = owned.version

    def served(self, head: Any) -> None:
        """A call the owner served made ``head``: the call's step."""
        assert self._owned is not None
        if head.number != self._owned.head().number:
            self._owned.committed(head)

    @contextmanager
    def savepoint(self, start: Any = None) -> Iterator[bool]:
        view = self.document
        savepoint = getattr(view, "savepoint", None)
        with (savepoint() if savepoint is not None else view.history.savepoint(start)):
            yield True

    @contextmanager
    def watch(self) -> Iterator[Any]:
        change = OwnedChange(self)
        change.view = self._view_change
        self._watches.append(change)
        try:
            yield change
        finally:
            self._watches.remove(change)
            change.end = self.version

    def mark(self) -> Any:
        return None

    def touched_of(self, change: Any, fmt: Any) -> list[dict[str, Any]]:
        """What ``change`` touched, as the owner says it between the heads it went from
        and to; by its parts where one of them was let go."""
        history = self._owned
        if history is None:
            return [{"part": part} for part in sorted(change.parts)]
        end = history.version if change.end is None else change.end
        before, after = history.head(change.start), history.head(end)
        if before is None or after is None:
            return [{"part": part} for part in sorted(change.parts)]
        return list(self.owner.touched(self.handle, before, after))

    # -- versions for others ----------------------------------------------------------------

    def commit(self, fmt: Any) -> None:
        """Keep this version as the committed one for reads that do not wait for the edit
        after it (:meth:`~.session.DocumentEntry.reader`): the owner's version, its bytes and
        its Python reader made only when a read first asks (the owner's ``reader``)."""
        owned = self._owned
        committed = self.committed
        version = self.version
        if owned is None:
            return
        if committed is not None and committed.version == version:
            if committed.refs != self.refs or (committed.tracking or None) != (self.tracking or None):
                self.committed = committed.with_refs(
                    dict(self.refs), dict(self.ref_states.get(version, self.refs)),
                    dict(self.ref_owners), tracking=self.tracking)
                self._told()
            return
        head = owned.head()
        self.committed = Committed(version, head.save, self.owner.reader(self.handle, head, version),
                                   dict(self.refs), dict(self.ref_states.get(version, self.refs)),
                                   tracking=self.tracking, owners=dict(self.owners),
                                   seq=self.change_seq, ref_owners=dict(self.ref_owners))
        self._told()

    def snapshot(self) -> bytes:
        version = self.version
        kept = self._snapshot
        if kept is not None and kept[0] == version:
            return kept[1]
        data = self.owner.save(self.handle)
        self._snapshot = (version, data)
        return data

    def release(self) -> None:
        super().release()
        view_history = getattr(self.held, "history", None)
        if view_history is not None:
            view_history.clear()
        if self.owner is not None:
            self.owner.close(self.handle)


__all__ = ["DEPTH", "OwnedChange", "OwnedEntry", "OwnedHistory", "OwnerDisagrees"]
