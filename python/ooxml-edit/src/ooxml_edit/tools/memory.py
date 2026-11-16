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
