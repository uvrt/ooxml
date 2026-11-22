"""The body's top-level blocks in its part's bytes, one reading per version shared by the
passes (lane K, 0.1.123).

Each pass that reads a version per change -- the touched sections (:mod:`.tools.touched`),
revalidate's body facts (:mod:`.revalidate`), the ids note (:class:`.edit.ids.IdScan`), the
body's index (:func:`.edit.ids.body_index`), an undo's restore (:mod:`.oxml.package`) -- finds the same blocks
from the same bytes: the bytes before the first that differs from the version before and
after the last, the blocks kept on either side, the blocks between found by their tags.
``ooxml_core.BodySpans`` holds a version's spans in Rust (no Python object per block) and
makes the step from one version to the next once, with the interpreter lock let go; the
passes take it from there.  A document keeps the latest few versions' readings, by their
bytes (the same bytes object: the passes read the version's bytes as the package holds
them), so passes that began from different readings meet on one.

The steps are the same as the Python functions they replace (``_scan``, ``_blocks``,
``_rescan``, ``_kept_counts``; ``DOCX_AGENT_FULL_VALIDATE=1`` compares each reading with
them); without the core, or with ``DOCX_AGENT_INCREMENTAL=0``, the passes read as before.
"""

from __future__ import annotations

import threading
from typing import Any

from . import incremental

#: Readings kept per document: the latest versions' (an edit's, its batch's ops', an undo's).
_KEPT = 4
_LOCK = threading.Lock()


def core() -> Any:
    """``ooxml_core`` where it has ``BodySpans`` and the passes are made per change."""
    if not incremental.enabled():
        return None
    try:
        import ooxml_core
    except ImportError:
        return None
    return ooxml_core if hasattr(ooxml_core, "BodySpans") else None


def _known(holder: Any) -> list:
    found = holder.__dict__.get("_body_spans_kept")
    if found is None:
        found = holder.__dict__["_body_spans_kept"] = []
    return found


def _remember(holder: Any, data: bytes, spans: Any) -> None:
    with _LOCK:
        kept = _known(holder)
        for k, (d, _) in enumerate(kept):
            if d is data:
                del kept[k]
                break
        kept.append((data, spans))
        del kept[:-_KEPT]


def known(holder: Any, data: bytes) -> Any:
    """The reading of ``data`` (this very bytes object) kept on ``holder``, or ``None``."""
    if holder is None:
        return None
    with _LOCK:
        for d, spans in reversed(_known(holder)):
            if d is data:
                return spans
    return None


def scan(holder: Any, data: bytes) -> Any:
    """``data``'s reading (``BodySpans``), kept on ``holder`` (the document); ``None`` where
    the blocks are not read from its bytes (or without the core)."""
    c = core()
    if c is None or not isinstance(data, bytes):
        return None
    found = known(holder, data)
    if found is not None:
        incremental.STATS["body spans: shared"] += 1
        return found
    found = c.BodySpans.scan(data)
    incremental.STATS["body spans: scanned"] += 1
    if found is not None and incremental.verifying():
        _verify(found, data)
    if found is not None and holder is not None:
        _remember(holder, data, found)
    return found


def step(holder: Any, old: Any, data: bytes) -> "tuple[Any, int, int, int, int] | None":
    """From ``old`` (a reading of another version of the body) to ``data``'s: ``(reading,
    before, after, begin, stop)`` -- the blocks kept before and after, the stretch of
    ``data`` read again -- or ``None`` where the change reaches outside the body or its tags
    there do not nest (``_rescan``'s rule)."""
    if old is None or not isinstance(data, bytes):
        return None
    canonical = known(holder, old.data)
    if canonical is not None:
        old = canonical
    found = old.advance(data)
    if found is None:
        return None
    incremental.STATS["body spans: stepped"] += 1
    if incremental.verifying():
        _verify_step(old, data, found)
    if holder is not None:
        _remember(holder, data, found[0])
    return found


def _verify(found: Any, data: bytes) -> None:
    from .revalidate import _scan

    whole = _scan(data)
    if whole is None or list(whole[0]) != found[:] or tuple(whole[1]) != found.body:
        raise incremental.Mismatch("body spans: the reading in Rust differs from the bytes' blocks")


def _verify_step(old: Any, data: bytes, found: Any) -> None:
    from .tools.touched import _blocks, _common_prefix, _common_suffix, _kept_counts

    spans = old[:]
    olds = old.data
    prefix = _common_prefix(olds, data)
    suffix = _common_suffix(olds, data, min(len(olds), len(data)) - prefix)
    before, after = _kept_counts(spans, prefix, len(olds) - suffix)
    delta = len(data) - len(olds)
    begin = spans[before - 1][1] if before else old.body[0]
    stop = spans[len(spans) - after][0] + delta if after else old.body[1] + delta
    middle = _blocks(data, begin, stop)
    shifted = [(s + delta, e + delta) for s, e in spans[len(spans) - after:]]
    expected = spans[:before] + (middle or []) + shifted
    new, b, a, g, t = found
    if (b, a, g, t) != (before, after, begin, stop) or middle is None or new[:] != expected:
        raise incremental.Mismatch("body spans: the step in Rust differs from the bytes' blocks")
    _verify(new, data)
