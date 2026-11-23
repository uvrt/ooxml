"""The docx format's hooks for a document the Rust session owns (lane H7a): ooxml-edit gives
them an :class:`~ooxml_edit.tools.OwnedDocument` -- the owner, its handle (the
:class:`~.reads.Mirror`: the session and the document's id there) and the version read --
and each answers from the session's reading of that version, never from the Python view.

- :func:`summary` -- what ``open_document`` says of it (``Version.summary``);
- :func:`problems` -- its validation problems (``Version.problems``), as :class:`Problem`
  items;
- :func:`id_state` -- the state kept beside its bytes (``Version.id_state``), numbered as the
  tool layer numbers its states, the retired ids as the session records them;
- :func:`block_kind` and :func:`has_drawing` -- what a source value names in it, for the
  ``own_source`` and ``source_hint`` refusals (lane H7d: ``Version.block_kind``,
  ``Version.has_drawing``);
- :func:`provenance` -- the records of placed values it keeps (``Version.records``), for
  ``read_provenance`` (lane H7d);
- :func:`number_texts` and :func:`scale_values` -- what a check of numbers and a scale read
  in it (lane H7e: ``Version.number_texts``, ``Version.scale_values``), placed with the
  session's own engine for the document (:func:`.engines.session_pages`);
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Mapping

from . import reads
from .args import pages_of, ranges_of


@dataclass(frozen=True)
class Problem:
    """One validation problem the session found: its code, the part and what it says."""

    code: str
    part: str
    detail: str = ""

    def __str__(self) -> str:
        return f"[{self.code}] {self.part}: {self.detail}"


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
    found = version_of(subject).problems() or []
    return [Problem(code, part, detail) for code, part, detail in found]


def id_state(subject: Any) -> str:
    """The format's ``state`` of an owned document: the state kept beside its bytes, from the
    ids the session holds, the retired ones each numbered by the tool layer's number of the
    version that retired it (one whose call is still ending is left out until it is)."""
    mirror = _mirror(subject)
    version = version_of(subject)
    number = version.state
    last = max(number, mirror.session.highest(mirror.doc))
    gone: dict[int, int] = {}
    drawings_gone: dict[int, int] = {}
    earlier: dict[int, Any] = {}
    said: dict[int, str] = {}
    for kind, value, seq, caller, tool, then, words in mirror.session.retired(mirror.doc):
        if seq is not None:
            then = mirror.numbers.get(seq)
            if then is None:
                continue
            if tool:
                context = ", ".join(f"{key}={found}" for key, found in json.loads(caller).items()) if caller else ""
                said[then] = f"{tool} ({context})" if context else tool
        else:
            earlier[then] = words
        (gone if kind == "block" else drawings_gone)[value] = then
    state = version.id_state(number, last, gone=list(gone.items()), drawings_gone=list(drawings_gone.items()),
                             earlier=list(earlier.items()), said=list(said.items()))
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


def number_texts(call: Any, subject: Any, scope: Mapping[str, Any]) -> list[Any]:
    """The format's ``number_texts`` for an owned document (lane H7e): the texts a check of
    numbers reads in the version, as the session reads them (``Version.number_texts``),
    each placed on its page by the session's own engine for the document."""
    from ooxml_edit.tools import NumberText

    pages = pages_of(call, scope.get("slides"), scope.get("pages"))
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

    pages = pages_of(call, scope.get("slides"), scope.get("pages"))
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
    return [reads.refs(call, item) for item in ranges_of(value)]


def _values(call: Any, subject: Any, method: str, request: dict[str, Any], pages: Any,
            always: bool) -> dict[str, Any] | None:
    """The session's reply to ``Version.<method>`` for ``request`` (the session's engine for
    the document given where its texts are placed: always, or where ``pages`` are asked
    for); its warnings said, its refusal raised."""
    from .engines import session_pages
    from .refusals import refusal

    engine = session_pages(reads.owned(call.entry)) if always or pages is not None else None
    reply = getattr(version_of(subject), method)(call.entry.doc_id, json.dumps(request), engine)
    found = json.loads(reply)
    if isinstance(found, dict) and found.get("error") is not None:
        raise refusal(found["error"])
    for warning in (found or {}).get("warnings", ()):
        call.warn(warning)
    return found

