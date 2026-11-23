"""What a conversion raises when the layout does not lay a document out."""

from __future__ import annotations

#: What a caller can do instead, said with every refusal.
WAY_OUT = ("read the document as text instead (docx-agent's word_read), or open it in Word; the forms the "
           "layout does not lay out yet are listed in docx-agent's SUPPORTED.md")


class LayoutError(NotImplementedError):
    """The layout (``ooxml_core``, in Rust) does not lay this document out yet: ``reason`` says
    what and where, ``way_out`` what to do instead.  The Rust layout's own refusal is the
    exception's ``__cause__``.  A subclass of :class:`NotImplementedError`: what is not laid
    out is a form not supported yet, never a failure of the document.  ``path`` is kept for the
    callers that read it; it is ``None`` (the reason names where)."""

    def __init__(self, reason: str, way_out: str = WAY_OUT) -> None:
        super().__init__(reason, way_out)
        self.reason = reason
        self.way_out = way_out
        self.path: str | None = None

    def __str__(self) -> str:
        return f"{self.reason}: {self.way_out}"


def failure(path: str, error: BaseException) -> LayoutError:
    """``error`` met laying out the element at ``path`` (docx2svg's former Python layout,
    which goes in the next release)."""
    found = LayoutError(f"laying out {path} failed: {type(error).__name__}: {error}")
    found.path = path
    return found
