"""The docx format's hooks for a document the Rust session owns (lane H7a): ooxml-edit gives
them an :class:`~ooxml_edit.tools.OwnedDocument` -- the owner, its handle (the
:class:`~.reads.Mirror`: the session and the document's id there) and the version read --
and each answers from the session's reading of that version, never from the Python view.

- :func:`summary` -- what ``open_document`` says of it (``Version.summary``);
- :func:`problems` -- its validation problems (``Version.problems``), as
  :class:`docx_agent.validate.Problem` items;
- :func:`id_state` -- the state kept beside its bytes (``Version.id_state``), numbered as the
  tool layer numbers its states, the retired ids as the session records them
  (:func:`.owner.retired_ids`);
- :func:`block_kind` and :func:`has_drawing` -- what a source value names in it, for the
  ``own_source`` and ``source_hint`` refusals (lane H7d: ``Version.block_kind``,
  ``Version.has_drawing``);
- :func:`provenance` -- the records of placed values it keeps (``Version.records``), for
  ``read_provenance`` (lane H7d);
- :func:`number_texts` and :func:`scale_values` -- what a check of numbers and a scale read
  in it (lane H7e: ``Version.number_texts``, ``Version.scale_values``), placed with the
  session's own engine for the document (:func:`.engines.session_pages`);
- :func:`closed` and :func:`restored` -- nothing of the session's: the session lets go of the
  document as the owner closes it, and what a batch or scoped undo puts back is its own
  version; only what docx-agent kept in this process for a view made of it goes.
"""

from __future__ import annotations

import json
from typing import Any, Mapping

from . import reads


def owned(subject: Any) -> Any:
    """``subject`` where it is an owned document (an ``OwnedDocument``), else ``None``."""
    from ooxml_edit.tools import OwnedDocument

    return subject if isinstance(subject, OwnedDocument) else None


def _mirror(subject: Any) -> "reads.Mirror":
    mirror = subject.handle
    if not isinstance(mirror, reads.Mirror):
        raise TypeError(f"not a document the Rust session owns: {mirror!r}")
    return mirror


def version_of(subject: Any) -> Any:
    """The session's ``Version`` the hook reads: the head the calls read now, or the
    committed version ``subject.version`` (a read copy's: kept by the tool layer's numbers,
    else the newest the session numbers so).  A version let go since is refused."""
    mirror = _mirror(subject)
    if subject.version is None:
        return mirror.session.head(mirror.doc)
    with mirror.lock:
        found = mirror.versions.get(subject.version)
    if found is None:
        # (committed and read before the tool layer kept it here: the session finds it)
        found = mirror.session.numbered(mirror.doc, subject.version)
    if found is None:
        raise LookupError(f"version {subject.version} of {mirror.doc} is no longer kept")
    return found


def summary(subject: Any) -> dict[str, Any]:
    """The format's ``summary`` of an owned document: cheap facts, no layout."""
    found = version_of(subject).summary()
    return dict(json.loads(found)) if found is not None else {}


def problems(subject: Any) -> list[Any]:
    """The format's ``problems`` of an owned document: the version's, as the session finds
    them (the save gate compares them with those it opened with by ``problem_key``)."""
    from ..validate import Problem

    found = version_of(subject).problems() or []
    return [Problem(code, part, detail) for code, part, detail in found]


def id_state(subject: Any) -> str:
    """The format's ``state`` of an owned document: ``Document.id_state``'s string, from the
    ids the session holds (the same record docx-agent's view writes; the deflate bytes may
    differ, and either opens the other's)."""
    from .owner import retired_ids

    mirror = _mirror(subject)
    version = version_of(subject)
    number = version.state
    last = max(number, mirror.session.highest(mirror.doc))
    retired = retired_ids(mirror)
    state = version.id_state(
        number, last, gone=list(retired.gone.items()),
        drawings_gone=list(retired.drawings_gone.items()), earlier=list(retired.earlier.items()),
        said=list(retired.said.items()))
    if state is None:
        raise TypeError(f"{mirror.doc} is not a Word document")
    return str(state)


def block_kind(subject: Any, identifier: str) -> str | None:
    """What kind of block ``identifier`` names in an owned document: ``"paragraph"``,
    ``"table"`` or ``"content control"`` (a block-level one), a renamed id followed; ``None``
    when it names none of its blocks."""
    found = version_of(subject).block_kind(identifier)
    return str(found) if found is not None else None


def has_drawing(subject: Any, identifier: str) -> bool:
    """Whether an owned document has the drawing ``identifier`` (``d:3``)."""
    return bool(version_of(subject).has_drawing(identifier))


