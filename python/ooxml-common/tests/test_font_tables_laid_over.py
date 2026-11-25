"""Untrusted fonts (a document's embedded faces, a font folder) whose ``cmap`` or ``kern``
ranges are laid over each other again and again: refused in a moment, not read for
hours.  The Rust reader's fuzz timeout (``glyph_ink``), the same tables made up here."""

from __future__ import annotations

import struct
import time
from pathlib import Path

from ooxml_common.fonts import office, rasterdb, sfnt

FONT = Path(__file__).parent / "fixtures" / "fonts" / "cmap-groups-laid-over.ttf"


def _cmap(fmt_body: bytes, key: tuple[int, int] = (3, 1)) -> bytes:
    return struct.pack(">HHHHI", 0, 1, *key, 12) + fmt_body


def _format12(groups: list[tuple[int, int, int]]) -> bytes:
    body = struct.pack(">HHIII", 12, 0, 16 + 12 * len(groups), 0, len(groups))
    return body + b"".join(struct.pack(">III", *g) for g in groups)


def _format4(segments: list[tuple[int, int]]) -> bytes:
    n = len(segments)
    body = struct.pack(">HHHHHHH", 4, 0, 0, 2 * n, 0, 0, 0)
    body += struct.pack(f">{n}H", *(end for _, end in segments)) + b"\0\0"
    body += struct.pack(f">{n}H", *(start for start, _ in segments))
    return body + struct.pack(f">{n}h", *([1] * n)) + struct.pack(f">{n}H", *([0] * n))


def _quick(read, *args):
    started = time.perf_counter()
    out = read(*args)
    assert time.perf_counter() - started < 1.0
    return out


def test_the_made_up_font_maps_nothing_and_is_read_in_a_moment():
    data = FONT.read_bytes()
    face = office.Face(data)
    assert _quick(lambda: face.glyph("A")) is None
    tables = sfnt.read_tables(data)
    assert _quick(sfnt._cmap, tables[b"cmap"]) == {}
    assert _quick(rasterdb._coverage, tables[b"cmap"]) == (frozenset(), frozenset())


def test_groups_claiming_more_than_the_codespace_in_all_are_refused():
    # Half the codespace, twice: under sfnt's count of distinct codes, each group read again.
    table = _cmap(_format12([(0, 0x7FFFF, 1)] * 200))
    assert _quick(office.read_cmap, table) == {}
    assert _quick(sfnt._cmap, table) == {}
    # A group wider than the codespace is skipped, the rest read.
    table = _cmap(_format12([(0, 0x110000, 9), (0x41, 0x42, 5)]), (3, 10))
    assert office.read_cmap(table) == {0x41: 5, 0x42: 6}


def test_segments_claiming_more_than_sixteen_bits_in_all_are_refused():
    table = _cmap(_format4([(0, 0xFFFE)] * 2000))
    assert _quick(office.read_cmap, table) == {}
    assert _quick(sfnt._cmap, table) == {}
    table = _cmap(_format4([(0x41, 0x42), (0xFFFF, 0xFFFF)]))
    assert office.read_cmap(table) == {0x41: 0x42, 0x42: 0x43}
    assert sfnt._cmap(table) == {0x41: 0x42, 0x42: 0x43}


def test_kern_subtables_laid_over_each_other_are_read_no_further_than_the_table_holds():
    # 65,535 subtables of length 0, each the same one of 10,000 pairs.
    pairs = b"".join(struct.pack(">HHh", k, k, 1) for k in range(10_000))
    table = struct.pack(">HH", 0, 0xFFFF) + struct.pack(">HHHHHHH", 0, 0, 1, 10_000, 0, 0, 0) + pairs
    assert len(_quick(office.read_legacy_kern, table)) == 10_000
    # Apple's header: 2^32 - 1 subtables, the first of length 0 and not format 0.
    table = struct.pack(">II", 0x00010000, 0xFFFFFFFF) + struct.pack(">IHH", 0, 2, 0)
    assert _quick(office.read_legacy_kern, table) == {}
