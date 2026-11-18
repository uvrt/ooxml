"""Lane L4b: a document collected while its finalizer's work would take a lock the
collecting thread already holds -- the 3.15t hang (a document's finalizer, fired by the
collector inside :func:`docx_agent.pages.lay_out`, waited for the engines' lock that very
thread held).  A finalizer only hands its work over (:mod:`docx_agent.finalizers`); the work
is done at the next safe point."""

from __future__ import annotations

import gc
import threading

import pytest

from docx_agent import Document, finalizers, pages
from docx_agent import layout as layout_module

DEADLINE = 30.0


class _Engine:
    """Stands in for a ``docx2svg.rust.Engine``; ``fits`` runs ``inside`` first -- where the
    collector ran in the hang (``fits`` -> ``_engine_key`` -> ``_fonts_key``)."""

    def __init__(self, inside=None):
        self.inside = inside

    def fits(self, convert_options):
        if self.inside is not None:
            self.inside()
        return True

    def lay_out(self, data, convert_options):
        return None


def _collectable() -> tuple[list, str]:
    """A document with its layout state (and finalizer) made, its only reference in a list
    the caller empties, and its lineage."""
    document = Document.new()
    layout_module._keeper(document)
    return [document], document._layout_lineage


def _run(target) -> threading.Thread:
    thread = threading.Thread(target=target, daemon=True)
    thread.start()
    thread.join(DEADLINE)
    return thread


@pytest.fixture
def engines(monkeypatch):
    from docx2svg import rust

    monkeypatch.setattr(pages, "available", lambda: True)
    monkeypatch.setattr(rust, "unsupported_options", lambda options: None)
    finalizers.drain()
    with pages._LOCK:
        saved = dict(pages._ENGINES)
        pages._ENGINES.clear()
    yield pages._ENGINES
    finalizers.drain()
    with pages._LOCK:
        pages._ENGINES.clear()
        pages._ENGINES.update(saved)


def test_a_document_collected_inside_lay_out_does_not_deadlock(engines):
    held, collected = _collectable()

    def collect_inside():
        held.clear()            # the last reference: the collector runs the finalizer here
        gc.collect()

    engines[("mine", "key")] = _Engine(collect_inside)
    engines[(collected, "key")] = _Engine()

    thread = _run(lambda: pages.lay_out(b"", {}, "mine"))
    assert not thread.is_alive(), "lay_out deadlocked on a finalizer"
    # The collected document's engines go at the next safe point, not inside the finalizer.
    assert (collected, "key") in engines
    assert finalizers.pending() >= 1
    pages.forget("someone else")
    assert (collected, "key") not in engines
    assert ("mine", "key") in engines


def test_a_finalizer_fired_while_the_engines_lock_is_held_does_not_deadlock(engines):
    # The same, in the strictest form: the collector runs while the lock itself is held.
    held, collected = _collectable()
    engines[(collected, "key")] = _Engine()

    def collect_under_the_lock():
        with pages._LOCK:
            held.clear()
            gc.collect()

    thread = _run(collect_under_the_lock)
    assert not thread.is_alive(), "a finalizer took the engines' lock"
    assert finalizers.drain() >= 1
    assert (collected, "key") not in engines


def test_drained_work_failing_is_logged_not_raised():
    def fails():
        raise RuntimeError("gone")

    finalizers.later(fails)
    assert finalizers.drain() == 1
    assert finalizers.pending() == 0


@pytest.mark.skipif(not pages.available(), reason="needs ooxml_core's Rust layout")
def test_the_real_engine_key_collecting_a_document_does_not_deadlock(monkeypatch):
    # The hang's own path: a real engine, the collector run from inside ``_engine_key``.
    from docx2svg import rust

    lineage = "l4b-finalizer"
    assert pages.lay_out(Document.new().to_bytes(), {}, lineage) is not None
    held, collected = _collectable()
    real = rust._engine_key

    def collecting(options):
        if held:
            held.clear()
            gc.collect()
        return real(options)

    monkeypatch.setattr(rust, "_engine_key", collecting)
    try:
        thread = _run(lambda: pages.lay_out(Document.new().to_bytes(), {}, lineage))
        assert not thread.is_alive(), "lay_out deadlocked on a finalizer"
        assert not held
    finally:
        pages.forget(lineage)
