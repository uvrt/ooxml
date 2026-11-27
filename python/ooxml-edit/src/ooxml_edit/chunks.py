"""A saved file's chunks for a co-authoring host (MS-FSSHTTPD, request 4): the shared chunk
writer of the Rust core, the public home of what ``ooxml_core.chunk_file`` was.

    from ooxml_edit.chunks import chunk_file

    chunks = chunk_file(saved)                          # FileChunks: chunks, leaves(), root
    delta = chunks.delta(chunk_file(previous).leaves())  # ChunkDelta: new, held, repeated, new_bytes
"""

from __future__ import annotations

from ooxml_core import ChunkDelta, FileChunk, FileChunks, chunk_file

__all__ = ["ChunkDelta", "FileChunk", "FileChunks", "chunk_file"]
