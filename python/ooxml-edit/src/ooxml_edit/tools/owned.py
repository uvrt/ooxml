"""Documents whose owner is a store outside the tool layer (lane P2c4): a format's
:attr:`~.session.DocumentFormat.owner` (a library's native session) keeps the document's
versions, its steps, its ids and its saved bytes; the tool layer keeps no working copy of
its own.

**What the owner is.**  An object with these methods, each given the handle
:meth:`open` returned (the owner's per-document state):

- ``open(document) -> handle | None``: hold ``document`` (as opened, its bytes and ids) from
  now on; ``None``: the owner does not take it (the tool layer then holds it itself);
- ``head(handle) -> head``: the document's latest version, a value whose ``number`` is the
  owner's own count of its versions (a new one for every version it makes) and whose
  ``state`` is the number of its state as the tool layer numbers states (the state opened 0,
  a step one more than the highest given, an undo or redo the number of the state it brings
  back: equal numbers, equal content);
- ``numbered(handle, number) -> head | None``: the state numbered ``number`` the head
  reaches (the newest such), ``None`` when let go; ``highest(handle) -> int``: the highest
  state number given; ``numbers(handle) -> (highest, steps, redo)``: those and the steps undo
  can reach (oldest first) and redo can bring back (the next last), each ``(before, after,
  seq, caller)`` in state numbers; ``continue_numbers(handle, state, highest)``: before the
  document's first step, its head is the state ``state`` and the next step's number one more
  than ``highest`` (a document opened again goes on with its numbers);
- ``opened(handle) -> (version, last)``: the state number the document was opened at and the
  highest its history gave (``(0, 0)``: none; :meth:`OwnedEntry.own` goes on from them);
- optionally, ``open_data(data, state, name) -> handle | None``: open a document's bytes
  (``state``: what was saved beside them, or ``None``) as ``name``, the owner alone -- no
  document of the tool layer's is made (lane H7c; :meth:`~.session.Session.open`); ``None``:
  the format opens the bytes and the owner takes them through ``open``.  A refused state is
  an :class:`~ooxml_edit.errors.ArgumentError`;
- ``parts_between(handle, before, after) -> Iterable[str]``: the parts whose content differs
  between two heads, and ``touched(handle, before, after) -> list[dict]``: what changed
  between them in the format's own addresses (``DocumentFormat.touched``'s entries);
- ``follow(handle, document, head, number, last)``: bring ``document`` -- the view -- to
  ``head``, numbered ``number`` (``last``: the highest number given), its ids too;
- ``bridge(handle, tool, caller, parts) -> head``: commit the parts a Python tool changed in
  the view (bytes, ``None`` removes one) as the step of ``tool`` and ``caller``;
- ``history(handle, name, steps, redo, caller) -> reply``: undo (``redo``: redo) up to
  ``steps`` steps of the document's history -- with a ``caller`` (a call's context), that
  caller's own -- by the owner's own undo tool, the document named ``name`` in its words: the
  tool's reply envelope (``summary``, ``changed``, ``data`` with ``steps`` and ``version``,
  ``warnings``; or ``error``).  The owner keeps who made each step (lane U4);
- ``keep(handle, head, first)``: the head is the document's version ``head.state`` (reads find it);
  ``first``: the owner's number of the step's first version, where it made several (each is that state);
- ``reader(handle, head, number) -> make``: what makes a Python reader of the version ``head``
  (numbered ``number``) from its bytes, for the reads of a committed version (``head.save()``);
- ``save(handle) -> bytes`` and ``close(handle)``;
- optionally, step groups (a ``batch``'s ops as one step of the owner's): ``open_group(handle) ->
  group``, ``group_mark(handle, group) -> mark``, ``revert_group(handle, group, mark, caller) -> head |
  None`` (what the group committed after ``mark`` -- ``None``: since it opened -- taken back, no
  redo of it; the new head, ``None`` when nothing was committed since) and ``close_group(handle,
  group, caller, tool) -> head | None`` (one step of ``caller``'s, made by ``tool``; ``None`` when it
  committed nothing).

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

**The hooks' subject** (lane H7a).  A format's hooks that take a document (``summary``,
``problems``, ``state``, ``closed``, ``restored``, ...) are given, for a document its owner
holds, an :class:`OwnedDocument` -- the owner, its handle and the version -- never the
entry or the view (:attr:`~.session.DocumentEntry.subject`): they read the owner.

**Numbers.**  The owner numbers the document's states as
:class:`~ooxml_edit.history.History` does (a step a new number, undo and redo back to the
number of the state they bring back), so every version-keyed thing of the tool layer (refs,
owners, caches) means what it meant; :class:`OwnedHistory` reads them (``head.state``) and
keeps no steps of its own.  Calls the owner serves side by side are kept and counted as they
end, in any order: what is counted per change is ordered by the owner's count of its
versions, never by the order the calls end in.

Must not: keep a copy of the document beside the owner's, or read the view on a served call.
"""

