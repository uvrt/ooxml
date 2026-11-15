"""Gates: a step held until the test opens it, in place of a sleep long enough to see a wait.

A test that claims "X does not wait for Y" holds Y at a gate, waits until Y is in it, runs X,
notes that X is done, then opens the gate: the order the notes come in is the claim.  Had X
waited for Y, Y's hold times out (:data:`TIMEOUT`) and notes so, and the test fails on the
order -- it never passes slowly, and a busy machine cannot make it fail.

A gate lives in a directory, so a worker process can hold it as a thread can: the notes are
lines appended to a file there (under a lock: :meth:`Gate._locked`), the gate is open once a
file named ``open`` is there.  A :class:`Gate` pickles as its path."""

from __future__ import annotations

import os
import threading
import time
from pathlib import Path

#: How long a hold waits for the test to open its gate before it notes that it timed out.
TIMEOUT = 60.0

#: How old the gate's lock is when it is taken as left behind by a killed worker (seconds).
STALE = 10.0

#: How often a hold looks whether its gate is open (seconds).
POLL = 0.005


class Gate:
    def __init__(self, path: str | os.PathLike) -> None:
        self.path = str(path)

    def __reduce__(self):
        return (Gate, (self.path,))

    @property
    def _dir(self) -> Path:
        return Path(self.path)

    def _locked(self):
        """The gate's lock across threads and processes: a directory made, then removed.
        Windows emulates an append as a seek and a write, so appends from two processes
        can land on one another; the notes are written and read under this instead."""
        gate = self

        class _Lock:
            def __enter__(self):
                ends = time.monotonic() + TIMEOUT
                while True:
                    try:
                        os.mkdir(gate._dir / "lock")
                        return self
                    except FileExistsError:
                        # One left by a worker killed while it wrote (a test kills some) is
                        # taken over: nobody holds it this long otherwise.
                        try:
                            if time.time() - os.stat(gate._dir / "lock").st_mtime > STALE:
                                os.rmdir(gate._dir / "lock")
                                continue
                        except FileNotFoundError:
                            continue
                        assert time.monotonic() < ends, f"the gate's lock was never let go: {gate.path}"
                        time.sleep(POLL / 5)

            def __exit__(self, *exc):
                os.rmdir(gate._dir / "lock")
                return False

        return _Lock()

    def note(self, what: str) -> None:
        """Append ``what`` to the order (one line, under the gate's lock)."""
        with self._locked():
            with open(self._dir / "order", "a", encoding="utf-8") as out:
                out.write(what + "\n")

    def order(self) -> list[str]:
        with self._locked():
            try:
                return (self._dir / "order").read_text(encoding="utf-8").splitlines()
            except FileNotFoundError:
                return []

    def hold(self, name: str = "held", timeout: float = TIMEOUT) -> bool:
        """Note ``<name> in``, wait until the gate is open, note ``<name> out`` -- or, past
        ``timeout``, ``<name> timed out``.  Whether it was opened."""
        self.note(f"{name} in")
        ends = time.monotonic() + timeout
        while not self.is_open():
            if time.monotonic() > ends:
                self.note(f"{name} timed out")
                return False
            time.sleep(POLL)
        self.note(f"{name} out")
        return True

    def is_open(self) -> bool:
        return (self._dir / "open").exists()

    def open(self) -> None:
        (self._dir / "open").touch()

    def close(self) -> None:
        """Closed again, the order kept."""
        try:
            (self._dir / "open").unlink()
        except FileNotFoundError:
            pass

    def entered(self, name: str = "held") -> bool:
        return f"{name} in" in self.order()

    def wait_entered(self, name: str = "held", timeout: float = TIMEOUT) -> None:
        """Wait until ``name`` is held at the gate."""
        ends = time.monotonic() + timeout
        while not self.entered(name):
            assert time.monotonic() < ends, f"{name} never reached the gate: {self.order()}"
            time.sleep(POLL)

    def wait_noted(self, what: str, timeout: float = TIMEOUT) -> None:
        ends = time.monotonic() + timeout
        while what not in self.order():
            assert time.monotonic() < ends, f"{what!r} never noted: {self.order()}"
            time.sleep(POLL)


class Watched:
    """A :class:`threading.Condition` that notes, under its own name, each thread's first
    wait on it (``<thread name> waits``), and counts the waits: put in place of a lock-free
    object's condition, it shows who waits for what."""

    def __init__(self, condition, gate: Gate) -> None:
        self.condition, self.gate = condition, gate
        self.waits = 0
        self.waiting = threading.Event()
        self._seen: set[str] = set()

    def __enter__(self):
        return self.condition.__enter__()

    def __exit__(self, *exc):
        return self.condition.__exit__(*exc)

    def wait(self, timeout=None):
        name = threading.current_thread().name
        self.waits += 1                          # under the condition's lock
        if name not in self._seen:
            self._seen.add(name)
            self.gate.note(f"{name} waits")
        self.waiting.set()
        return self.condition.wait(timeout)

    def __getattr__(self, name):
        return getattr(self.condition, name)


def until(condition, what: str, timeout: float = TIMEOUT) -> None:
    """Wait (up to ``timeout``) until ``condition()`` holds."""
    ends = time.monotonic() + timeout
    while not condition():
        assert time.monotonic() < ends, what
        time.sleep(POLL)


def open_when(gate: Gate, condition, timeout: float = TIMEOUT) -> threading.Thread:
    """Open ``gate`` from a thread once ``condition()`` holds, or ``timeout`` on (the holds
    then note that they timed out first).  The thread, started."""

    def run() -> None:
        ends = time.monotonic() + timeout
        while not condition() and time.monotonic() < ends:
            time.sleep(POLL)
        gate.open()

    opener = threading.Thread(target=run, daemon=True)
    opener.start()
    return opener


def thread(fn):
    """Run ``fn`` in a thread: the thread (started) and a dict its result lands in."""
    out: dict = {}
    t = threading.Thread(target=lambda: out.setdefault("result", fn()), daemon=True)
    t.start()
    return t, out
