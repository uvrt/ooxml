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
- ``keep(handle, number, head, first)``: the head is the document's version ``number`` (reads find it);
  ``first``: the owner's number of the step's first version, where it made several (each is ``number``);
- ``reader(handle, head, number) -> make``: what makes a Python reader of the version ``head``
  (numbered ``number``) from its bytes, for the reads of a committed version (``head.save()``);
- ``save(handle) -> bytes`` and ``close(handle)``;
- optionally, step groups (a ``batch``'s ops as one step of the owner's): ``open_group(handle) ->
  group``, ``group_mark(handle, group) -> mark``, ``revert_group(handle, group, mark, caller) -> head |
  None`` (what the group committed after ``mark`` -- ``None``: since it opened -- taken back, no
  redo of it; the new head, ``None`` when nothing was committed since) and ``close_group(handle,
  group, caller) -> head | None`` (one step of ``caller``'s; ``None`` when it committed nothing).

**The view.**  :attr:`OwnedEntry.document` is a Python document of the format, made current
lazily: only when something reads it -- a Python tool (whose change the owner commits through
``bridge``), a check only Python makes -- and only when the head moved since it was last
made current.  A call the owner serves itself (a Rust editing tool, :meth:`OwnedEntry.served`)
never reads it.

**Batches** (lane H1).  Where the owner has step groups, a batch on the document opens one
(:meth:`OwnedEntry.batch`) and each of its ops is a sub-step of it (:meth:`OwnedEntry.op_step`):
a call the owner serves commits there, the Python tools' changes to the view before it
committed through ``bridge`` first (:meth:`OwnedEntry.flush_view`: so it edits them too; a
read of the owner's head flushes the same way), the rest when the batch ends -- a batch of
Python tools alone is one step of the view's, as before -- and a failed op is taken back to
where it began.  The batch ends as one step (``close_group``), or none where it
fails (the whole group taken back; undo and redo as they were).  The group is closed on every
path, an exception's too.  While it is open the view and the batch's reads follow the group's
head (:meth:`OwnedHistory.reading_head`).

**Numbers.**  :class:`OwnedHistory` numbers the document's states as
:class:`~ooxml_edit.history.History` does (a step a new number, undo and redo back to the
number of the state they bring back), so every version-keyed thing of the tool layer (refs,
owners, caches) means what it meant.  The steps are the owner's: an undo the owner does not
make as the numbers say is an error, never smoothed over.

Must not: keep a copy of the document beside the owner's, or read the view on a served call.
"""

from __future__ import annotations

import contextvars
import threading
from contextlib import ExitStack, contextmanager
from dataclasses import dataclass, field
from typing import Any, Callable, Iterator, Mapping

from ..history import Step
from .gate import Gate, GatedLock
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
        #: Steps are numbered under it, in the order the owner made them (lane P2c6).
        self._cond = threading.Condition()
        #: The calls the owner is serving now (:meth:`serving`).
        self._serving = 0
        #: While a step group is open (a batch's): the owner's head its ops left.
        self._group_head: Any = None

    @property
    def version(self) -> int:
        return self._number

    @property
    def last_version(self) -> int:
        return self._last

    @property
    def in_batch(self) -> bool:
        return self._entry.batch_depth > 0

    def at_head(self, then: Callable[[], None]) -> None:
        """``then()`` with no step numbered meanwhile (what is kept per version, by a call
        that made no step)."""
        with self._cond:
            then()

    def head(self, number: int | None = None) -> Any:
        """The owner's version numbered ``number`` (now: the latest), ``None`` when let go."""
        return self._heads.get(self._number if number is None else number)

    def reading_head(self) -> Any:
        """The owner's version the calls on the document read now: within a step group, the
        one its ops left; else the latest numbered one."""
        return self._group_head if self._group_head is not None else self.head()

    @property
    def grouped(self) -> bool:
        return self._group_head is not None

    def open_group(self) -> None:
        """A step group opened at the head."""
        with self._cond:
            self._group_head = self.head()

    def group_moved(self, head: Any) -> None:
        """An op of the open group left the owner's head at ``head``."""
        with self._cond:
            self._group_head = head

    def close_group(self, head: Any, first: int | None, view: Any = None) -> int | None:
        """The open group closed: ``head`` (the owner's versions from ``first`` on, ``None``
        where it made no step) numbered as one step, which is returned; else the version as
        it was, its head the owner's ``reading`` one now (a revert's, of the same content)."""
        with self._cond:
            now, self._group_head = self._group_head, None
            if head is not None:
                return self._committed(head, view, first)
            if now is not None and now is not self.head():
                self._heads[self._number] = now
                self._entry.owner.keep(self._entry.handle, self._number, now, None)
            return None

    @contextmanager
    def serving(self) -> Iterator[None]:
        """A call the owner serves (its edit made side by side with others', lane P2c6),
        from before its edit until its step is numbered (:meth:`served`) or it made none."""
        with self._cond:
            self._serving += 1
        try:
            yield
        finally:
            with self._cond:
                self._serving -= 1
                self._cond.notify_all()

    def served(self, first: int, head: Any, then: Any = None) -> tuple[int, int]:
        """The step a served call made -- the owner's versions ``first`` (its number) to
        ``head`` -- numbered once every version before ``first`` is: steps are numbered in
        the order the owner made them, whatever order their calls end in.  ``then(number,
        before)`` runs as it is numbered (what is counted per step, in that order).  The
        step's number and the number before it.  Within :meth:`serving`."""
        with self._cond:
            if self._group_head is not None:
                # (within a batch's group: the op's versions are the group's, numbered with it)
                if self._group_head.number + 1 != first:
                    raise OwnerDisagrees(f"{self._entry.doc_id}: a served step from the owner's "
                                         f"version {first} where the group is at "
                                         f"{self._group_head.number}")
                self._group_head = head
                self._cond.notify_all()
                return self._number, self._number
            while True:
                at = self._heads[self._number].number
                if at + 1 == first:
                    break
                if at + 1 > first or self._serving <= 1:
                    # (No other call in flight could number the versions between.)
                    raise OwnerDisagrees(f"{self._entry.doc_id}: a served step from the owner's "
                                         f"version {first} where the history is at {at}")
                self._cond.wait()
            before = self._number
            number = self._committed(head, None, first)
            if then is not None:
                then(number, before)
            self._cond.notify_all()
            return number, before

    def committed(self, head: Any, view: Any = None) -> int:
        """A step the owner made: its new head numbered, redo dropped; ``view``: the view
        that made it (a Python step), numbered as the head before the owner keeps it."""
        with self._cond:
            return self._committed(head, view, None)

    def _committed(self, head: Any, view: Any, first: int | None) -> int:
        self._undo.append(self._number)
        if len(self._undo) > DEPTH:
            del self._undo[0]
        self._redo.clear()
        self._last += 1
        self._number = self._last
        if view is not None:
            view.history.follow(self._number, self._last)
        self._keep(head, first)
        return self._number

    def _keep(self, head: Any, first: int | None = None) -> None:
        self._heads[self._number] = head
        kept = set(self._undo) | set(self._redo) | {self._number}
        for number in [n for n in self._heads if n not in kept]:
            del self._heads[number]
        self._entry.owner.keep(self._entry.handle, self._number, head, first)

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
        with self._cond:
            return self._move(self._entry.owner.undo, self._undo, self._redo)

    def redo(self) -> bool:
        with self._cond:
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
    #: Held shared by each call the owner serves, alone with the document's lock (:meth:`own`).
    gate: Gate = field(default_factory=Gate, repr=False)
    batch_depth: int = field(default=0, repr=False)
    #: The owner's number of the version the view is (``None``: none yet).
    _view_seq: int | None = field(default=None, repr=False)
    #: The owner's step group open for the batch now (``open_group``'s), else ``None``.
    _group: Any = field(default=None, repr=False)
    #: The batch's step owner while its group is open.
    _group_owner: StepOwner | None = field(default=None, repr=False)
    #: The op running in the group now: where a failure takes the owner back to, and the
    #: view's savepoint (:meth:`op_step`).
    _op_mark: Any = field(default=None, repr=False)
    _op_point: ExitStack | None = field(default=None, repr=False)
    _owned: OwnedHistory | None = field(default=None, repr=False)
    _step: ExitStack | None = field(default=None, repr=False)
    _view_change: Any = field(default=None, repr=False)
    _watches: list[OwnedChange] = field(default_factory=list, repr=False)

    owned = True

    def own(self, owner: Any, handle: Any) -> None:
        """The document (the one this entry was made with) is ``owner``'s, which holds it
        as ``handle`` (what its ``open`` gave).  Its lock holds its :attr:`gate` alone
        (:mod:`.gate`): the calls the owner serves hold it side by side."""
        self.lock = GatedLock(self.gate)
        document = self.held
        self.owner = owner
        self.handle = handle
        history = getattr(document, "history", None)
        version = getattr(history, "version", 0)
        last = getattr(history, "last_version", version)
        head = owner.head(self.handle)
        self._owned = OwnedHistory(self, head, version, last)
        self._view_seq = head.number

    # -- the view ---------------------------------------------------------------------------

    @property
    def document(self) -> Any:
        """The view at the head, made current now if the head moved since it last was."""
        owned = self._owned
        if owned is not None and self._view_seq != owned.reading_head().number:
            with self.lock:     # (a read that came before the lock: no edit meanwhile)
                head = owned.reading_head()
                if self._view_seq != head.number:
                    self.owner.follow(self.handle, self.held, head, owned.version,
                                      owned.last_version)
                    self._view_seq = head.number
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
        return self._owned is None or self._view_seq == self._owned.reading_head().number

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
        owner's already.  Where the owner has step groups, a step group of the owner's
        (see the module docstring), closed on every path."""
        outer = self.batch_depth == 0
        before = self.version
        self.batch_depth += 1
        try:
            if not outer:
                yield
            elif self._groups():
                with self._grouped(owner):
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

    def _groups(self) -> bool:
        """Whether the owner makes a batch one step of its own (step groups)."""
        return self._owned is not None and getattr(self.owner, "open_group", None) is not None

    @contextmanager
    def _grouped(self, owner: StepOwner | None) -> Iterator[None]:
        """The batch as one step group of the owner's: opened here, closed as one step where
        the batch ends, taken back whole where it raises -- the group closed either way."""
        owned = self._owned
        assert owned is not None
        group = self.owner.open_group(self.handle)
        first = owned.reading_head().number + 1
        caller = dict(owner.caller) if owner is not None else None
        self._group, self._group_owner = group, owner
        owned.open_group()
        token = _IN_STEP.set(_IN_STEP.get() | {id(self)})
        closed = False
        try:
            self._step = ExitStack()
            try:
                yield
                self._flush(owner)
            except BaseException as error:
                self._unwind(error)
                self._group_moved(self.owner.revert_group(self.handle, group, None, caller))
                raise
            finally:
                _IN_STEP.reset(token)
            head = self.owner.close_group(self.handle, group, caller)
            closed = True
            view = self.held if head is not None and self._view_seq == head.number else None
            owned.close_group(head, first, view)
        finally:
            self._group = self._group_owner = None
            if not closed:
                # (taken back, or the revert failed: the group closes so the document is not
                # left in it; the error that brought us here goes on)
                try:
                    owned.close_group(self.owner.close_group(self.handle, group, caller), first)
                except Exception:  # noqa: BLE001 -- the batch's own error is the one raised
                    owned.close_group(None, first)

    @contextmanager
    def op_step(self, owner: StepOwner | None = None) -> Iterator[None]:
        """One op of a batch, as a sub-step of the batch's step group (where it has one):
        everything the op committed, and what it changed in the view, taken back where it
        raises.  A Python tool's change stays in the view's step, which the batch's later
        Python ops go on in, as in a batch without the owner's tools; a call the owner
        serves, or a read of the owner's head, has it committed first (:meth:`flush_view`)."""
        group = self._group
        if group is None:
            yield
            return
        self._op_mark = self.owner.group_mark(self.handle, group)
        point = ExitStack()
        saved = self._view_change is not None
        if saved:
            # (the view's step is open, so the view is current: its savepoint is cheap)
            view = self.held
            savepoint = getattr(view, "savepoint", None)
            point.enter_context(savepoint() if savepoint is not None else view.history.savepoint())
        self._op_point = point
        try:
            yield
        except BaseException as error:
            if self._op_point is point and saved:
                point.__exit__(type(error), error, error.__traceback__)
            elif self._view_change is not None:
                self._unwind(error)       # (a view step this op opened)
            self._group_moved(self.owner.revert_group(
                self.handle, group, self._op_mark, dict(owner.caller) if owner is not None else None))
            raise
        else:
            if self._op_point is point:
                point.close()
        finally:
            self._op_point = None

    def flush_view(self) -> None:
        """Within a batch's step group: the view's changes so far committed in the group, so
        a call the owner serves edits them, or a read of the owner's head reads them; the
        op running now takes back only what it does from here on."""
        if self._group is None or self._view_change is None:
            return
        point, self._op_point = self._op_point, None
        if point is not None:
            point.close()         # (what the op changed so far stays: the step goes on)
        self._flush(self._group_owner)
        self._op_mark = self.owner.group_mark(self.handle, self._group)

    def _flush(self, owner: StepOwner | None) -> None:
        """Within a step group: the view's step closed and what it changed committed in the
        group (as ``owner``'s); a new one opens when the view is next read."""
        stack, self._step = self._step, ExitStack()
        if stack is None:
            return
        stack.close()
        self._end_step(owner)
        self._view_change = None

    def _unwind(self, error: BaseException) -> None:
        """Within a step group: the view's step taken back (its batch rolls back as it
        closes with ``error``), so the view is the version it followed last."""
        stack, self._step = self._step, ExitStack()
        self._view_change = None
        if stack is not None:
            stack.__exit__(type(error), error, error.__traceback__)

    def _group_moved(self, head: Any) -> None:
        if head is not None and self._owned is not None:
            self._owned.group_moved(head)

    def _end_step(self, owner: StepOwner | None) -> None:
        change, owned = self._view_change, self._owned
        if change is None or not change.changed or not change.parts or owned is None:
            return
        if self.owner.head(self.handle).number != owned.reading_head().number:
            raise OwnerDisagrees(f"{self.doc_id}: the owner's head moved during a Python step")
        package = self.held.package
        parts = {part: (package.read(part) if package.has_part(part) else None)
                 for part in change.parts}
        head = self.owner.bridge(self.handle, owner.tool if owner is not None else "library",
                                 dict(owner.caller) if owner is not None else None, parts)
        self._view_seq = head.number
        if owned.grouped:
            owned.group_moved(head)
        else:
            owned.committed(head, self.held)

    @contextmanager
    def serving(self) -> Iterator[None]:
        """A call the owner serves, from before its edit to its step (:meth:`served`)."""
        assert self._owned is not None
        with self._owned.serving():
            yield

    def served(self, first: int, head: Any, owner: StepOwner | None = None,
               counted: bool = False, refs: Mapping[str, str] | None = None) -> tuple[int, int]:
        """A call the owner served made its versions ``first`` (the owner's number) to
        ``head``: the call's step, numbered in the order the owner made it (within
        :meth:`serving`).  ``counted``: the call ran without the document's lock (lane P2c6),
        so the step is counted here as it is numbered -- the change and its parts, the
        step's owner, the refs the call defined (``refs``, owned by ``owner``'s caller), the
        committed version for reads -- which the dispatcher does under the lock otherwise.
        The step's number and the number before it."""
        assert self._owned is not None

        def count(number: int, before: int) -> None:
            change = OwnedChange(self)
            change.start, change.end = before, number
            self.keep_change(before, change)
            self.note_change(change.parts, before)
            if owner is not None:
                self.owners[number] = owner
                self.own_undos = [undo for undo in self.own_undos if undo.kind != "plain"]
            if refs:
                self.keep_refs(dict(refs), owner.caller if owner is not None else None)
            self.ref_states.setdefault(number, dict(self.refs))
            self.commit(None)

        return self._owned.served(first, head, count if counted else None)

    def served_refs(self, refs: Mapping[str, str], caller: Mapping[str, Any]) -> None:
        """The refs a served call that made no step defined (``caller``'s), kept under the
        version they name -- in the owner's order, as a step's are (:meth:`served`)."""
        assert self._owned is not None
        if refs:
            self._owned.at_head(lambda: self.keep_refs(dict(refs), caller))

    @contextmanager
    def savepoint(self, start: Any = None) -> Iterator[bool]:
        if self._group is not None:
            yield True      # (the op's sub-step of the group takes it back: :meth:`op_step`)
            return
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
