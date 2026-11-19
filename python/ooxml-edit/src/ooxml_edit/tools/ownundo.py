"""One caller's undo or redo of its own changes to a document (:meth:`.session.Session.undo`
with ``caller``): which step is the caller's, whether it is the latest, and the refusal's words
when it is not.  Moved out of :mod:`.session` (lane P2c4).

Must not: undo another caller's change, or say a refusal without the later change and who
made it.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, Mapping, Sequence

from .results import ToolError

if TYPE_CHECKING:
    from .session import DocumentEntry, DocumentFormat, Session


@dataclass(frozen=True)
class OwnUndo:
    """An undo a caller made of its own change, for its redo: ``plain`` (the document's
    latest change, on the history's redo stack) or ``taken`` (taken out from under later
    changes, :meth:`~ooxml_edit.history.History.redo_step`); ``after`` is the version the
    change made, ``parts`` what it touched (``None`` when unknown)."""

    caller: Mapping[str, Any]
    kind: str
    after: int
    parts: frozenset[str] | None


class OwnUndoing:
    """One caller's undo or redo of its own changes to one document, its lock held
    (:meth:`Session.undo` with ``caller``)."""

    def __init__(self, session: Session, entry: DocumentEntry, fmt: DocumentFormat | None,
                 history: Any, caller: dict[str, Any], scope: frozenset[str] | None,
                 doc_id: str) -> None:
        self.session = session
        self.entry = entry
        self.fmt = fmt
        self.history = history
        self.caller = caller
        self.scope = scope
        self.doc_id = doc_id
        #: Whether parts were put back behind the document's back (``fmt.restored``).
        self.restored = False

    def _mine(self, version: int) -> bool:
        owner = self.entry.owners.get(version)
        return owner is not None and dict(owner.caller) == self.caller

    def _in_scope(self, parts: frozenset[str] | None) -> bool:
        return self.scope is None or bool(parts and parts & self.scope)

    def _who(self, version: int) -> str:
        owner = self.entry.owners.get(version)
        if owner is None:
            return "a change made outside the tools"
        if dict(owner.caller) == self.caller:
            return f"your own {owner.tool}"
        return owner.describe()

    def _later(self, versions: Sequence[int]) -> list[dict[str, Any]]:
        out = []
        for version in versions:
            owner = self.entry.owners.get(version)
            out.append({"version": version, **(owner.to_json() if owner else {})})
        return out

    def undo(self) -> bool:
        """Undo the caller's latest change; ``False`` when it has none left to undo."""
        from ..history import Entangled

        entry, history = self.entry, self.history
        steps = history.steps()
        mine = [k for k, step in enumerate(steps)
                if self._mine(step.after) and self._in_scope(step.parts)]
        if not mine:
            return False
        index = mine[-1]
        step = steps[index]
        later = steps[index + 1:]
        if not later:
            # The document's latest change: a plain undo, exact, as the session's.
            target = entry.step_target("undo")
            target()
            entry.restore_refs()
            entry.own_undos.append(OwnUndo(self.caller, "plain", step.after, step.parts))
            return True
        owner = entry.owners[step.after]
        mine_words = f"your latest change to {self.doc_id} ({owner.tool}, version {step.after})"
        last = later[-1].after
        if (self.fmt is None or self.fmt.undo_scope is None or step.parts is None
                or not hasattr(history, "undo_step")):
            # One body every change shares: a later change may build on any of it.
            raise ToolError(
                "entangled", f"{mine_words} is not the latest: {self._who(last)} changed "
                f"{self.doc_id} after it (version {last}"
                + (f", one of {len(later)} later changes" if len(later) > 1 else "")
                + "), and undoing yours would take back what came after. Change it back "
                "with an edit, or ask whoever made the later change to undo it first",
                details={"version": step.after, "later": self._later(
                    [later_step.after for later_step in later])})
        shared = (list(self.fmt.undo_shared(entry.document)) if self.fmt.undo_shared
                  else [])
        try:
            done = history.undo_step(step.after, shared=shared)
        except Entangled as exc:
            if not exc.later:
                raise ToolError(
                    "entangled", f"{mine_words} also changed {', '.join(exc.shared)}, which "
                    "other pages show too, and others changed the document since: it is no "
                    "one caller's to take back. Change it back with an edit",
                    details={"version": step.after, "parts": exc.parts,
                             "shared": exc.shared}) from None
            first = exc.later[0]
            raise ToolError(
                "entangled", f"{mine_words} is not the latest: {self._who(first)} (version "
                f"{first}) changed {', '.join(exc.shared)} after it, which yours changed "
                "too, so undoing yours would revert that as well. Change it back with an "
                "edit, or ask whoever made the later change to undo it first",
                details={"version": step.after, "parts": exc.parts, "shared": exc.shared,
                         "later": self._later(exc.later)}) from None
        self.restored = True
        entry.scoped_refs(done, undone=True)
        entry.renumber_owners(done, undone=True)
        entry.own_undos.append(OwnUndo(self.caller, "taken", step.after, step.parts))
        return True

    def redo(self) -> bool:
        """Redo the caller's latest undo of its own; ``False`` when there is none."""
        from ..history import Entangled

        entry, history = self.entry, self.history
        while True:
            found = [k for k, undo in enumerate(entry.own_undos)
                     if dict(undo.caller) == self.caller and self._in_scope(undo.parts)]
            if not found:
                return False
            index = found[-1]
            undo = entry.own_undos[index]
            if undo.kind == "plain":
                waiting = getattr(history, "next_redo", None)
                if waiting is None:     # the redo stack went: nothing of it comes back
                    del entry.own_undos[index]
                    continue
                if waiting != undo.after:
                    raise ToolError(
                        "entangled", f"your undone change to {self.doc_id} (version "
                        f"{undo.after}) is not the next to redo: another caller undid a "
                        "change since. Make your change again instead")
                target = entry.step_target("redo")
                target()
                entry.restore_refs()
                del entry.own_undos[index]
                return True
            try:
                step = history.redo_step(undo.after)
            except Entangled as exc:
                raise ToolError(
                    "entangled", f"your undone change to {self.doc_id} (version "
                    f"{undo.after}) cannot come back: {', '.join(exc.shared)} changed since "
                    "the undo. Make your change again instead",
                    details={"version": undo.after, "parts": exc.parts,
                             "shared": exc.shared}) from None
            del entry.own_undos[index]
            if step is None:        # out of the history's depth: gone; the one before?
                continue
            self.restored = True
            entry.scoped_refs(step, undone=False)
            entry.renumber_owners(step, undone=False)
            return True

