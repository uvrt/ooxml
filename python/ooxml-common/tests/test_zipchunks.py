"""The MS-FSSHTTPD chunking check (tests/support/zipchunks.py) against the probe side's test
vectors (fixtures/zipchunks-vectors.json: made-up files, from
agent-work/coauth-2026-11/vectors), and the local headers a package keeps for a writer to
copy (zipparts.copyable_header)."""

from __future__ import annotations

import base64
import copy
import io
import json
import struct
import sys
import zipfile
from pathlib import Path

import pytest

from ooxml_common.zipparts import read_parts

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / "support"))
from zipchunks import (MIB, changed_chunks, entry_names, layout_problems,  # noqa: E402
                       zip_chunks)

VECTORS = json.loads((HERE / "fixtures" / "zipchunks-vectors.json").read_text())
ANALYSED = ["v1-small-entries.zip", "v2-large-entries.zip"]


def _vector(name: str) -> bytes:
    return base64.b64decode(VECTORS[name]["file_base64"])


def _expected(name: str) -> list[tuple[int, int, str | None]]:
    return [(c["offset"], c["size"], None if c["signature"].startswith("random")
             else c["signature"]) for c in VECTORS[name]["chunks"]]


def _found(data: bytes) -> list[tuple[int, int, str | None]]:
    return [(c.start, c.size, None if c.signature is None else c.signature.hex())
            for c in zip_chunks(data)]


@pytest.mark.parametrize("name", ANALYSED)
def test_the_vectors_chunk_as_the_probe_side_says(name):
    data = _vector(name)
    assert len(data) == VECTORS[name]["size"]
    assert _found(data) == _expected(name)
    assert layout_problems(data) == []


def test_a_first_entry_with_a_data_descriptor_is_not_analysed():
    name = "v4-data-descriptor.zip"
    data = _vector(name)
    assert VECTORS[name]["zip_analysis"] is False
    chunks = zip_chunks(data)
    assert [(c.kind, c.start, c.end) for c in chunks] == [("simple", 0, len(data))]
    assert any("data descriptor" in problem for problem in layout_problems(data))


def test_an_entry_over_a_mebibyte_is_a_header_and_a_data_chunk():
    """v3's layout made again (its 2.5 MB file is not kept): a stored entry of 2,500,000
    bytes, then v2's small entry copied as it is -- the same offsets, sizes and kinds, the
    small entry's signature the vector's, the data chunk signed with its CRC-32 and sizes."""
    name = "v3-over-1mb.zip"
    small = _vector("v2-large-entries.zip")
    with zipfile.ZipFile(io.BytesIO(small)) as archive:
        kept = copy.copy(archive.getinfo("doc/small.xml"))
        raw = small[kept.header_offset:archive.start_dir]
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w") as archive:
        big = zipfile.ZipInfo("media/big.bin", date_time=(2026, 1, 1, 0, 0, 0))
        big.create_system, big.external_attr = 3, 0o600 << 16
        archive.writestr(big, bytes(i * 7 % 251 for i in range(2_500_000)))
        kept.header_offset = archive.fp.tell()
        archive.fp.write(raw)
        archive.start_dir = archive.fp.tell()
        archive.filelist.append(kept)
        archive.NameToInfo[kept.filename] = kept
    data = out.getvalue()
    assert len(data) == VECTORS[name]["size"]
    expected, found = _expected(name), _found(data)
    assert [(at, size) for at, size, _ in found] == [(at, size) for at, size, _ in expected]
    assert found[2] == expected[2]                       # the small entry, as in v2
    info = zipfile.ZipFile(io.BytesIO(data)).getinfo("media/big.bin")
    assert zip_chunks(data)[1].signature == struct.pack("<LQQ", info.CRC, 2_500_000, 2_500_000)
    assert zip_chunks(data)[1].size > MIB and layout_problems(data) == []


def _zip(entries: dict[str, bytes], *, seekable: bool = True) -> bytes:
    out = io.BytesIO()
    target = out if seekable else _Unseekable(out)
    with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, data in entries.items():
            archive.writestr(zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0)), data)
    return out.getvalue()


class _Unseekable(io.RawIOBase):
    """A stream zipfile cannot seek back into, so it writes data descriptors."""

    def __init__(self, out: io.BytesIO) -> None:
        self._out = out

    def writable(self) -> bool:
        return True

    def write(self, data) -> int:
        return self._out.write(data)

    def tell(self) -> int:
        raise OSError("unseekable")

    def flush(self) -> None:
        pass


def test_one_changed_entry_is_one_changed_chunk_and_the_final_one():
    before = _zip({"a.xml": b"<a/>", "b.xml": b"<b/>"})
    after = _zip({"a.xml": b"<a/>", "b.xml": b"<b>x</b>"})
    assert changed_chunks(before, before) == (0, 3)
    assert changed_chunks(before, after) == (2, 3)
    assert entry_names(after) == ["a.xml", "b.xml"]


def test_a_needless_zip64_record_is_a_layout_problem():
    data = bytearray(_zip({"a.xml": b"<a/>"}))
    # a ZIP64 extra field (tag 1, no values) in the local header, what follows moved on
    name_len, extra_len = struct.unpack_from("<HH", data, 26)
    assert extra_len == 0
    zip64 = struct.pack("<HH", 1, 0)
    patched = bytearray(bytes(data[:30 + name_len]) + zip64 + bytes(data[30 + name_len:]))
    struct.pack_into("<H", patched, 28, len(zip64))
    end = patched.rfind(b"PK\x05\x06")
    (directory,) = struct.unpack_from("<L", patched, end + 16)
    struct.pack_into("<L", patched, end + 16, directory + len(zip64))
    assert any("ZIP64" in problem for problem in layout_problems(bytes(patched)))


def test_a_package_keeps_a_copyable_header_only():
    plain = _zip({"a.xml": b"<a/>"})
    _, parts = read_parts(plain)
    header = parts.stored("a.xml").header
    assert header is not None and bytes(header) == plain[:len(header)]
    _, parts = read_parts(_zip({"a.xml": b"<a/>"}, seekable=False))
    assert parts.stored("a.xml").header is None       # sizes in a data descriptor
