"""What a :class:`pptx2svg.ModelCache` holds: each picture once, and no more than its budget.

A production worker editing a deck on a picture-heavy template grew without bound: every
version of every slide kept its own base64 copy of every picture it showed, its layout's
and master's included, up to 512 slides.  Now a picture is one string, by its bytes, across
slides, versions and decks, and the slides and their pictures are kept within a byte
budget.  The deck is made up: the sample fixture with made-up noise pictures added to its
master and to every slide.
"""

from __future__ import annotations

import random
import struct
import sys
import threading
import zlib

import pytest

from pptx2svg import CacheStats, ConvertOptions, ModelCache, convert_pptx_to_model, open_parts
from test_model_cache import drawn, parts_of

REL_IMAGE = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"


def noise_png(seed: int, side: int = 96) -> bytes:
    """A made-up RGB picture of noise, ``side`` pixels square, stored uncompressed."""
    rows = random.Random(seed).randbytes(side * side * 3)
    raw = b"".join(b"\x00" + rows[y * side * 3:(y + 1) * side * 3] for y in range(side))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF))

    header = struct.pack(">IIBBBBB", side, side, 8, 2, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw, 0))
            + chunk(b"IEND", b""))


def picture(shape_id: int, rel_id: str, x: int) -> str:
    return (f'<p:pic><p:nvPicPr><p:cNvPr id="{shape_id}" name="Picture {shape_id}"/>'
            '<p:cNvPicPr/><p:nvPr/></p:nvPicPr>'
            f'<p:blipFill><a:blip r:embed="{rel_id}"/><a:stretch><a:fillRect/></a:stretch>'
            f'</p:blipFill><p:spPr><a:xfrm><a:off x="{x}" y="457200"/><a:ext cx="914400" '
            'cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>'
            '</p:pic>')


def with_picture(parts: dict[str, bytes], part: str, media: str, data: bytes,
                 shape_id: int, x: int) -> None:
    """Add a picture of ``data`` (the part ``media``) to ``part``'s shape tree."""
    folder, name = part.rsplit("/", 1)
    rels = f"{folder}/_rels/{name}.rels"
    rel_id = f"rIdPic{shape_id}"
    parts[media] = data
    parts[rels] = parts[rels].replace(
        b"</Relationships>",
        f'<Relationship Id="{rel_id}" Type="{REL_IMAGE}" '
        f'Target="../media/{media.rsplit("/", 1)[1]}"/></Relationships>'.encode())
    parts[part] = parts[part].replace(
        b"</p:spTree>", picture(shape_id, rel_id, x).encode() + b"</p:spTree>")


def picture_deck(version: int = 0, slide_seed=lambda number: number,
                 side: int = 96) -> dict[str, bytes]:
    """The sample deck with one noise picture, ``side`` pixels square, on its master and one
    on each slide (slide ``n``'s is ``slide_seed(n)``'s noise, each in a part of its own);
    ``version`` changes slide 1's title, as an edit would."""
    parts = parts_of("sample.pptx")
    types = "[Content_Types].xml"
    parts[types] = parts[types].replace(
        b'<Default Extension="xml"',
        b'<Default Extension="png" ContentType="image/png"/><Default Extension="xml"')
    with_picture(parts, "ppt/slideMasters/slideMaster1.xml", "ppt/media/master.png",
                 noise_png(1000, side), 9100, 8 * 914400)
    for number in range(1, 7):
        with_picture(parts, f"ppt/slides/slide{number}.xml", f"ppt/media/slide{number}.png",
                     noise_png(slide_seed(number), side), 9200 + number, 457200)
    if version:
        first = "ppt/slides/slide1.xml"
        parts[first] = parts[first].replace(b"</p:spTree>", (
            '<p:sp><p:nvSpPr><p:cNvPr id="9300" name="Version"/><p:cNvSpPr txBox="1"/><p:nvPr/>'
            '</p:nvSpPr><p:spPr><a:xfrm><a:off x="457200" y="4572000"/><a:ext cx="3657600" '
            'cy="457200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>'
            '<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US"/>'
            f'<a:t>Version {version}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree>').encode())
    return parts


def texts(model) -> list[str]:
    """Every picture's base64 text in the model, slide by slide, in order."""
    found: list[str] = []
    for slide in model.slides:
        stack = list(slide.elements)
        while stack:
            element = stack.pop()
            if getattr(element, "image_data", None) is not None:
                found.append(element.image_data)
            stack.extend(getattr(element, "children", None) or ())
    return found


def recount(cache: ModelCache) -> tuple[int, int]:
    """``(entry_bytes, image_bytes)`` worked out again from what the cache holds."""
    with cache._lock:
        entries = sum(kept.weight for kept in cache._slides.values())
        digests = {digest for kept in cache._slides.values() for digest in kept.images}
        # No conversion is under way: the store holds the kept slides' pictures alone.
        assert digests == set(cache._images)
        held = sum(sys.getsizeof(cache._images[digest].text) for digest in digests)
        for digest, picture in cache._images.items():
            assert picture.using == 0
            assert picture.kept == sum(digest in kept.images for kept in cache._slides.values())
    return entries, held


def test_a_picture_shown_on_every_slide_is_one_string():
    # The master's picture under six slides, and three slides' pictures of equal bytes in
    # parts of their own: one string each, in one conversion, with or without a cache.
    parts = picture_deck(slide_seed=lambda number: number % 3)
    for options in ({}, {"cache": ModelCache()}):
        model = convert_pptx_to_model(open_parts(parts), ConvertOptions(**options))
        found = texts(model)
        assert len(found) == 12
        assert len({id(text) for text in found}) == 4      # the master's and three others
        assert len(set(found)) == 4


