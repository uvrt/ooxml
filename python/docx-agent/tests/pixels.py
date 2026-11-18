"""A PNG's pixels, for comparing images drawn by two rasterisers: resvg-py's and the Rust
one's (``ooxml_core.PageRaster``) draw the same pixels, which their encoders compress
differently."""

from __future__ import annotations

import struct
import zlib


def pixels(png: bytes) -> tuple:
    """An 8-bit RGBA PNG's ``((width, height), pixels)`` (resvg's and tiny-skia's encoders
    write no other kind)."""
    at, chunks = 8, {}
    while at < len(png):
        size, = struct.unpack(">I", png[at:at + 4])
        kind = png[at + 4:at + 8]
        chunks[kind] = chunks.get(kind, b"") + png[at + 8:at + 8 + size]
        at += 12 + size
    width, height, depth, colour = struct.unpack(">IIBB", chunks[b"IHDR"][:10])
    assert (depth, colour) == (8, 6), (depth, colour)
    raw, stride, out, previous = zlib.decompress(chunks[b"IDAT"]), width * 4, bytearray(), bytearray(width * 4)
    for row in range(height):
        kind, line = raw[row * (stride + 1)], bytearray(raw[row * (stride + 1) + 1:(row + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - 4] if i >= 4 else 0
            b = previous[i]
            c = previous[i - 4] if i >= 4 else 0
            if kind == 1:
                line[i] = (line[i] + a) & 255
            elif kind == 2:
                line[i] = (line[i] + b) & 255
            elif kind == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif kind == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        out += line
        previous = line
    return (width, height), bytes(out)


def all_pixels(pngs) -> list:
    """Each PNG's :func:`pixels`."""
    return [pixels(png) for png in pngs]
