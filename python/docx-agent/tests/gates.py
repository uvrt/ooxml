"""Order by events, not clocks: a step held until the test lets it go, and the order of the
steps as they happened.

A concurrency test says "the read did not wait for the layout" by holding the layout until
the read is back, then letting it go -- not by stalling the layout a second and comparing
clock readings taken in two threads, which a busy machine turns around.  A held step that is
never let go ends after :data:`TIMEOUT` seconds: a call that waited for it then comes back
after it, and the test fails on the order -- it never passes slowly nor fails by chance."""

from __future__ import annotations

import threading

#: How long a held step waits to be let go: only a test that is failing waits this long.
TIMEOUT = 60.0


class Gate:
    """Steps held at :meth:`hold` until :meth:`open`; :attr:`order`, what happened in turn."""

    def __init__(self, timeout: float = TIMEOUT) -> None:
        self.timeout = timeout
        self.order: list[str] = []
        self.entered = threading.Event()
        self._opened = threading.Event()
        self._lock = threading.Lock()

    def note(self, what: str) -> None:
        """``what`` happened now (after everything noted before it)."""
        with self._lock:
            self.order.append(what)

    def hold(self, name: str = "held") -> bool:
        """Note ``"<name> in"``, wait to be let go, note ``"<name> out"`` (or ``"<name>
        timed out"``); whether it was let go."""
        self.note(f"{name} in")
        self.entered.set()
        opened = self._opened.wait(self.timeout)
        self.note(f"{name} out" if opened else f"{name} timed out")
        return opened

    def wait_entered(self) -> None:
        assert self.entered.wait(self.timeout), "nothing reached the gate"

    def open(self) -> None:
        self._opened.set()

    @property
    def is_open(self) -> bool:
        return self._opened.is_set()

    def before(self, first: str, then: str) -> bool:
        """Whether ``first`` was noted, and before ``then`` (or ``then`` not at all)."""
        with self._lock:
            order = list(self.order)
        if first not in order:
            return False
        return then not in order or order.index(first) < order.index(then)


class Watched(threading.Condition):
    """A condition variable that counts the threads waiting on it now: put in place of an
    object's own (``owner._cond``) to know that a caller is waiting there, not merely that
    it was started.  :meth:`until_waiting` waits for that many."""

    def __init__(self) -> None:
        super().__init__()
        self.waiting = 0
        self._seen = threading.Condition()

    def wait(self, timeout=None):
        with self._seen:
            self.waiting += 1
            self._seen.notify_all()
        try:
            return super().wait(timeout)
        finally:
            with self._seen:
                self.waiting -= 1
                self._seen.notify_all()

    def until_waiting(self, count: int, timeout: float = TIMEOUT) -> bool:
        """Whether ``count`` callers were waiting at once within ``timeout`` seconds."""
        with self._seen:
            return self._seen.wait_for(lambda: self.waiting >= count, timeout)


class Arrivals:
    """A count of callers that reached a point (:meth:`arrive`), and a wait for that many."""

    def __init__(self) -> None:
        self.count = 0
        self._cond = threading.Condition()

    def arrive(self) -> None:
        with self._cond:
            self.count += 1
            self._cond.notify_all()

    def until(self, count: int, timeout: float = TIMEOUT) -> bool:
        """Whether ``count`` arrived within ``timeout`` seconds."""
        with self._cond:
            return self._cond.wait_for(lambda: self.count >= count, timeout)
