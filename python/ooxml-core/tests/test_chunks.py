"""``ooxml_core.chunk_file``: a saved file's MS-FSSHTTPD 2.4.1 chunks for a co-authoring host
(the WOPI host stores a version's leaves and sends only a new version's changed chunks), on
the probe side's test vectors (crates/ooxml-zip/tests/vectors; the Rust tests hold every
vector byte for byte, these hold the module's view of them)."""

from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import ooxml_core
import pytest

VECTORS = Path(__file__).resolve().parents[3] / "crates" / "ooxml-zip" / "tests" / "vectors"


def _read(name: str) -> bytes:
    return (VECTORS / name).read_bytes()


def test_a_file_is_chunked_as_office_chunks_it():
    data = _read("v2-large-entries.zip")
    chunks = ooxml_core.chunk_file(data)
    assert chunks.zip_analysed and chunks.size == len(data) == 13734
    assert [(c.kind, c.name, c.offset, c.size) for c in chunks.chunks] == [
        ("header", "doc/body.xml", 0, 42), ("data", "doc/body.xml", 42, 7394),
        ("header", "media/raw.bin", 7436, 43), ("data", "media/raw.bin", 7479, 6000),
        ("entry", "doc/small.xml", 13479, 57), ("final", "", 13536, 198)]
    assert chunks.chunks[1].signature.hex() == "7a7a4136e21c0000000000002f23000000000000"
    assert chunks.chunks[4].signature.hex() == "f6f8d9f410f86b618f9bee4a2756204c9a9d3f2c"
    assert not any(c.random or c.subchunks for c in chunks.chunks)
    assert chunks.root == (b"", 13734)
    assert chunks.leaves()[0] == (chunks.chunks[0].signature, 42)
    assert len(chunks) == 6


def test_a_chunk_over_a_mebibyte_has_random_subchunks():
    data = _read("v3-over-1mb.head") + bytes(2_500_000) + _read("v3-over-1mb.tail")
    first, second = ooxml_core.chunk_file(data), ooxml_core.chunk_file(data)
    big = first.chunks[1]
    assert (big.size, big.signature.hex()) == (2_500_000, "8644a576a025260000000000a025260000000000")
    assert [(o, s, len(sig)) for o, s, sig in big.subchunks] == [
        (43, 1_048_576, 8), (43 + 1_048_576, 1_048_576, 8), (43 + 2 * 1_048_576, 402_848, 8)]
    # random signatures differ between writers; the parent leaf is reused whole
    assert big.subchunks[0][2] != second.chunks[1].subchunks[0][2]
    delta = second.delta(first.leaves())
    assert delta.new == [] and delta.held == [0, 1, 2, 3] and delta.new_bytes == 0


def test_a_file_not_analysed_is_cut_into_simple_chunks():
    data = _read("v4-data-descriptor.zip")
    chunks = ooxml_core.chunk_file(data)
    assert not chunks.zip_analysed
    assert [(c.kind, c.offset, c.size) for c in chunks.chunks] == [("simple", 0, 190)]
    assert chunks.chunks[0].signature.hex() == "8a1a522de8c36378038c0ff8d0fd88548db3f793"
    empty = ooxml_core.chunk_file(b"")
    assert (empty.zip_analysed, len(empty), empty.root) == (False, 0, (b"", 0))


def test_only_changed_chunks_travel():
    v1, v2 = _read("v1-small-entries.zip"), _read("v2-large-entries.zip")
    held = ooxml_core.chunk_file(v1).leaves()
    assert ooxml_core.chunk_file(v1).delta(held).held == [0, 1, 2, 3]
    # v1's first entry twice, then the rest: the repeat travels once
    twice = v1[:127] + v1
    delta = ooxml_core.chunk_file(twice).delta([])
    assert delta.new[:1] == [0] and delta.repeated[:1] == [(1, 0)]
    delta = ooxml_core.chunk_file(v2).delta(held)
    assert delta.new == [0, 1, 2, 3, 4, 5] and delta.new_bytes == len(v2)


def test_a_signature_longer_than_a_sha1_is_refused():
    with pytest.raises(ValueError, match="at most 20 bytes"):
        ooxml_core.chunk_file(b"").delta([(bytes(21), 1)])


def test_files_chunk_in_parallel():
    data = _read("v2-large-entries.zip")
    with ThreadPoolExecutor(4) as pool:
        found = list(pool.map(lambda _: ooxml_core.chunk_file(data).leaves(), range(16)))
    assert all(leaves == found[0] for leaves in found)
