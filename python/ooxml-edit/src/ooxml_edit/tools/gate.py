"""Who may change a document its owner holds, side by side (lane P2c6).

A document whose owner commits edits concurrently (:class:`~.owned.OwnedEntry`: the Rust
session prepares each edit on a snapshot and commits it in a short section of its own) lets
the calls it serves itself run side by side: each holds the document's :class:`Gate`
*shared* -- never the document's lock -- for as long as its edit runs and its step is
numbered.  Everything else that changes the document through the tool layer holds the gate
*exclusively*: a Python tool's step on the view (made current from the head, then committed
by the owner as one step), an undo or redo, the application's own changes.  It takes it with
the document's lock (:class:`GatedLock`), so every holder of that lock holds the document
alone, as before.

The gate prefers the exclusive side: once one waits, new shared holders wait behind it, so a
Python step is never starved by a stream of served edits.

Must not: be taken shared by a thread that holds the document's lock, or the lock taken by a
thread that holds the gate shared (each waits for the other); hold anything but counts.
"""

from __future__ import annotations

import threading
import time
from contextlib import contextmanager
from typing import Iterator

from .timing import TimedLock, count_wait


class Gate:
    """A shared/exclusive gate over one document, the exclusive side first."""

    __slots__ = ("_cond", "_shared", "_exclusive", "_waiting")

    def __init__(self) -> None:
        self._cond = threading.Condition(threading.Lock())
        #: The threads holding it shared (a thread may hold it more than once).
        self._shared: dict[int, int] = {}
        self._exclusive = False
        self._waiting = 0

    @contextmanager
    def shared(self) -> Iterator[None]:
        """Held beside other shared holders, while no one holds it exclusively."""
        began = time.perf_counter()
        me = threading.get_ident()
        with self._cond:
            while (self._exclusive or self._waiting) and me not in self._shared:
                self._cond.wait()
            self._shared[me] = self._shared.get(me, 0) + 1
        count_wait(time.perf_counter() - began)
        try:
            yield
        finally:
            with self._cond:
                left = self._shared.pop(me) - 1
                if left:
                    self._shared[me] = left
                elif not self._shared:
                    self._cond.notify_all()

    def acquire_exclusive(self, blocking: bool = True, timeout: float = -1) -> bool:
        """Held alone: once the shared holders let it go (new ones wait meanwhile)."""
        deadline = None if timeout < 0 else time.monotonic() + timeout
        with self._cond:
            if threading.get_ident() in self._shared:
                raise RuntimeError("a thread holding a document's gate shared took its lock")
            if not blocking and (self._exclusive or self._shared):
                return False
            self._waiting += 1
            try:
                while self._exclusive or self._shared:
                    left = None if deadline is None else deadline - time.monotonic()
                    if left is not None and left <= 0:
                        return False
                    self._cond.wait(left)
                self._exclusive = True
                return True
            finally:
                self._waiting -= 1
                if not self._exclusive:
                    self._cond.notify_all()

    def release_exclusive(self) -> None:
        with self._cond:
            self._exclusive = False
            self._cond.notify_all()


class GatedLock(TimedLock):
    """A document's lock (:class:`~.timing.TimedLock`) that holds its :class:`Gate`
    exclusively while any thread holds the lock: taken first, let go last."""

    __slots__ = ("gate",)

    def __init__(self, gate: Gate) -> None:
        super().__init__()
        self.gate = gate

    def acquire(self, blocking: bool = True, timeout: float = -1) -> bool:
        if self._is_owned():
            return super().acquire(blocking, timeout)
        began = time.perf_counter()
        if not self.gate.acquire_exclusive(blocking, timeout):
            return False
        count_wait(time.perf_counter() - began)
        if not super().acquire(blocking, timeout):
            self.gate.release_exclusive()
            return False
        return True

    def release(self) -> None:
        super().release()
        if not self._is_owned():
            self.gate.release_exclusive()


__all__ = ["Gate", "GatedLock"]