from __future__ import annotations

import bisect
import contextvars
import threading
from contextlib import ExitStack, contextmanager
from dataclasses import dataclass, field
from collections import deque
from typing import Any, Callable, Iterable, Iterator, Mapping

from ..history import Step
from .gate import Gate, GatedLock
from .session import CHANGES_KEPT, Committed, DocumentEntry, StepOwner

#: The steps kept for undo, as the owner keeps them (``ooxml_core.Session``'s depth).
DEPTH = 50

#: The entries (``id``) whose call's step is open in this context: only that call's reads of
#: the view open the view's step (a read made meanwhile by another call does not).
_IN_STEP: contextvars.ContextVar[frozenset[int]] = contextvars.ContextVar("ooxml_edit_owned_step",
                                                                           default=frozenset())


@dataclass(frozen=True)
class OwnedDocument:
    """A document its format's owner holds, as the format's hooks are given it (lane H7a):
    the owner, the handle its ``open`` gave, and the version the hook reads -- ``None``: the
    version the calls on the document read now (within a batch's step group, the one its ops
    left); a number: that committed version (a read copy's, :attr:`~.session.DocumentEntry.
    pinned`).  Never the view: a hook reads the owner."""

    owner: Any
    handle: Any
    version: int | None = None


class OwnerDisagrees(RuntimeError):
    """The owner did what the tool layer's numbers say it would not (an undo it did not make,
    a step where none was expected): the two would say different things from now on."""