def provenance(subject: Any) -> dict[str, Any]:
    """``{address: record}`` for the tables and charts an owned document has (tracked changes
    accepted), as the session reads its records: ``Document.provenance``'s."""
    found = version_of(subject).records()
    if found is None:
        return {}
    return {str(key): dict(record) for key, record in json.loads(found)["provenance"]}


def live_view(subject: Any) -> Any:
    """The Python view docx-agent made of an owned document in this process, as it is (not
    made current), or ``None``."""
    live = getattr(_mirror(subject), "live", None)
    return live() if live is not None else None


def number_texts(call: Any, subject: Any, scope: Mapping[str, Any]) -> list[Any]:
    """The format's ``number_texts`` for an owned document (lane H7e): the texts a check of
    numbers reads in the version, as the session reads them (``Version.number_texts``),
    each placed on its page by the session's own engine for the document."""
    from ooxml_edit.tools import NumberText

    from ..tools.shared import _pages_of

    pages = _pages_of(call, scope.get("slides"), scope.get("pages"))
    request = {"range": _ranges(call, scope.get("range")), "pages": pages}
    found = _values(call, subject, "number_texts", request, pages, always=True)
    return [NumberText(text["at"], text["text"], tuple((a, b) for a, b in text["written"]), {"range": text["at"]},
                       {"page": text["page"]} if text["page"] is not None else {})
            for text in (found or {}).get("texts", ())]


def scale_values(call: Any, subject: Any, scale: Any, scope: Mapping[str, Any]) -> list[Any]:
    """The format's ``scale_values`` for an owned document (lane H7e): the values the
    scale's targets hold in the version, each with the step it is styled as now, as the
    session reads them (``Version.scale_values``); with ``pages``, placed by the session's
    own engine for the document."""
    from ooxml_edit.tools import ScaleValue

    from ..tools.shared import _pages_of

    pages = _pages_of(call, scope.get("slides"), scope.get("pages"))
    request = {"scale": {"name": scale.name, "targets": list(scale.targets), "labels": scale.labels,
                         "steps": [{"fill": s.fill, "text": s.text, "label": s.label} for s in scale.steps]},
               "range": _ranges(call, scope.get("range")), "pages": pages}
    found = _values(call, subject, "scale_values", request, pages, always=False)
    out = []
    for value in (found or {}).get("values", ()):
        handle = value["handle"]
        if handle is not None:
            kind, where, ids = handle
            handle = (kind, tuple(where) if kind == "range" else where, list(ids))
        out.append(ScaleValue(value["target"], value["text"], given=value["given"], now=value["now"],
                              exact=value["exact"], differs=value["differs"], skip=value["skip"],
                              where={"page": value["page"]} if value["page"] is not None else {},
                              handle=handle, kept=value["kept"], source=value["source"]))
    return out


def _ranges(call: Any, value: Any) -> list[str]:
    """``range`` as a list of addresses, refs resolved (a heading's range is read as its
    span by the session, as ``check`` reads it)."""
    from ..scope import ranges_of
    from ..tools._base import refs_resolved

    return ranges_of(refs_resolved(call, value))


def _drawn_as(call: Any) -> Any:
    """What the document of ``call`` is laid out with -- its font folders and locale, settings
    the toolbox keeps on its view (``set_language``), not what the view holds of the
    document --; the toolbox session's where the call's entry is not one the session owns."""
    owned = reads._owned(call.entry)
    return owned.held if owned is not None else call.session


def _values(call: Any, subject: Any, method: str, request: dict[str, Any], pages: Any,
            always: bool) -> dict[str, Any] | None:
    """The session's reply to ``Version.<method>`` for ``request`` (the session's engine for
    the document given where its texts are placed: always, or where ``pages`` are asked
    for); its warnings said, its refusal raised."""
    from ooxml_edit.tools import ToolError

    from .engines import session_pages
    from .refusals import refusal

    engine = None
    if always or pages is not None:
        engine, why = session_pages(_mirror(subject), _drawn_as(call))
        if engine is None:
            if pages is not None:
                raise ToolError("layout_failed", f"numbers on pages need the layout: {why}; leave pages out "
                                "to read the whole body, or give range", field="pages")
            call.warn("numbers: the layout failed (layout_failed): no pages said")
    reply = getattr(version_of(subject), method)(call.entry.doc_id, json.dumps(request), engine)
    found = json.loads(reply)
    if isinstance(found, dict) and found.get("error") is not None:
        raise refusal(found["error"])
    for warning in (found or {}).get("warnings", ()):
        call.warn(warning)
    return found
