"""Documents whose owner is a store outside the tool layer (lane P2c4): a format's
:attr:`~.session.DocumentFormat.owner` (a library's native session) keeps the document's
versions, its steps, its ids and its saved bytes; the tool layer keeps no working copy of
its own.

**What the owner is.**  An object with these methods, each given the handle
:meth:`open` returned (the owner's per-document state):

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
- ``open_data(data, state, name) -> handle``: open a document's bytes (``state``: what was
  saved beside them, or ``None``) as ``name``, the owner alone -- no document of the tool
  layer's is made (lane H7c; :meth:`~.session.Session.open`).  A refused state is an
  :class:`~ooxml_edit.errors.ArgumentError`;
- ``parts_between(handle, before, after) -> Iterable[str]``: the parts whose content differs
  between two heads, and ``touched(handle, before, after) -> list[dict]``: what changed
  between them in the format's own addresses (``DocumentFormat.touched``'s entries);
- ``history(handle, name, steps, redo, caller) -> reply``: undo (``redo``: redo) up to
  ``steps`` steps of the document's history -- with a ``caller`` (a call's context), that
  caller's own -- by the owner's own undo tool, the document named ``name`` in its words: the
  tool's reply envelope (``summary``, ``changed``, ``data`` with ``steps`` and ``version``,
  ``warnings``; or ``error``).  The owner keeps who made each step (lane U4);
- ``keep(handle, head, first)``: the head is the document's version ``head.state`` (reads find it);
  ``first``: the owner's number of the step's first version, where it made several (each is that state);
- ``save(handle) -> bytes`` and ``close(handle)``;
- optionally, step groups (a ``batch``'s ops as one step of the owner's): ``open_group(handle) ->
  group``, ``group_mark(handle, group) -> mark``, ``revert_group(handle, group, mark, caller) -> head |
  None`` (what the group committed after ``mark`` -- ``None``: since it opened -- taken back, no
  redo of it; the new head, ``None`` when nothing was committed since) and ``close_group(handle,
  group, caller, tool) -> head | None`` (one step of ``caller``'s, made by ``tool``; ``None`` when it
  committed nothing).

**No Python document.**  The owner alone holds the document: the entry holds none
(:attr:`~.session.DocumentEntry.held` is ``None``), every changing call on it is the owner's
to serve (:meth:`OwnedEntry.served`), and a committed version has no Python reader copy (its
reads take the document's lock).

**Batches** (lane H1).  Where the owner has step groups, a batch on the document opens one
(:meth:`OwnedEntry.batch`) and each of its ops is a sub-step of it (:meth:`OwnedEntry.op_step`):
a call the owner serves commits there, and a failed op is taken back to where it began.  The
batch ends as one step (``close_group``), or none where it fails (the whole group taken back;
undo and redo as they were).  The group is closed on every path, an exception's too.  While it
is open the batch's reads follow the group's head (:meth:`OwnedHistory.reading_head`).

**The hooks' subject** (lane H7a).  A format's hooks that take a document (``summary``,
``problems``, ``state``, ``closed``, ``restored``, ...) are given, for a document its owner
holds, an :class:`OwnedDocument` -- the owner, its handle and the version -- never the
entry (:attr:`~.session.DocumentEntry.subject`): they read the owner.

**Numbers.**  The owner numbers the document's states as
:class:`~ooxml_edit.history.History` does (a step a new number, undo and redo back to the
number of the state they bring back), so every version-keyed thing of the tool layer (refs,
owners, caches) means what it meant; :class:`OwnedHistory` reads them (``head.state``) and
keeps no steps of its own.  Calls the owner serves side by side are kept and counted as they
end, in any order: what is counted per change is ordered by the owner's count of its
versions, never by the order the calls end in.

Must not: keep a copy of the document beside the owner's.
"""

from __future__ import annotations

import bisect
import threading
from contextlib import contextmanager
from dataclasses import dataclass, field
from collections import deque
from typing import Any, Callable, Iterable, Iterator, Mapping

from ..history import Step
from .gate import Gate, GatedLock
from .session import CHANGES_KEPT, Committed, DocumentEntry, StepOwner

#: The steps kept for undo, as the owner keeps them (``ooxml_core.Session``'s depth).
DEPTH = 50


