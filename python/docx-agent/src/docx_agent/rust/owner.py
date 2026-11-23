"""docx-agent's documents, owned by the Rust session.

A Word document open in a toolbox session is held by the process's Rust session
(``ooxml_core.Session``, the one :mod:`.reads` reads): its versions, its steps, its ids and
its saved bytes are the session's.  :class:`RustOwner` is the format's
:attr:`~ooxml_edit.tools.session.DocumentFormat.owner` (the protocol is
:mod:`ooxml_edit.tools.owned`'s):

- a document is opened from its bytes (and the id state saved beside them) in the session
  alone (:meth:`RustOwner.open_data`); no Python document is made;
- every tool runs in the session (:mod:`.edits`, :mod:`.reads`);
- undo and redo are the session's own ``undo`` tool; the save is the session's bytes;
- a changing call's checks read the session's validation (:func:`owned_validation`).

There is no Python view: the protocol's view methods (``follow``, ``bridge``, ``reader``'s
document) refuse as ``internal``, so a path that still reads one is found at once.
"""

from __future__ import annotations

import json
import warnings
import weakref
from collections.abc import Mapping
from typing import Any

from ooxml_edit.errors import ArgumentError
from ooxml_edit.tools import ToolError

from .. import finalizers
from . import reads


class TemplateOpened(UserWarning):
    """A template (``.dotx``, ``.dotm``) was opened to edit: edits change the template
    itself; ``new_document`` with the template's blob makes a document from it."""


class IdStateError(ArgumentError):
    """The id state given beside a document's bytes was refused: not one docx-agent wrote,
    or not the state of these bytes (the session's words say what to pass instead)."""


def caller_key(caller: Mapping[str, Any] | None) -> str | None:
    """A call's context (:attr:`~ooxml_edit.tools.dispatch.Call.caller`) as the session's
    caller: its JSON, keys sorted; ``None`` for none (the session's own)."""
    if not caller:
        return None
    return json.dumps(dict(caller), sort_keys=True, default=str, separators=(",", ":"))


def context_of(caller: Mapping[str, Any] | None, limits: Any = None,
               layout_timeout: float | None = None, *, date: str | None = None,
               typography: bool = True, locale: Any = None) -> Any:
    """An ``ooxml_core.CallContext`` for ``caller``: its key, and its context in the order
    given (as the session's replies name the caller: ``agent=b``); with the host's ``limits``
    on its reply (an :class:`ooxml_edit.tools.Limits`, as an ``ooxml_core.Limits``), how long
    laying the document out may take in it (``layout_timeout``, seconds; the session's own
    where ``None``), its writes dated ``date`` (UTC to the second, the session clock's) and
    spaced by their language unless ``typography`` is off, and read under ``locale`` -- the
    last three as an ``ooxml_core.CallWriting`` (a save's ``app.xml`` pages and lines are
    the document's page engine's)."""
    import ooxml_core

    given: dict[str, Any] = {"layout_timeout": layout_timeout}
    if caller:
        given["caller"] = caller_key(caller)
        given["context"] = json.dumps(dict(caller), default=str)
    if date is not None or not typography or locale is not None:
        given["writing"] = ooxml_core.CallWriting(date=date, typography=typography, locale=locale)
    if limits is not None:
        given["limits"] = ooxml_core.Limits(result_chars=limits.max_result_chars,
                                            list_items=limits.max_list_items,
                                            images_per_call=limits.max_images_per_call,
                                            image_edge=limits.max_image_edge)
    return ooxml_core.CallContext(**given)


def options(**given: Any) -> Any:
    """An ``ooxml_core.CallOptions``: what one ``Session.call`` or ``Session.edit`` runs with
    beside its name and arguments (``context``, ``documents``, ``layout``, ``blobs``,
    ``written``, ``limit``)."""
    import ooxml_core

    return ooxml_core.CallOptions(**given)


def documents(**given: Any) -> Any:
    """An ``ooxml_core.CallDocuments``: what a call needs about the open documents it names
    (``at``, ``names``, ``tables``, ``sources``)."""
    import ooxml_core

    return ooxml_core.CallDocuments(**given)


