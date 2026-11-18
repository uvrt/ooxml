"""What a garbage-collector finalizer hands over, done later at a safe point.

A finalizer (a ``weakref.finalize`` callback, a ``__del__``) runs wherever the collector
happens to run: on any thread, in the middle of any code -- also while that very thread
holds a lock the finalizer's work would take, which then never comes free (the 3.15t
deadlock: a document collected inside :func:`docx_agent.pages.lay_out`, whose finalizer
waited on the engines' lock the same thread held).  So docx-agent's finalizers do no work
and take no lock: they hand the work over (:func:`later`, a ``queue.SimpleQueue``, whose
``put`` is safe from a finalizer by design), and the work is done at a safe point
(:func:`drain`) -- the start of a call that lays a document out or lets one go, before it
takes any lock of its own.  The work handed over takes only the module-level leaf locks of
the layouts and engines, never a document's.
"""

from __future__ import annotations

import logging
import queue
from typing import Any, Callable

logger = logging.getLogger(__name__)

_PENDING: "queue.SimpleQueue[tuple[Callable[..., Any], tuple]]" = queue.SimpleQueue()


def later(work: Callable[..., Any], *args: Any) -> None:
    """``work(*args)`` handed over, to be done by the next :func:`drain`: what a finalizer
    runs (``weakref.finalize(obj, later, work, *args)``).  Takes no lock, does no work."""
    _PENDING.put((work, args))


def drain() -> int:
    """Do the work finalizers handed over, on this thread, now: how many were done.  Called
    with none of docx-agent's layout or engine locks held.  A finalizer that fires during
    it only hands more over, done in the same drain."""
    done = 0
    while True:
        try:
            work, args = _PENDING.get_nowait()
        except queue.Empty:
            return done
        try:
            work(*args)
        except Exception as error:  # noqa: BLE001 -- a collected document's clean-up: logged, never raised into the caller
            logger.debug("a finalizer's work failed: %s", error)
        done += 1


def pending() -> int:
    """How many finalizers' work is waiting for the next :func:`drain` (approximate)."""
    return _PENDING.qsize()