class OwnedHistory:
    """The numbers of an owned document's states and its undo and redo, as the owner keeps
    them (the protocol the tool layer reads of a :class:`~ooxml_edit.history.History`): the
    owner numbers each state (``head.state``), lists its steps and finds a state by its
    number; this keeps only the newest head it was told of and the head of a step group open
    now.  Calls the owner serves side by side end in any order: a head is newer by the
    owner's own count (``head.number``), never by the order it is told in."""

    compares = False

    def __init__(self, entry: "OwnedEntry", head: Any) -> None:
        self._entry = entry
        self._head = head
        #: Only around moving the head on (never waited on for an order).
        self._lock = threading.Lock()
        #: While a step group is open (a batch's): the owner's head its ops left.
        self._group_head: Any = None

    @property
    def version(self) -> int:
        return self._head.state

    @property
    def last_version(self) -> int:
        return max(self.version, self._entry.owner.highest(self._entry.handle))

    @property
    def in_batch(self) -> bool:
        return self._entry.batch_depth > 0

    def at_head(self, then: Callable[[], None]) -> None:
        """``then()`` with no head told meanwhile (what is kept per version, by a call that
        made no step)."""
        with self._lock:
            then()

    def head(self, number: int | None = None) -> Any:
        """The owner's state numbered ``number`` (now: the latest), ``None`` when let go."""
        head = self._head
        if number is None or number == head.state:
            return head
        return self._entry.owner.numbered(self._entry.handle, number)

    def reading_head(self) -> Any:
        """The owner's version the calls on the document read now: within a step group, the
        one its ops left; else the latest one told."""
        return self._group_head if self._group_head is not None else self._head

    @property
    def grouped(self) -> bool:
        return self._group_head is not None

    def open_group(self) -> None:
        """A step group opened at the head."""
        self._group_head = self._head

    def group_moved(self, head: Any) -> None:
        """An op of the open group left the owner's head at ``head``."""
        self._group_head = head

    def close_group(self, head: Any, first: int | None, view: Any = None) -> int | None:
        """The open group closed: ``head`` (the owner's versions from ``first`` on, ``None``
        where it made no step) is its step, whose number is returned; else the version as it
        was, its head the owner's ``reading`` one now (a revert's, of the same state)."""
        now, self._group_head = self._group_head, None
        if head is not None:
            return self._committed(head, view, first)
        if now is not None:
            with self._lock:
                if self._tell(now):
                    self._entry.owner.keep(self._entry.handle, now, None)
        return None

    @contextmanager
    def serving(self) -> Iterator[None]:
        """A call the owner serves (its edit made side by side with others', lane P2c6)."""
        yield

    def served(self, first: int, head: Any, then: Any = None,
               before: int | None = None) -> tuple[int, int]:
        """The step a served call made -- the owner's versions ``first`` to ``head``, from the
        state numbered ``before`` (``None``: the version now) -- kept as its state, whatever
        order the calls end in.  ``then(number, before, seq)`` runs as it is kept (what is
        counted per step; ``seq`` the owner's count of ``head``).  The step's number and the
        number before it."""
        if self._group_head is not None:
            # (within a batch's group: the op's versions are the group's, numbered with it)
            self._group_head = head
            return self.version, self.version
        with self._lock:
            if before is None:
                before = self.version
            self._tell(head)
            number = head.state
            self._entry.owner.keep(self._entry.handle, head, first)
            if then is not None:
                then(number, before, head.number)
            return number, before

    def committed(self, head: Any, view: Any = None) -> int:
        """A step the owner made: its head kept; ``view``: the view that made it (a Python
        step), numbered as the head before the owner keeps it."""
        return self._committed(head, view, None)

    def _committed(self, head: Any, view: Any, first: int | None) -> int:
        with self._lock:
            self._tell(head)
            number = head.state
            if view is not None:
                view.history.follow(number, max(number, self._entry.owner.highest(self._entry.handle)))
            self._entry.owner.keep(self._entry.handle, head, first)
            return number

    def _tell(self, head: Any) -> bool:
        """``head`` the latest where the owner made it after the one held; whether it is."""
        if head.number <= self._head.number:
            return False
        self._head = head
        return True

    def steps(self) -> list[Step]:
        _highest, steps, _redo = self._entry.owner.numbers(self._entry.handle)
        return [Step(before, after) for before, after, _seq, _caller in steps]

    def can_undo(self) -> bool:
        return bool(self._entry.owner.numbers(self._entry.handle)[1])

    def can_redo(self) -> bool:
        return bool(self._entry.owner.numbers(self._entry.handle)[2])

    @property
    def next_redo(self) -> int | None:
        """The number of the state redo brings back next (``None``: none)."""
        redo = self._entry.owner.numbers(self._entry.handle)[2]
        return redo[-1][1] if redo else None

    def moved(self, head: Any) -> None:
        """The owner's undo or redo left the document at ``head``: kept as its state."""
        with self._lock:
            self._tell(head)
            self._entry.owner.keep(self._entry.handle, head, None)

    def mark(self) -> Any:
        return None

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
    #: The owner's version each of :attr:`changes_kept` ends at, in the same order.
    _kept_seqs: deque = field(default_factory=lambda: deque(maxlen=CHANGES_KEPT), repr=False)
    #: The owner's newest version whose change is counted.
    _counted_seq: int = field(default=0, repr=False)

    owned = True

    def own(self, owner: Any, handle: Any) -> None:
        """The document (the one this entry was made with) is ``owner``'s, which holds it
        as ``handle`` (what its ``open`` gave).  Its lock holds its :attr:`gate` alone
        (:mod:`.gate`): the calls the owner serves hold it side by side."""
        self.lock = GatedLock(self.gate)
        self.owner = owner
        self.handle = handle
        version, last = owner.opened(handle)
        if last:
            # (a document opened again: the owner goes on with the numbers it had)
            owner.continue_numbers(self.handle, version, last)
        head = owner.head(self.handle)
        self._owned = OwnedHistory(self, head)
        self._view_seq = self._counted_seq = head.number

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
    def subject(self) -> Any:
        """What the format's hooks are given (:class:`OwnedDocument`, at the version the
        calls read now).  Within a batch's step group, the view's changes so far are
        committed in the group first (:meth:`flush_view`), so the owner's version is the
        document as the batch left it."""
        if self._owned is None:
            return self.held
        self.flush_view()
        return OwnedDocument(self.owner, self.handle, None)

    def subject_at(self, version: int) -> OwnedDocument:
        """:attr:`subject` at the committed version ``version`` (a read copy's)."""
        return OwnedDocument(self.owner, self.handle, version)

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

    @property
    def owns_history(self) -> bool:
        return self._owned is not None

    def step_target(self, direction: str) -> Any:
        """(Undo and redo are the owner's: :meth:`owned_history`.)"""
        return None

    def owned_history(self, steps: int, redo: bool, caller: Mapping[str, Any] | None) -> dict[str, Any]:
        """Undo or redo up to ``steps`` steps by the owner's undo tool (``caller``'s own, with
        one), under the document's lock: its reply; the state it left kept as the head."""
        assert self._owned is not None
        reply = self.owner.history(self.handle, self.doc_id, steps, redo, caller)
        if reply.get("ok"):
            number = (reply.get("data") or {}).get("version")
            head = self.owner.numbered(self.handle, number) if number is not None else None
            self._owned.moved(head if head is not None else self.owner.head(self.handle))
        return reply

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
        tool = owner.tool if owner is not None else None
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
            head = self.owner.close_group(self.handle, group, caller, tool)
            closed = True
            view = self.held if head is not None and self._view_seq == head.number else None
            owned.close_group(head, first, view)
        finally:
            self._group = self._group_owner = None
            if not closed:
                # (taken back, or the revert failed: the group closes so the document is not
                # left in it; the error that brought us here goes on)
                try:
                    owned.close_group(self.owner.close_group(self.handle, group, caller, tool), first)
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
               counted: bool = False, refs: Mapping[str, str] | None = None,
               before: int | None = None) -> tuple[int, int]:
        """A call the owner served made its versions ``first`` (the owner's number) to
        ``head``, from the state numbered ``before`` (``None``: the version now): the call's
        step, kept whatever order the calls end in.  ``counted``: the call ran without the
        document's lock (lane P2c6), so the step is counted here as it is kept -- the change
        and its parts, the step's owner, the refs the call defined (``refs``, owned by
        ``owner``'s caller), the committed version for reads -- which the dispatcher does
        under the lock otherwise.  The step's number and the number before it."""
        assert self._owned is not None

        def count(number: int, before: int, seq: int) -> None:
            change = OwnedChange(self)
            change.start, change.end = before, number
            newest = self._count_at(seq, before, change)
            if newest:
                if refs:
                    self.keep_refs(dict(refs), owner.caller if owner is not None else None)
                self.ref_states.setdefault(number, dict(self.refs))
            else:
                # (a step older than one counted already: the refs are the newer state's
                # too, its own state's are those it began from and its own)
                if refs:
                    self.keep_refs(dict(refs), owner.caller if owner is not None else None)
                self.ref_states.setdefault(number, {**self.ref_states.get(before, {}), **dict(refs or {})})
            self.commit(None)

        return self._owned.served(first, head, count if counted else None, before)

    # -- the changes counted, in the owner's order ------------------------------------------

    def _seq_now(self) -> int:
        assert self._owned is not None
        return self._owned.reading_head().number

    def _keep_at(self, seq: int, kept: tuple[Any, Any, Any]) -> None:
        """``kept`` into :attr:`changes_kept` where the owner's version ``seq`` puts it."""
        changes, seqs = self.changes_kept, self._kept_seqs
        if changes.maxlen is not None and len(changes) >= changes.maxlen:
            changes.popleft()
            seqs.popleft()
        at = bisect.bisect_right(seqs, seq)
        changes.insert(at, kept)
        seqs.insert(at, seq)

    def _count_at(self, seq: int, before: int, change: "OwnedChange") -> bool:
        """A served step's change counted (:meth:`keep_change` and :meth:`note_change`, in
        the owner's order of its versions, ``seq`` the step's last): never taken for a change
        made outside the tools because a step the owner made before it ends after it.
        Whether it is the newest counted."""
        parts = change.parts
        with self._flight_lock:
            self._keep_at(seq, (before, change.end, change))
        self.change_seq += 1
        for part in parts:
            self.part_stamps[part] = self.change_seq
        if seq <= self._counted_seq:
            return False
        self._counted_seq = seq
        self.change_version = change.end
        return True

    def keep_change(self, before: int | None, change: Any) -> None:
        """(Under the document's lock: no call the owner serves is in flight, so the head is
        the newest version.)"""
        seq = self._seq_now()
        with self._flight_lock:
            if before is not None and self.change_version is not None and before != self.change_version:
                self._keep_at(seq, (self.change_version, before, None))
            self._keep_at(seq, (before, self.version, change))

    def note_change(self, parts: Iterable[str] | None, before: int | None) -> None:
        super().note_change(parts, before)
        self._counted_seq = max(self._counted_seq, self._seq_now())

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


__all__ = ["DEPTH", "OwnedChange", "OwnedDocument", "OwnedEntry", "OwnedHistory", "OwnerDisagrees"]
