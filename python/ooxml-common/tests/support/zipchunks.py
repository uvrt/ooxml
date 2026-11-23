"""MS-FSSHTTPD 2.4.1 ZIP file chunking, as a check for the tests (no pytest here): the chunks a
co-authoring host keeps of a saved package, and whether the package is laid out so that
Office's analysis chunks it entry by entry.

Office stores a package on a co-authoring host as chunks, one per zip entry (or its header
and its data), each signed so that an entry whose bytes did not change keeps its signature
and only changed entries travel (the design page "CyberAI as an Office Client", requests 1
and 3).  :func:`zip_chunks` follows the probe side's written description of the analysis
(agent-work/coauth-2026-11/spec-description.txt, 23 November 2026, part A) and reproduces its
test vectors (tests/fixtures/zipchunks-vectors.json); it was written from that description and
the specification's text, not from the probe's code:

* walk the local headers from the start of the file; an entry's header is ``h = 30 + n + x``
  bytes, a size of 0xFFFFFFFF read from its ZIP64 extra field (no field: the walk stops);
  a first entry with a data descriptor and no compressed size means the file is not
  analysed at all (simple chunking: 1 MiB pieces, each signed with its SHA-1);
* H = SHA-1 of the header, D = CRC-32 (u32), compressed size (u64), size (u64);
  ``h + compressed <= 4096``: one chunk signed H XOR D, else a header chunk signed H and a
  data chunk signed D;
* everything after the last entry walked -- the central directory and end records -- is one
  final chunk, signed with its SHA-1 when at most 1 MiB, else with random bytes.

Random signatures (and the random subchunks of a chunk over 1 MiB, part B) never match, so
they are ``None`` here.  :func:`layout_problems` adds what the saved packages are held to
beyond the analysis: every entry walked, no data descriptors, no ZIP64 record where no size
needs one.

This is the check only.  The chunk writer that signs a save's changed parts for upload is
the shared package layer's (lane coauth-2), not this module.
"""

from __future__ import annotations

import hashlib
import io
import struct
import zipfile
from dataclasses import dataclass

#: The most bytes an entry's header and data take as one chunk.
SMALL = 4096
#: The largest final chunk signed with its SHA-1, and the simple chunks' size.
MIB = 1 << 20

_LOCAL = struct.Struct("<4s5H3L2H")
_ZIP64 = 0x0001
_FULL = 0xFFFFFFFF


@dataclass(frozen=True)
class Chunk:
    """One chunk: ``kind`` (``entry``, ``header``, ``data``, ``final`` or ``simple``), the
    entry's ``name`` (``""`` for the others), its byte range, and its ``signature``
    (``None`` where the writer signs it with random bytes)."""

    kind: str
    name: str
    start: int
    end: int
    signature: bytes | None

    @property
    def size(self) -> int:
        return self.end - self.start


def _zip64_sizes(extra: bytes, compressed: int, size: int) -> tuple[int, int] | None:
    """The sizes from the ZIP64 extra field: its u64s are the size, then the compressed
    size, each only where the header's is 0xFFFFFFFF; ``None`` without the field."""
    at = 0
    while at + 4 <= len(extra):
        tag, length = struct.unpack_from("<HH", extra, at)
        if tag == _ZIP64:
            values = list(struct.unpack_from(f"<{length // 8}Q", extra, at + 4))
            if size == _FULL:
                size = values.pop(0)
            if compressed == _FULL:
                compressed = values.pop(0)
            return compressed, size
        at += 4 + length
    return None


def _simple(data: bytes) -> list[Chunk]:
    return [Chunk("simple", "", at, min(at + MIB, len(data)),
                  hashlib.sha1(data[at:at + MIB]).digest())
            for at in range(0, len(data), MIB)]


def zip_chunks(data: bytes) -> list[Chunk]:
    """``data``'s chunks in file order, as Office's analysis makes them."""
    chunks: list[Chunk] = []
    at = 0
    while data[at:at + 4] == b"PK\x03\x04" and len(data) - at >= 30:
        (_, _, flags, _, _, _, crc, compressed, size, name_len,
         extra_len) = _LOCAL.unpack_from(data, at)
        h = 30 + name_len + extra_len
        if _FULL in (compressed, size):
            found = _zip64_sizes(data[at + 30 + name_len:at + h], compressed, size)
            if found is None:
                break
            compressed, size = found
        if not chunks and flags & 0x8 and compressed == 0:
            return _simple(data)
        end = at + h + compressed
        if end > len(data):
            break
        name = data[at + 30:at + 30 + name_len].decode("utf-8" if flags & 0x800 else "cp437")
        header = hashlib.sha1(data[at:at + h]).digest()
        sizes = struct.pack("<LQQ", crc, compressed, size)
        if h + compressed <= SMALL:
            chunks.append(Chunk("entry", name, at, end, bytes(a ^ b for a, b in zip(header, sizes))))
        else:
            chunks.append(Chunk("header", name, at, at + h, header))
            chunks.append(Chunk("data", name, at + h, end, sizes))
        at = end
    if not chunks:
        return _simple(data)
    rest = data[at:]
    chunks.append(Chunk("final", "", at, len(data),
                        hashlib.sha1(rest).digest() if len(rest) <= MIB else None))
    return chunks


