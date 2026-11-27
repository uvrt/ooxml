"""What a Word document keeps of the values placed in it, as the Rust session reads its
records (P4e: no Python view; a session on the Rust host's, or the Python host's mirror's until E2): the tables' and charts' sources (``provenance``, also the
session's ``Session.provenance``), the values written into paragraphs by reference
(``facts``) and the labels written in place of values (``labels``), each by paragraph id.

Must not: import pytest or docx-agent's document library.
"""

from __future__ import annotations

import json
from typing import Any


def records(session: Any, doc: str) -> dict[str, Any]:
    """The session's reading of the document's records at the version the calls read now:
    ``{"provenance": [[key, record]...], "facts": [[paragraph, facts]...], "labels": [...]}``."""
    from ooxml_edit.tools.hosted import HostedSession

    if isinstance(session, HostedSession):
        found = session.rust.version(doc).records()
    else:
        from docx_agent.rust import hooks

        found = hooks.version_of(session.entry(doc).subject).records()
    out = json.loads(found) if found is not None else {}
    return {"provenance": out.get("provenance", []), "facts": out.get("facts", []),
            "labels": out.get("labels", []), **{k: v for k, v in out.items()
                                                if k not in ("provenance", "facts", "labels")}}


def text_facts(session: Any, doc: str) -> dict[str, list[dict[str, Any]]]:
    """The values written into paragraphs by reference that are still their text:
    ``{paragraph id: facts}`` (once ``Document.text_facts``)."""
    return {paragraph: facts for paragraph, facts in records(session, doc)["facts"]}


def text_labels(session: Any, doc: str) -> dict[str, list[dict[str, Any]]]:
    """The labels written in place of values still in their paragraphs: ``{paragraph id:
    labels}`` (once ``Document.text_labels``)."""
    return {paragraph: labels for paragraph, labels in records(session, doc)["labels"]}