def _no_view(what: str) -> ToolError:
    return ToolError("internal", f"{what}: a Word document has no Python view; the Rust session holds it")


class RustOwner:
    """The Rust session as the owner of docx-agent's documents in a toolbox session; its
    handle per document is the :class:`~.reads.Mirror` the reads use.  ``settings``: the
    application's settings of the format (``docx_format``: ``min_text_size``,
    ``typography``), which every document's calls read."""

    def __init__(self, **settings: Any) -> None:
        self.settings = {key: value for key, value in settings.items() if value is not None}

    def open_data(self, data: bytes, state: Any, name: str) -> "reads.Mirror":
        """Open ``data`` (a Word package's bytes; ``state`` the id state saved beside them,
        or ``None``) in the Rust session alone, as ``name``: the session reads its parts once,
        its ids against the state, and keeps the ids the state retired.  A state that is not
        these bytes' is refused (:class:`IdStateError`, in the session's words); a package
        without a Word document's main part too; a template warns (:class:`TemplateOpened`)."""
        import ooxml_core

        finalizers.drain()
        held = reads.session()
        try:
            if state is None:
                doc, numbers = held.open(data, kind="docx", name=name), (0, 0)
            else:
                doc, version, last = held.open_state(data, state, name=name)
                numbers = (version, last)
        except ooxml_core.StateRefused as exc:
            raise IdStateError(str(exc)) from None
        except ooxml_core.NotWordPackage:
            raise ArgumentError("not a Word package: no main document part") from None
        mirror = reads.Mirror(held, doc, self.settings)
        # the finalizer only hands the close over (docx_agent.finalizers): never inside the collector
        weakref.finalize(mirror, finalizers.later, reads.close, held, doc)
        with mirror.lock:
            mirror.opened = held.head(doc)
            mirror.numbers_opened = numbers
        kind = mirror.opened.file_kind()
        if kind in ("dotx", "dotm"):
            warnings.warn(TemplateOpened(
                f"this file is a template (.{kind}): editing it edits the template. To make a document "
                "from it use new_document with its blob; saving as docx writes a document either way"),
                stacklevel=4)
        return mirror

    def opened(self, mirror: "reads.Mirror") -> tuple[int, int]:
        """The version the document was opened at and the highest its history gave (its
        state's, ``(0, 0)`` for bytes opened alone): the tool layer's numbers go on from them."""
        return mirror.numbers_opened

    def open(self, document: Any) -> Any:
        """(A Python document is never held: documents are opened from their bytes.)"""
        raise _no_view("a Python document handed to the owner")

    def head(self, mirror: "reads.Mirror") -> Any:
        return mirror.session.head(mirror.doc)

    def parts_between(self, mirror: "reads.Mirror", before: Any, after: Any) -> list[str]:
        return after.parts_changed_since(before)

    def touched(self, mirror: "reads.Mirror", before: Any, after: Any) -> list[dict[str, Any]]:
        return json.loads(after.touched_since(before))

    def follow(self, mirror: "reads.Mirror", document: Any, head: Any, number: int, last: int) -> None:
        raise _no_view("the view followed")

    def bridge(self, mirror: "reads.Mirror", tool: str, caller: Mapping[str, Any] | None,
               parts: dict[str, bytes | None]) -> Any:
        raise _no_view(f"{tool} changed the view")

    def history(self, mirror: "reads.Mirror", name: str, steps: int, redo: bool,
                caller: Mapping[str, Any] | None) -> dict[str, Any]:
        """The session's ``undo`` tool on the document, named ``name`` in its reply."""
        arguments = {"doc": name, "steps": steps, "redo": redo}
        reply, _images, _app = mirror.session.call("undo", json.dumps(arguments), options(
            context=context_of(caller), documents=documents(names={name: mirror.doc})))
        return json.loads(reply)

    def reader(self, mirror: "reads.Mirror", head: Any, number: int) -> Any:
        """What would make a Python reader of ``head``: none is made (the reads are the
        session's); one asked for refuses."""

        def make(data: bytes) -> Any:
            raise _no_view(f"a reader of version {number}")

        return make

    def keep(self, mirror: "reads.Mirror", head: Any, first: int | None = None) -> None:
        with mirror.lock:
            mirror.keep(head, first)

    # -- the history's numbers: the session's -------------------------------------------------

    def continue_numbers(self, mirror: "reads.Mirror", state: int, highest: int) -> None:
        with mirror.lock:
            mirror.session.continue_numbers(mirror.doc, state, highest)
            mirror.opened = mirror.session.head(mirror.doc)

    def numbered(self, mirror: "reads.Mirror", number: int) -> Any:
        return mirror.session.numbered(mirror.doc, number)

    def highest(self, mirror: "reads.Mirror") -> int:
        return mirror.session.highest(mirror.doc)

    def numbers(self, mirror: "reads.Mirror") -> Any:
        return mirror.session.numbers(mirror.doc)

    # -- step groups: a batch's ops as one step of the session's --------------------------------

    def open_group(self, mirror: "reads.Mirror") -> "_Group":
        with mirror.lock:
            return _Group(mirror.session.open_group(mirror.doc), mirror.tracking)

    def group_mark(self, mirror: "reads.Mirror", group: "_Group") -> tuple[Any, Any]:
        with mirror.lock:
            return group.steps.mark(), mirror.tracking

    def revert_group(self, mirror: "reads.Mirror", group: "_Group", mark: tuple[Any, Any] | None,
                     caller: Mapping[str, Any] | None) -> Any:
        """What the group committed after ``mark`` taken back (the session's tracking mode
        too, as it was then): the new head, or ``None``."""
        with mirror.lock:
            made = group.steps.revert(mark[0] if mark is not None else None, context=context_of(caller))
            mirror.tracking = mark[1] if mark is not None else group.tracking
            return made

    def close_group(self, mirror: "reads.Mirror", group: "_Group", caller: Mapping[str, Any] | None,
                    tool: str | None = None) -> Any:
        """The group closed as one step: its head, or ``None``.  What it committed reflowed is
        measured once first, while it is open (its ops measure none of their own), for the
        batch's checks (:func:`group_reflow`)."""
        with mirror.lock:
            context = context_of(caller)
            reflow = group.steps.reflow(context=context)
            head = group.steps.close(context=context, tool=tool)
            mirror.group_reflow = (head.number, reflow) if head is not None and reflow is not None else None
            return head

    def save(self, mirror: "reads.Mirror") -> bytes:
        return bytes(mirror.session.save(mirror.doc))

    def close(self, mirror: "reads.Mirror") -> None:
        reads.close(mirror.session, mirror.doc)