@dataclass(frozen=True)
class OwnedDocument:
    """A document its format's owner holds, as the format's hooks are given it (lane H7a):
    the owner, the handle its ``open`` gave, and the version the hook reads -- ``None``: the
    version the calls on the document read now (within a batch's step group, the one its ops
    left); a number: that committed version (a read copy's, :attr:`~.session.DocumentEntry.
    pinned`)."""

    owner: Any
    handle: Any
    version: int | None = None


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

    def close_group(self, head: Any, first: int | None) -> int | None:
        """The open group closed: ``head`` (the owner's versions from ``first`` on, ``None``
        where it made no step) is its step, whose number is returned; else the version as it
        was, its head the owner's ``reading`` one now (a revert's, of the same state)."""
        now, self._group_head = self._group_head, None
        if head is not None:
            with self._lock:
                self._tell(head)
                self._entry.owner.keep(self._entry.handle, head, first)
                return head.state
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
               before: Any = None) -> tuple[int, Any]:
        """The step a served call made -- the owner's versions ``first`` to ``head``, from the
        owner's version ``before`` (held, as the owner handed it with the edit; ``None``: the
        version now) -- kept as its state, whatever order the calls end in.  ``then(number,
        before, seq)`` runs as it is kept (what is counted per step, ``before`` the version
        held; ``seq`` the owner's count of ``head``).  The step's number and the version
        before it, held: what the step changed is read between it and ``head``, never by its
        number (other calls' steps may have let it go from the owner's history since)."""
        if self._group_head is not None:
            # (within a batch's group: the op's versions are the group's, numbered with it)
            self._group_head = head
            return self.version, self._head
        with self._lock:
            if before is None:
                before = self._head
            self._tell(head)
            number = head.state
            self._entry.owner.keep(self._entry.handle, head, first)
            if then is not None:
                then(number, before, head.number)
            return number, before

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
    parts between the heads before and after (asked of the owner).  A served step's change
    is given its two heads, held (``heads``): its parts are read between them, never looked
    up by number again (the owner may have let the one before go by then)."""

    def __init__(self, entry: "OwnedEntry", heads: tuple[Any, Any] | None = None) -> None:
        self._entry = entry
        self._heads = heads
        self.start = entry.version if heads is None else heads[0].state
        self.end: int | None = None if heads is None else heads[1].state
        self._parts: frozenset[str] | None = None

    @property
    def changed(self) -> bool:
        return (self._entry.version if self.end is None else self.end) != self.start

    @property
    def parts(self) -> frozenset[str]:
        if self._parts is None:
            history = self._entry.history
            end = history.version if self.end is None else self.end
            if self._heads is not None:
                before, after = self._heads
            else:
                before, after = history.head(self.start), history.head(end)
            if end == self.start or before is None or after is None:
                self._parts = frozenset()
            else:
                self._parts = frozenset(self._entry.owner.parts_between(
                    self._entry.handle, before, after))
        return self._parts

    def before(self, path: str) -> bytes | None:
        """(The owner keeps the parts' bytes: none here.)"""
        return None

    def after(self, path: str) -> bytes | None:
        """(The owner keeps the parts' bytes: none here.)"""
        return None


@dataclass(eq=False)
class OwnedEntry(DocumentEntry):
    """A document the format's owner holds (see the module docstring)."""

    owner: Any = field(default=None, repr=False)
    handle: Any = field(default=None, repr=False)
    #: Held shared by each call the owner serves, alone with the document's lock (:meth:`own`).
    gate: Gate = field(default_factory=Gate, repr=False)
    batch_depth: int = field(default=0, repr=False)
    #: The owner's step group open for the batch now (``open_group``'s), else ``None``.
    _group: Any = field(default=None, repr=False)
    _owned: OwnedHistory | None = field(default=None, repr=False)
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
        self._counted_seq = head.number

    # -- what the hooks read ----------------------------------------------------------------

    @property
    def subject(self) -> Any:
        """What the format's hooks are given (:class:`OwnedDocument`, at the version the
        calls read now: within a batch's step group, the one its ops left)."""
        if self._owned is None:
            return self.held
        return OwnedDocument(self.owner, self.handle, None)

    def subject_at(self, version: int) -> OwnedDocument:
        """:attr:`subject` at the committed version ``version`` (a read copy's)."""
        return OwnedDocument(self.owner, self.handle, version)

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

    @contextmanager
    def batch(self, owner: StepOwner | None = None) -> Iterator[None]:
        """One step around a call: a served call's step is the owner's already.  Where the
        owner has step groups, a batch is one step group of the owner's (see the module
        docstring), closed on every path."""
        outer = self.batch_depth == 0
        self.batch_depth += 1
        try:
            if outer and self._groups():
                with self._grouped(owner):
                    yield
            else:
                yield
        finally:
            self.batch_depth -= 1

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
        self._group = group
        owned.open_group()
        closed = False
        try:
            try:
                yield
            except BaseException:
                self._group_moved(self.owner.revert_group(self.handle, group, None, caller))
                raise
            head = self.owner.close_group(self.handle, group, caller, tool)
            closed = True
            owned.close_group(head, first)
        finally:
            self._group = None
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
        everything the op committed taken back where it raises."""
        group = self._group
        if group is None:
            yield
            return
        mark = self.owner.group_mark(self.handle, group)
        try:
            yield
        except BaseException:
            self._group_moved(self.owner.revert_group(
                self.handle, group, mark, dict(owner.caller) if owner is not None else None))
            raise

    def _group_moved(self, head: Any) -> None:
        if head is not None and self._owned is not None:
            self._owned.group_moved(head)

    @contextmanager
    def serving(self) -> Iterator[None]:
        """A call the owner serves, from before its edit to its step (:meth:`served`)."""
        assert self._owned is not None
        with self._owned.serving():
            yield

    def served(self, first: int, head: Any, owner: StepOwner | None = None,
               counted: bool = False, refs: Mapping[str, str] | None = None,
               before: Any = None) -> tuple[int, Any]:
        """A call the owner served made its versions ``first`` (the owner's number) to
        ``head``, from the owner's version ``before`` (held, as the owner handed it with the
        edit; ``None``: the version now): the call's step, kept whatever order the calls end in.  ``counted``: the call ran without the
        document's lock (lane P2c6), so the step is counted here as it is kept -- the change
        and its parts, the step's owner, the refs the call defined (``refs``, owned by
        ``owner``'s caller), the committed version for reads -- which the dispatcher does
        under the lock otherwise.  The step's number and the owner's version before it, held
        (:meth:`OwnedHistory.served`)."""
        assert self._owned is not None

        def count(number: int, from_head: Any, seq: int) -> None:
            change = OwnedChange(self, (from_head, head))
            before = change.start
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
        """Within a batch's step group, the op's sub-step of the group takes back what a
        failing op committed (:meth:`op_step`); outside one, a served call that fails commits
        nothing, and the tool layer has nothing of its own to take back."""
        yield self._group is not None

    @contextmanager
    def watch(self) -> Iterator[Any]:
        change = OwnedChange(self)
        try:
            yield change
        finally:
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
        after it (:meth:`~.session.DocumentEntry.reader`): the owner's version, its bytes made
        only when a read first asks.  No Python reader copy is made of it
        (:func:`_no_reader_copy`): its reads take the document's lock."""
        owned = self._owned
        committed = self.committed
        version = self.version
        if owned is None:
            return
        if committed is not None and committed.version == version:
            if committed.refs != self.refs:
                self.committed = committed.with_refs(
                    dict(self.refs), dict(self.ref_states.get(version, self.refs)),
                    dict(self.ref_owners))
                self._told()
            return
        head = owned.head()
        self.committed = Committed(version, head.save, _no_reader_copy,
                                   dict(self.refs), dict(self.ref_states.get(version, self.refs)),
                                   owners=dict(self.owners),
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
        if self.owner is not None:
            self.owner.close(self.handle)


def _no_reader_copy(data: Any) -> Any:
    """(A document its owner holds has no Python reader copy: :meth:`Committed.entry` finds
    none, and the reads of a committed version take the document's lock.)"""
    raise LookupError("a document its owner holds has no Python reader copy")


__all__ = ["DEPTH", "OwnedChange", "OwnedDocument", "OwnedEntry", "OwnedHistory"]
