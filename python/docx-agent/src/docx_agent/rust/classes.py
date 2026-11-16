"""Classes made once, on first use (lane L, 0.1.125).

A class here derives from one defined where this package cannot import it at load time
(``docx_agent.edit.effective._Model``, ``docx_agent.scope.Scope``: they import back), so it
is made when first asked for -- once per process, under a lock.  These classes used to be
defined inside the functions that made their instances, a new class per call.  On a
free-threaded CPython 3.15.2 classes made by several threads at once can hang the
interpreter for good (one thread stopping the world for a type change while the others spin
for the type lock without letting it stop them; the stdlib alone reproduces it, round 47), and
every call making classes made it likely with many writers; a class per call also left every
attribute lookup on it outside the type cache."""

from __future__ import annotations

import threading
from typing import Any, Callable

_LOCK = threading.Lock()
_MADE: dict[str, type] = {}


def once(make: Callable[[], type]) -> Callable[[], type]:
    """``make`` (no arguments, returning a class) called once, its class kept: the returned
    function gives that class, made under a lock the first time."""
    key = f"{make.__module__}.{make.__qualname__}"

    def get() -> type:
        found = _MADE.get(key)
        if found is not None:
            return found
        with _LOCK:
            found = _MADE.get(key)
            if found is None:
                found = _MADE[key] = make()
        return found

    get.__doc__ = make.__doc__
    get.__wrapped__ = make  # type: ignore[attr-defined]
    return get


def made() -> dict[str, Any]:
    """The classes made so far, by their factory's name (tests)."""
    return dict(_MADE)
