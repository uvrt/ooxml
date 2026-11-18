"""A task the layout workers run in tests: what a worker holds (round 58, Q2)."""

from __future__ import annotations


def held_lineages() -> dict:
    """In a worker: the lineages whose layouts it holds, each with the tokens a live view's
    versions are kept under there."""
    from docx_agent import layout

    with layout._HELD_LOCK:
        return {held.lineage: sorted(held.extra.get("views") or ()) for held in layout._HELD.values()}


held_lineages.__ooxml_edit_task__ = True  # type: ignore[attr-defined]


def rust_engines() -> int:
    """In a worker: how many Rust layout engines it holds (lane L4b: none)."""
    from docx_agent import pages

    with pages._LOCK:
        return len(pages._ENGINES)


rust_engines.__ooxml_edit_task__ = True  # type: ignore[attr-defined]