def layout_problems(data: bytes) -> list[str]:
    """What keeps ``data`` from chunking entry by entry as Office's own saves do: an entry
    with a data descriptor (flag bit 3), a ZIP64 record where no size needs one, a local
    header whose sizes are not the central directory's, or an entry the analysis did not
    reach (the final chunk is not the central directory).  Empty for a good package."""
    problems: list[str] = []
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        infos = sorted(archive.infolist(), key=lambda info: info.header_offset)
        directory = archive.start_dir
    for info in infos:
        at = info.header_offset
        (_, _, flags, _, _, _, crc, compressed, size, name_len,
         extra_len) = _LOCAL.unpack_from(data, at)
        extra = data[at + 30 + name_len:at + 30 + name_len + extra_len]
        if flags & 0x8 or info.flag_bits & 0x8:
            problems.append(f"{info.filename}: a data descriptor (flag bit 3)")
        if _zip64_sizes(extra, 0, 0) is not None and info.compress_size < _FULL \
                and info.file_size < _FULL:
            problems.append(f"{info.filename}: a ZIP64 record where no size needs one")
        if not flags & 0x8 and (crc, compressed, size) != (info.CRC, info.compress_size,
                                                           info.file_size):
            problems.append(f"{info.filename}: its local header's sizes are not the central "
                            "directory's")
    chunks = zip_chunks(data)
    if chunks[-1].kind != "final" or chunks[-1].start != directory:
        problems.append("the analysis does not reach the central directory")
    return problems


def entry_names(data: bytes) -> list[str]:
    """The entries' names in the order the analysis walks them."""
    return [chunk.name for chunk in zip_chunks(data) if chunk.kind in ("entry", "header")]


def changed_chunks(before: bytes, after: bytes) -> tuple[int, int]:
    """How many of ``after``'s chunks a host holding ``before`` does not hold (by signature;
    a random one never), and how many chunks ``after`` has."""
    held = {chunk.signature for chunk in zip_chunks(before)} - {None}
    chunks = zip_chunks(after)
    return sum(chunk.signature not in held for chunk in chunks), len(chunks)


def changed_entries(before: bytes, after: bytes) -> set[str]:
    """The names of the entries with a chunk in ``after`` that a host holding ``before``
    does not hold (the final chunk, which every change changes, left out)."""
    held = {chunk.signature for chunk in zip_chunks(before)} - {None}
    return {chunk.name for chunk in zip_chunks(after)
            if chunk.signature not in held and chunk.kind in ("entry", "header", "data")}


def has_directories(data: bytes) -> bool:
    """Whether ``data`` has directory entries: not parts, so a package does not keep them
    (the one way a save without an edit is not the bytes read)."""
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return any(name.endswith("/") for name in archive.namelist())


def new_problems(before: bytes, after: bytes) -> list[str]:
    """:func:`layout_problems` of ``after`` that ``before`` did not have: a save keeps an
    untouched entry as it was read, a data descriptor too, and adds none of its own."""
    return sorted(set(layout_problems(after)) - set(layout_problems(before)))


def input_order(data: bytes) -> list[str]:
    """``data``'s file entries in the order of their local headers, whether it chunks or
    not."""
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        infos = sorted(archive.infolist(), key=lambda info: info.header_offset)
    return [info.filename for info in infos if not info.is_dir()]


class _Unseekable(io.RawIOBase):
    """A stream zipfile cannot seek back into, so it follows each entry with a data
    descriptor (as some writers do)."""

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


def with_data_descriptors(data: bytes) -> bytes:
    """``data``'s entries written again (same names, order, times, methods and bytes) with a
    data descriptor after each: a made-up package of the kind such writers make."""
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, \
            zipfile.ZipFile(_Unseekable(out), "w") as archive:
        for info in source.infolist():
            entry = zipfile.ZipInfo(info.filename, date_time=info.date_time)
            entry.compress_type = info.compress_type
            with archive.open(entry, "w") as target:
                target.write(source.read(info))
    return out.getvalue()


def entry_records(data: bytes) -> dict[str, bytes]:
    """Each entry's local record -- header, data and any data descriptor -- by name: up to
    the next entry's header, the last up to the central directory."""
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        infos = sorted(archive.infolist(), key=lambda info: info.header_offset)
        ends = [info.header_offset for info in infos[1:]] + [archive.start_dir]
    return {info.filename: data[info.header_offset:end] for info, end in zip(infos, ends)}