class _Group:
    """A document's open step group: the session's (``ooxml_core.StepGroup``) and the
    tracking mode the session edited under when it opened."""

    __slots__ = ("steps", "tracking")

    def __init__(self, steps: Any, tracking: Any) -> None:
        self.steps = steps
        self.tracking = tracking


def group_reflow(entry: Any, head: int) -> Any:
    """The reflow of the step group that closed at the session's version ``head`` (a batch's,
    measured once before it closed), parsed; ``None`` where none was measured for it."""
    found = reads.owned(entry).handle.group_reflow
    if found is None or found[0] != head:
        return None
    return json.loads(found[1])


def _validation(found: Any) -> dict[str, Any] | None:
    if found is None:
        return None
    new, fixed, baseline = found
    return {"new": list(new), "fixed": list(fixed), "baseline": baseline}


def owned_validation(entry: Any) -> dict[str, Any] | None:
    """The validation delta of the document's head, the session's, against the problems it
    opened with."""
    mirror = reads.owned(entry).handle
    return _validation(mirror.session.validation(mirror.doc))


def session_validation(entry: Any) -> dict[str, Any] | None:
    """The save gate's comparison of a document (or of a reader of one of its committed
    versions), at the entry's version: the session's, against the problems it opened with --
    the format's :attr:`~ooxml_edit.tools.DocumentFormat.validation`.  ``None`` for a version
    it let go."""
    mirror = reads.owned(entry).handle
    with mirror.lock:
        version = mirror.versions.get(entry.version)
    if version is None:
        return None
    return _validation(mirror.session.validation(mirror.doc, at=version))


OWNER = RustOwner()
