"""Made-up Word documents and what a test reads of them, through the toolbox only (P4e: no
Python view): a blank package from the Rust core, a body of raw WordprocessingML put into one,
and a document's blocks, ids and validation read with ``word_read`` and ``check``.

Must not: import pytest or docx-agent's library (the gates' environment has no pytest, and
the library has no document API).
"""

from __future__ import annotations

import io
import re
import zipfile
from datetime import datetime, timezone
from typing import Any

#: When the blank documents were made: fixed, so their bytes are the same each run.
CREATED = datetime(2026, 10, 4, tzinfo=timezone.utc)

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_BLOCK = re.compile(r"^<!-- ((?:p|t):[0-9A-Fa-f]+)(?: ([^>]*?))? -->\n(.*?)(?=^<!-- (?:p|t):|\Z)", re.M | re.S)


def blank(*, page: str = "A4", landscape: bool = False, language: str = "en-US",
          separators: tuple[str, str] = (".", ","), title: str | None = None, author: str | None = None,
          created: datetime = CREATED) -> bytes:
    """A new blank document's package as the Rust core makes it (``ooxml_core.new_document``,
    what ``new_document`` writes)."""
    import ooxml_core

    stamp = created.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return bytes(ooxml_core.new_document(page=(page, landscape), language=language, separators=separators,
                                         properties=(title, author, stamp)))


def replaced(data: bytes, parts: dict[str, Any]) -> bytes:
    """``data`` with each named part's bytes replaced: a value, or a function of the part's
    text giving its new text."""
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for info in source.infolist():
            content = source.read(info.filename)
            given = parts.get(info.filename)
            if callable(given):
                content = given(content.decode("utf-8")).encode("utf-8")
            elif given is not None:
                content = given if isinstance(given, bytes) else given.encode("utf-8")
            target.writestr(info, content)
    return out.getvalue()


def with_body(body: str, data: bytes | None = None) -> bytes:
    """A blank document (or ``data``) whose body is ``body`` (WordprocessingML with the ``w:``
    prefix) before its section properties."""

    def put(xml: str) -> str:
        start = xml.index("<w:body>") + len("<w:body>")
        end = xml.rindex("<w:sectPr")
        return xml[:start] + body + xml[end:]

    return replaced(data if data is not None else blank(), {"word/document.xml": put})


def paragraph(text: str = "", props: str = "") -> str:
    """A paragraph of one run of ``text`` (and ``props``, its ``w:pPr`` content)."""
    run = f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>' if text else ""
    return f"<w:p>{f'<w:pPr>{props}</w:pPr>' if props else ''}{run}</w:p>"


def call(box: Any, session: Any, tool: str, **arguments: Any) -> Any:
    """``tool``'s result, which must be ok."""
    result = box.dispatch(session, tool, arguments)
    assert result.ok, result.to_json()
    return result


def read(box: Any, session: Any, doc: str, **arguments: Any) -> str:
    """The whole of ``word_read``'s Markdown, every page of it."""
    out, cursor = [], None
    while True:
        result = call(box, session, "word_read", doc=doc, **({"cursor": cursor} if cursor else {}), **arguments)
        out.append(result.data)
        cursor = result.next_cursor
        if not cursor:
            return "".join(out)


def blocks(box: Any, session: Any, doc: str, **arguments: Any) -> list[tuple[str, str]]:
    """The body's blocks as ``word_read`` says them: (id, Markdown text), in order; an empty
    paragraph's text is ``""``."""
    return [(found.group(1), found.group(3).strip()) for found in _BLOCK.finditer(read(box, session, doc, **arguments))]


def ids(box: Any, session: Any, doc: str) -> dict[str, str]:
    """The id of each block by its text (the first block of a text)."""
    out: dict[str, str] = {}
    for block, text in blocks(box, session, doc):
        out.setdefault(text, block)
    return out


def paragraph_ids(box: Any, session: Any, doc: str) -> list[str]:
    """The body's paragraphs' ids, in order (tables left out)."""
    return [block for block, _ in blocks(box, session, doc) if block.startswith("p:")]


def validation(box: Any, session: Any, doc: str) -> dict[str, Any]:
    """``check``'s ``validate``: the problems now against those it opened with."""
    return call(box, session, "check", doc=doc, include=["validate"]).data["validate"]


def saved(session: Any, doc: str) -> bytes:
    """The document's bytes as the session saves them."""
    return session.save(doc).data


def part(data: bytes, name: str = "word/document.xml") -> str:
    """A part of a package, as text."""
    with zipfile.ZipFile(io.BytesIO(data)) as package:
        return package.read(name).decode("utf-8")
