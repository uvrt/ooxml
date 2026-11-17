"""How much memory this process holds now (round 58): what ``describe`` reports with a
document's own share, so a run can see its own growth without tools of its own."""

from __future__ import annotations

import os
import sys


def process_memory() -> dict[str, float]:
    """This process's memory in MB: ``rss`` (resident now) and, on Linux, ``pss`` (its share
    of pages it shares with others) -- read from ``/proc``; where that is not there (macOS,
    Windows), ``peak_rss``, the most it has held (``getrusage``; nothing on Windows, which
    has no ``resource``)."""
    out: dict[str, float] = {}
    rollup = "/proc/self/smaps_rollup"
    if os.path.exists(rollup):
        try:
            with open(rollup) as handle:
                for line in handle:
                    name, _, rest = line.partition(":")
                    if name in ("Rss", "Pss"):
                        out[name.lower()] = round(int(rest.split()[0]) / 1024, 1)
        except (OSError, ValueError, IndexError):
            out.clear()
    if not out:
        try:
            import resource
        except ImportError:          # Windows
            return out
        peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
        # Bytes on macOS, kilobytes elsewhere.
        out["peak_rss"] = round(peak / (2 ** 20 if sys.platform == "darwin" else 1024), 1)
    return out


def release_free_memory() -> None:
    """Hand the C allocator's free pages back to the system (round 58, Q3): glibc's
    ``malloc_trim(0)`` (every arena), macOS's ``malloc_zone_pressure_relief`` (every zone);
    elsewhere nothing.  Python frees a document's objects into the allocator, which keeps the
    pages for later use until it is asked -- on glibc, in each thread's arena."""
    import ctypes
    import ctypes.util

    try:
        if sys.platform == "darwin":
            libc = ctypes.CDLL(ctypes.util.find_library("c"))
            libc.malloc_zone_pressure_relief.restype = ctypes.c_size_t
            libc.malloc_zone_pressure_relief(None, ctypes.c_size_t(0))
        elif sys.platform.startswith("linux"):
            libc = ctypes.CDLL("libc.so.6")
            libc.malloc_trim(ctypes.c_size_t(0))
    except (OSError, AttributeError):
        pass                                # another C library (musl): nothing to ask


def reclaim() -> None:
    """What a closed document leaves, given back now (round 58, Q3): the cycles its objects
    make collected (``gc.collect``), then the allocator's free pages released
    (:func:`release_free_memory`) -- what :meth:`.Session.close` runs once the document is
    let go, so the process's resident memory falls back then, not at the next collection."""
    import gc

    gc.collect()
    release_free_memory()
