"""Where a call's time went (round 58): waiting for its documents, or working.

A document's lock (:attr:`~.session.DocumentEntry.lock`) is a :class:`TimedLock`: a
re-entrant lock that counts, for the thread that takes it, the seconds it waited to get it
and the seconds the thread held any document's lock -- from its first lock taken to its last
let go, however many it took or took again.  :func:`clock` reads the thread's counts, and the
dispatcher reports the difference over a call as the result's :attr:`~.results.Result.timing`
(for the application; never in the model's JSON):

* ``total`` -- the call, as :attr:`~.results.Result.duration`;
* ``lock_wait`` -- waiting for its documents' locks, or their committed copies' (an edit in
  flight, a read making the copy of a version) -- every time it took one, again after work
  done with them let go included;
* ``work`` -- the rest: ``total - lock_wait``;
* ``cpu`` -- the thread's own CPU in that time (``time.thread_time``), so ``work - cpu`` is
  what the call worked without running: waiting for a worker process's answer, for files, or
  for the interpreter while other threads held it;
* ``held`` -- how long the call held its documents' locks: what it made other edits of them
  wait for.

The counts are per thread, so a call's numbers are its own whatever runs beside it.
"""

from __future__ import annotations

import threading
import time
from typing import Any


class _Counts(threading.local):
    """One thread's counts: seconds waited, seconds held, and the locks it holds now."""

    def __init__(self) -> None:
        self.waited = 0.0
        self.held = 0.0
        self.depth = 0
        self.since = 0.0


_COUNTS = _Counts()


def clock() -> tuple[float, float]:
    """This thread's ``(seconds waited for a document lock, seconds it held one)`` so far,
    a lock it holds now counted up to now."""
    counts = _COUNTS
    held = counts.held + (time.perf_counter() - counts.since if counts.depth else 0.0)
    return counts.waited, held


class TimedLock:
    """A re-entrant lock (:class:`threading.RLock`'s protocol, ``_is_owned`` included) that
    counts the taking thread's wait and hold (the module's notes)."""

    __slots__ = ("_lock",)

    def __init__(self) -> None:
        self._lock = threading.RLock()

    def acquire(self, blocking: bool = True, timeout: float = -1) -> bool:
        counts = _COUNTS
        began = time.perf_counter()
        got = self._lock.acquire(blocking, timeout)
        if got:
            now = time.perf_counter()
            counts.waited += now - began
            if counts.depth == 0:
                counts.since = now
            counts.depth += 1
        return got

    def release(self) -> None:
        self._lock.release()
        counts = _COUNTS
        counts.depth -= 1
        if counts.depth == 0:
            counts.held += time.perf_counter() - counts.since

    def _is_owned(self) -> bool:
        return self._lock._is_owned()  # type: ignore[attr-defined]

    def __enter__(self) -> bool:
        return self.acquire()

    def __exit__(self, *exc: Any) -> None:
        self.release()

    def __repr__(self) -> str:
        return f"<TimedLock {self._lock!r}>"


class CallClock:
    """A call's :attr:`~.results.Result.timing`, measured from its start to :meth:`timing`."""

    __slots__ = ("_wall", "_cpu", "_waited", "_held")

    def __init__(self) -> None:
        self._wall = time.perf_counter()
        self._cpu = time.thread_time()
        self._waited, self._held = clock()

    def timing(self) -> dict[str, float]:
        waited, held = clock()
        total = time.perf_counter() - self._wall
        lock_wait = waited - self._waited
        return {"total": round(total, 4), "lock_wait": round(lock_wait, 4),
                "work": round(max(0.0, total - lock_wait), 4),
                "cpu": round(time.thread_time() - self._cpu, 4), "held": round(held - self._held, 4)}
