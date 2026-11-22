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
- :func:`closed` and :func:`restored` -- nothing of the session's: the session lets go of the
  document as the owner closes it, and what a batch or scoped undo puts back is its own
  version; only what docx-agent kept in this process for a view made of it goes.
"""

from __future__ import annotations

import json
from typing import Any

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


def live_view(subject: Any) -> Any:
    """The Python view docx-agent made of an owned document in this process, as it is (not
    made current), or ``None``."""
    live = getattr(_mirror(subject), "live", None)
    return live() if live is not None else None
