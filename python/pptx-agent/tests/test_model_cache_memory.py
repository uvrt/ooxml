"""The process's model cache over many edits of a picture-heavy deck, and its controls.

A production worker (pptx-agent 0.1.25, pptx2svg 0.3.9) grew to 7.4 GB and was killed:
every version of every slide the edits and renders resolved kept its own base64 copy of the
slide's pictures, the layout's and master's included, up to 512 slides.  Here, in small: a
made-up deck of slides with a noise picture each, a title set and the slide rendered after
every edit, in-process -- the pictures are held once, however many versions there are.
"""

from __future__ import annotations

import random
import struct
import zlib

import pytest
from ooxml_edit.tools import Limits, Toolbox

from pptx_agent import Document
from pptx_agent.edit import resolved
from pptx_agent.tools import GROUPS, TOOLS, deck_format

pptx2svg = pytest.importorskip("pptx2svg")
pytest.importorskip("resvg_py")

EMU = 914400


def noise_png(seed: int, side: int = 160) -> bytes:
    """A made-up RGB picture of noise, stored uncompressed (77 kB at 160 pixels)."""
    rows = random.Random(seed).randbytes(side * side * 3)
    raw = b"".join(b"\x00" + rows[y * side * 3:(y + 1) * side * 3] for y in range(side))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF))

    header = struct.pack(">IIBBBBB", side, side, 8, 2, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw, 0))
            + chunk(b"IEND", b""))


@pytest.fixture
def cache():
    """The process's cache, empty, at its default budget again afterwards."""
    found = resolved.model_cache()
    budget = found.budget
    found.clear()
    yield found
    found.budget = budget
    found.clear()


def test_edits_and_renders_hold_each_picture_once(cache):
    deck = Document.new(size="16:9", title="Made up")
    titles = []
    for number in range(4):
        slide = deck.add_slide("Title Only")
        slide.shapes[0].set_text(f"Slide {number + 1}")
        slide.add_picture(noise_png(number), left=EMU, top=2 * EMU, width=4 * EMU)
        titles.append(slide.shapes[0].id)
    toolbox = Toolbox(TOOLS, formats=[deck_format()], groups=GROUPS, workers=0,
                      limits=Limits(image_budget=None))
    session = toolbox.session()
    doc = session.open(deck.to_bytes(), "made-up.pptx")
    held = []
    for edit in range(16):
        number = edit % 4 + 1
        changed = toolbox.dispatch(session, "ppt_set_text", {"doc": doc, "items": [
            {"target": titles[number - 1], "text": f"Title {edit}"}]})
        assert changed.ok, changed.summary
        rendered = toolbox.dispatch(session, "render",
                                    {"doc": doc, "slides": [number], "width": 320})
        assert rendered.ok, rendered.summary
        held.append(resolved.model_cache_stats())
    # Every version of every slide is kept (well within the budget), and each slide's
    # picture once: the bytes held for pictures stop growing once each slide was drawn.
    assert held[-1]["entries"] >= 16
    assert held[3]["images"] == held[-1]["images"] == 4
    assert held[3]["image_bytes"] == held[-1]["image_bytes"]
    assert held[-1]["image_bytes"] < 4 * 2 * len(noise_png(0))
    assert held[-1]["bytes"] == held[-1]["entry_bytes"] + held[-1]["image_bytes"]
    assert held[-1]["bytes"] <= held[-1]["budget"]
    assert held[-1]["hits"] > 0 and held[-1]["misses"] > 0


def test_a_host_logs_and_sets_the_cache(cache):
    from ooxml_edit.tools.worker import task_reference

    stats = resolved.model_cache_stats()
    assert set(stats) == {"entries", "entry_bytes", "images", "image_bytes", "parts", "hits",
                          "misses", "budget", "limit", "bytes"}
    assert stats["budget"] == 256 * 2 ** 20 and stats["limit"] == 512
    resolved.set_model_cache_budget(64 * 2 ** 20)
    assert resolved.model_cache_stats()["budget"] == 64 * 2 ** 20 == cache.budget
    deck = Document.new()
    deck.add_slide("Title Only").add_picture(noise_png(7), left=EMU, top=EMU, width=EMU)
    deck.slides[0].render_svg()
    assert resolved.model_cache_stats()["images"] == 1
    resolved.clear_model_cache()
    assert resolved.model_cache_stats()["bytes"] == 0
    # Each is a worker task, for a host to broadcast to its worker processes.
    for task in (resolved.model_cache_stats, resolved.set_model_cache_budget,
                 resolved.clear_model_cache):
        assert task_reference(task) == f"pptx_agent.edit.resolved:{task.__name__}"
