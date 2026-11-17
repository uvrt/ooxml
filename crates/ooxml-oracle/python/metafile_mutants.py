#!/usr/bin/env python3
"""The metafile players' differential on hostile input (test support; phase P3, lane P3b1).

Reads the cases ``metafile_record.py`` wrote, makes ``--per`` mutants of each metafile
(bytes flipped, records cut short, duplicated or dropped, the file truncated -- seeded, so
a run is repeatable), draws each with ooxml-common exactly as ``metafile_to_svg`` does (an
exception is ``None``), and writes them as cases of the same format into ``OUT``; then
``metafile-diff OUT`` plays them in Rust.

    python crates/ooxml-oracle/python/metafile_mutants.py CASES OUT [--per 20] [--seed 1]

Needs ooxml-common importable (this checkout's ``python/ooxml-common/src``).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from metafile_record import _fields, meta_json  # noqa: E402


def read_fields(blob: bytes) -> list[bytes]:
    out, at = [], 0
    while at < len(blob):
        n = struct.unpack_from("<I", blob, at)[0]
        out.append(blob[at + 4 : at + 4 + n])
        at += 4 + n
    return out


def records(data: bytes) -> list[tuple[int, int]]:
    """``(offset, size)`` of each EMF record (WMF files are mutated as bytes only)."""
    out, at = [], 0
    while at + 8 <= len(data) and len(out) < 10_000:
        size = struct.unpack_from("<I", data, at + 4)[0]
        if size < 8 or at + size > len(data):
            break
        out.append((at, size))
        at += size
    return out


def mutate(data: bytes, rng: random.Random) -> bytes:
    b = bytearray(data)
    kind = rng.randrange(6)
    recs = records(data) if data[:4] == b"\x01\x00\x00\x00" else []
    if kind == 0 or not recs:
        for _ in range(rng.randint(1, 8)):
            if b:
                i = rng.randrange(len(b))
                b[i] = rng.randrange(256)
        return bytes(b)
    at, size = recs[rng.randrange(len(recs))]
    if kind == 1:  # a field of one record set to an extreme
        if size > 8:
            i = at + 8 + 4 * rng.randrange(max(1, (size - 8) // 4))
            if i + 4 <= len(b):
                b[i : i + 4] = struct.pack("<I", rng.choice([0, 1, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF, rng.randrange(1 << 32)]))
        return bytes(b)
    if kind == 2:  # one record dropped
        return bytes(b[:at] + b[at + size :])
    if kind == 3:  # one record duplicated
        return bytes(b[: at + size] + b[at : at + size] + b[at + size :])
    if kind == 4:  # the file truncated
        return bytes(b[: rng.randrange(max(1, len(b)))])
    i = at + rng.randrange(size)  # a byte inside one record
    b[i] ^= 1 << rng.randrange(8)
    return bytes(b)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("cases")
    parser.add_argument("out")
    parser.add_argument("--per", type=int, default=20)
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()
    from ooxml_common.metafile import _draw

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    written = 0
    for path in sorted(Path(args.cases).glob("*.case")):
        fields = read_fields(path.read_bytes())
        if fields[0] != b"metafile":
            continue
        rng = random.Random(f"{args.seed}:{path.name}")
        for _ in range(args.per):
            data = mutate(fields[1], rng)
            try:
                drawing = _draw(data, None)
            except Exception:
                drawing = None
            if drawing is None:
                blob = _fields(b"metafile", data, b"null", b"null", b"")
            else:
                blob = _fields(b"metafile", data, b"null", meta_json(drawing), drawing.svg)
            (out / (hashlib.sha256(blob).hexdigest()[:24] + ".case")).write_bytes(blob)
            written += 1
    print(json.dumps({"mutants": written}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
