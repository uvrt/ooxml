"""A saved file's chunks for a co-authoring host (MS-FSSHTTPD, request 4): the shared chunk
writer of the Rust core, the public home of what ``ooxml_core.chunk_file`` was; and what
changed in a session's document since the host last sent it, from the changes feed, so the
host saves and chunks only when something did.

    from ooxml_edit.chunks import chunk_file, file_changes

    changed = file_changes(session, doc, since=sent.token)   # FileChanges: version, token, parts, full
    if changed.parts:
        chunks = chunk_file(saved)                           # FileChunks: chunks, leaves(), root
        delta = chunks.delta(chunk_file(previous).leaves())  # ChunkDelta: new, held, repeated, new_bytes
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from ooxml_core import ChunkDelta, FileChunk, FileChunks, chunk_file

__all__ = ["ChunkDelta", "FileChanges", "FileChunk", "FileChunks", "chunk_file", "file_changes"]


@dataclass(frozen=True)
class FileChanges:
    """:func:`file_changes`'s answer: the ``version`` answered (the store's seq), its
    ``token`` (the next ``since``), the package ``parts`` changed after ``since`` (sorted;
    every part where ``full``), and whether ``since`` could not be answered exactly
    (``full``: none given, another session's, or past what the feed keeps) -- then the host
    sends the file whole."""

    version: int
    token: str
    parts: tuple[str, ...]
    full: bool


def file_changes(session: Any, doc: str, since: str | None = None) -> FileChanges:
    """What changed in the document ``doc`` of ``session`` (a hosted session) after
    ``since`` (a :class:`FileChanges` ``token``), from the changes feed without pages: never
    laid out.  No parts: nothing to send."""
    answer = session.changes(doc, since, pages=False)
    return FileChanges(answer["version"], answer["token"], tuple(answer["parts"]), answer["full"])