def test_versions_of_a_deck_hold_each_picture_once():
    # Pictures 256 pixels square (about 260 KB of text each), not the other tests' small
    # ones: next to those, what a slide's own objects weigh is the interpreter's object
    # layout (twice as much on 3.10 as on 3.12) rather than the cache's.
    cache = ModelCache()
    plain = drawn(open_parts(picture_deck(side=256)))
    first = None
    for version in range(1, 21):
        parts = picture_deck(version, side=256)
        convert_pptx_to_model(open_parts(parts), ConvertOptions(cache=cache))
        stats = cache.stats()
        if first is None:
            first = stats
        # Slide 1 is new each version, the others are hits; the pictures stay seven.
        assert stats.entries == 6 + version - 1
        assert stats.images == 7 and stats.image_bytes == first.image_bytes
        assert (stats.entry_bytes, stats.image_bytes) == recount(cache)
    assert drawn(open_parts(picture_deck(side=256)), cache=cache) == plain
    # One picture's text is most of what a slide holds; the slides' own objects are small:
    # the 25 slides kept weigh less than half the seven pictures.
    assert 2 * stats.entry_bytes < stats.image_bytes


def test_the_cache_never_holds_more_than_its_budget():
    # Every version a new picture on every slide: without a bound by bytes the pictures
    # pile up however few slides are kept.
    one = sys.getsizeof(texts(convert_pptx_to_model(open_parts(picture_deck())))[0])
    budget = 10 * one
    cache = ModelCache(budget=budget)
    for version in range(8):
        parts = picture_deck(version, slide_seed=lambda number: 100 * version + number)
        assert drawn(open_parts(parts), cache=cache) == drawn(open_parts(parts))
        stats = cache.stats()
        assert stats.bytes <= budget
        assert 0 < stats.entries <= 6 * (version + 1)
        assert (stats.entry_bytes, stats.image_bytes) == recount(cache)
    assert stats.entries < 10 and stats.misses == 48


def test_a_slide_over_the_budget_is_drawn_but_not_kept():
    parts = picture_deck()
    cache = ModelCache(budget=1000)
    assert drawn(open_parts(parts), cache=cache) == drawn(open_parts(parts))
    stats = cache.stats()
    assert (stats.entries, stats.images, stats.bytes, stats.misses) == (0, 0, 0, 6)


def test_stats_budget_and_clear():
    cache = ModelCache()
    assert cache.budget == 256 * 2 ** 20
    assert cache.stats() == CacheStats(entries=0, entry_bytes=0, images=0, image_bytes=0,
                                       parts=0, hits=0, misses=0, budget=256 * 2 ** 20,
                                       limit=512)
    parts = picture_deck()
    convert_pptx_to_model(open_parts(parts), ConvertOptions(cache=cache))
    convert_pptx_to_model(open_parts(parts), ConvertOptions(cache=cache, slide_numbers=[2]))
    stats = cache.stats()
    assert (stats.entries, stats.images, stats.hits, stats.misses) == (6, 7, 1, 6)
    assert stats.parts > 0 and stats.bytes == stats.entry_bytes + stats.image_bytes
    # A lower budget lets go of the least recently used at once; slide 2, just used, stays.
    cache.budget = stats.bytes // 2
    lower = cache.stats()
    assert lower.bytes <= stats.bytes // 2 and 0 < lower.entries < 6
    assert (lower.entry_bytes, lower.image_bytes) == recount(cache)
    convert_pptx_to_model(open_parts(parts), ConvertOptions(cache=cache, slide_numbers=[2]))
    assert cache.stats().hits == 2
    cache.budget = 0
    assert cache.stats().entries == 0 and cache.stats().bytes == 0
    cache.budget = 2 ** 30
    convert_pptx_to_model(open_parts(parts), ConvertOptions(cache=cache))
    cache.clear()
    cleared = cache.stats()
    assert (cleared.entries, cleared.images, cleared.parts, cleared.bytes) == (0, 0, 0, 0)
    assert (cleared.hits, cleared.misses) == (2, 12)


@pytest.mark.parametrize("budget", [256 * 2 ** 20, 40_000])
def test_threads_drawing_versions_share_one_picture_store(budget):
    # Several threads drawing versions of the deck at once, through one cache: the same
    # SVGs as drawn alone, the bounds held, the pictures each held once.
    versions = [picture_deck(version) for version in range(4)]
    plain = [drawn(open_parts(parts))[0] for parts in versions]
    cache = ModelCache(budget=budget)
    errors: list[BaseException] = []
    found: dict[tuple[int, int], list[str]] = {}

    def draw(worker: int) -> None:
        try:
            for turn in range(6):
                version = (worker + turn) % len(versions)
                found[worker, turn] = drawn(open_parts(versions[version]), cache=cache)[0]
                assert found[worker, turn] == plain[version]
        except BaseException as error:  # noqa: BLE001
            errors.append(error)

    threads = [threading.Thread(target=draw, args=(worker,)) for worker in range(6)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=600)
    assert not errors and len(found) == 36
    stats = cache.stats()
    assert stats.bytes <= budget and stats.images <= 7
    assert (stats.entry_bytes, stats.image_bytes) == recount(cache)
    model = convert_pptx_to_model(open_parts(versions[0]), ConvertOptions(cache=cache))
    assert len({id(text) for text in texts(model)}) == 7
