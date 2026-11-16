"""Gates: a step held until the test opens it, in place of a sleep long enough to see a wait.

A test that claims "X does not wait for Y" holds Y at a gate, waits until Y is in it, runs X,
notes that X is done, then opens the gate: the order the notes come in is the claim.  Had X
waited for Y, Y's hold times out (:data:`TIMEOUT`) and notes so, and the test fails on the
order -- it never passes slowly, and a busy machine cannot make it fail.

A gate lives in a directory, so a worker process can hold it as a thread can: each note is a
file made there in turn (:meth:`Gate.note`), the gate is open once a file named ``open`` is
there.  A :class:`Gate` pickles as its path."""

from __future__ import annotations

import os
import threading
import time
from pathlib import Path

#: How long a hold waits for the test to open its gate before it notes that it timed out.
TIMEOUT = 60.0

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

    def note(self, what: str) -> None:
        """``what`` happened now: the next note in turn, made as a file of its own whose name
        is its number and its text.  Making a file that must not exist yet is atomic on every
        platform, between processes too (an append is not on Windows, which emulates it as a
        seek and a write: two processes' notes could land on one another)."""
        notes = self._dir / "notes"
        notes.mkdir(exist_ok=True)
        number = len(os.listdir(notes)) + 1
        while True:
            try:
                os.close(os.open(notes / f"{number:06d}-{what.encode().hex()}",
                                 os.O_CREAT | os.O_EXCL | os.O_WRONLY))
                return
            except FileExistsError:
                number += 1

    def order(self) -> list[str]:
        try:
            names = sorted(os.listdir(self._dir / "notes"))
        except FileNotFoundError:
            return []
        return [bytes.fromhex(name.split("-", 1)[1]).decode() for name in names]

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
