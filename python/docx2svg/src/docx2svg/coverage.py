"""How much of a document a layout covers: the summary a caller reads before trusting it.

The Rust layout always reaches the end: where its measured rules do not reach it lays out by
an approximate rule (``layout-approximate:<reason>``) or draws a marked placeholder of the
item's height (``layout-placeholder:<reason>``), and an absent face is laid out with a
substitute.  Each of those is a warning; this is their sum (:func:`coverage_from`, from
``LaidPages.coverage_summary()``), so that "laid out and nothing was wrong" is told apart from
"could not lay it all out" without reading every warning:

* ``pages`` laid out (``estimated_pages`` the same, ``estimate_source`` ``layout``);
* ``blocks`` (the body's top-level paragraphs and tables), ``blocks_laid_out`` and
  ``blocks_skipped`` -- a block drawn as a placeholder counts as skipped;
* ``placeholders`` -- every placeholder drawn; ``stop`` and ``story_stops`` are always empty
  (the layout never stops);
* ``substituted_fonts`` and ``missing_fonts``;
* ``approximations`` -- every place laid out by a rule no probe measured: its reason,
  message, page and element path.

``complete`` is true only when nothing was drawn as a placeholder and no face was missing.
``status`` says ``complete`` (and nothing approximated), ``approximate`` (complete, with
``approximations``) or ``partial``.

Standard library only.
"""

from __future__ import annotations

from dataclasses import asdict, dataclass, field


@dataclass(frozen=True)
class StopInfo:
    """One stop: the warning's ``code`` (``layout-stopped:table``...), its ``reason`` (the
    part after the colon), its ``message``, the 1-based ``page`` and, for the body's stop,
    the ``path`` of the element it stopped at (:mod:`docx2svg.paths`)."""

    code: str
    reason: str
    message: str | None
    page: int | None = None
    path: str | None = None


@dataclass
class Coverage:
    complete: bool
    pages: int
    estimated_pages: int | None
    #: ``layout`` (the layout reached the end), ``app.xml`` (Word's count as last saved,
    #: which an edit since may have changed), or ``None`` (not known).
    estimate_source: str | None
    blocks: int
    blocks_laid_out: int
    blocks_skipped: int
    stop: StopInfo | None = None
    story_stops: list[StopInfo] = field(default_factory=list)
    #: ``{"family", "substitute", "metric_compatible"}`` per substituted face.
    substituted_fonts: list[dict] = field(default_factory=list)
    #: Faces asked for that are not installed, not embedded and have no substitute here.
    missing_fonts: list[str] = field(default_factory=list)
    #: Every place laid out by a rule no probe measured, without stopping.
    approximations: list[StopInfo] = field(default_factory=list)
    #: Every placeholder drawn where the layout carried on past what it cannot lay out
    #: (``layout-placeholder:<reason>``): its reason, message, page and element path.
    placeholders: list[StopInfo] = field(default_factory=list)

    @property
    def status(self) -> str:
        """``complete``, ``approximate`` (complete, with :attr:`approximations`) or ``partial``."""
        if not self.complete:
            return "partial"
        return "approximate" if self.approximations else "complete"

    def as_dict(self) -> dict:
        return {**asdict(self), "status": self.status}

    def summary(self) -> str:
        """One line: what was laid out, and why not everything."""
        if self.complete:
            text = f"complete: {self.pages} page(s), {self.blocks} block(s)"
            if self.approximations:
                text = (f"complete with approximations: {self.pages} page(s), {self.blocks} block(s); "
                        f"{len(self.approximations)} approximated (first on page {self.approximations[0].page}: "
                        f"{self.approximations[0].reason})")
        else:
            estimate = f" of ~{self.estimated_pages}" if self.estimate_source == "app.xml" else ""
            text = (f"partial: {self.pages}{estimate} page(s), {self.blocks_laid_out} of {self.blocks} block(s) "
                    "laid out")
            if self.stop is not None:
                text += f"; stopped on page {self.stop.page} ({self.stop.reason})"
            if self.story_stops:
                text += f"; {len(self.story_stops)} header/footer/box stop(s)"
            if self.placeholders:
                text += (f"; {len(self.placeholders)} placeholder(s) (first on page {self.placeholders[0].page}: "
                         f"{self.placeholders[0].reason})")
            if self.missing_fonts:
                text += f"; missing faces: {', '.join(self.missing_fonts)}"
            if self.approximations:
                text += f"; {len(self.approximations)} approximated"
        if self.substituted_fonts:
            text += "; substituted: " + ", ".join(f"{s['family']} -> {s['substitute']}"
                                                  + ("" if s["metric_compatible"] else " (approximate)")
                                                  for s in self.substituted_fonts)
        return text


def coverage_from(summary: dict) -> Coverage:
    """The :class:`Coverage` of a version the Rust layout laid out, from its
    ``LaidPages.coverage_summary()``: the layout always reaches the end (it draws a
    placeholder where it cannot lay something out), so there is no stop and the page count is
    its own."""
    def listed(rows) -> list[StopInfo]:
        return [StopInfo(code, reason, message or None, page, path) for code, reason, message, page, path in rows]

    blocks, laid_out = summary["blocks"], summary["laid_out"]
    substituted = [{"family": family, "substitute": substitute, "metric_compatible": compatible, "kind": kind,
                    "class": face_class}
                   for family, substitute, compatible, kind, face_class in summary["substituted"]]
    return Coverage(summary["complete"], summary["pages"], summary["pages"], "layout", blocks, laid_out,
                    blocks - laid_out, None, [], substituted, list(summary["missing"]),
                    listed(summary["approximations"]), listed(summary["placeholders"]))
