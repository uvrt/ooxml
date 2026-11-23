"""Measure again what Phase 1's recorded-Word tier left open (e1, e3).

* ``pictures``: Word's size for a picture that states no density, beside ones that state
  72, 96 and 144 dpi (e1's case measured 144 dpi where docx-agent's E1 notes said 96), each
  inserted as e1 inserts them (``make new inline picture``).
* ``bold-command``: e3's ``Bold`` case (``Bold rho sigma.``, characters 5-8, tracking on)
  made with Word's own Bold command -- the command its Cmd+B key runs (``execute key
  binding``), not AppleScript's ``bold`` property, which e3 used; with the command's name.
* ``tables-reject``: e3's table probe tracked again and Reject All of it (e3 recorded that
  the rejected column insert keeps its empty cells; docx-agent's ROADMAP said they go).
* ``colins-tools-reject``: Word's Reject All of the tools' own tracked column insert (the
  ``ColIns`` table, ``word_edit_table insert_column`` with tracking on).
* ``cells-reject``: Reject All of e3's recorded cell revisions again (``InsertLast``: Word
  took the cell's first paragraph out of the table); ``insertlast-reject-one``: only
  ``InsertLast``'s paragraph insertion rejected, as one revision.
* ``join-content``: e3's untracked ``JoinA`` deletion (``kappa.``, the mark, ``JoinB ``)
  made by setting the range's content to nothing rather than by typing a backspace over
  it (e3's ``join-untracked`` kept the mark that way).

    python crates/ooxml-oracle/word/probes/followups_probe.py   # writes observations/followups-word.json

Nothing Word writes is committed: only the facts read from it (e3's normalising).
"""

from __future__ import annotations

import json
import re
import struct
import sys
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_paths  # noqa: E402  (the driver, the fixture makers and the cases on the path)

import e3_probe  # noqa: E402
import oracle  # noqa: E402
import probe_tools  # noqa: E402

OBSERVATIONS = probe_paths.OBSERVATIONS / "followups-word.json"
#: The pictures: (case, density stated in dpi or None).  96 x 48 px each.
PICTURES = [("no density", None), ("72 dpi", 72), ("96 dpi", 96), ("144 dpi", 144)]
#: What the tools do with the no-density case (main's decision, 2026-11-23).
PICTURES_VERDICT = ("deliberate, Windows convention, unmeasured on Windows: the tools place a picture "
                    "stating no density at 96 dpi (Windows Word's convention, the clients' Word); this Mac "
                    "Word gives 144 dpi, seemingly its screen's Retina backing scale")


def png(dpi: int | None, width: int = 96, height: int = 48) -> bytes:
    """A one-colour PNG, with a ``pHYs`` density only where ``dpi`` is given."""

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + bytes((200, 30, 30)) * width for _ in range(height))
    out = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    if dpi:
        per_metre = int(round(dpi / 0.0254))
        out += chunk(b"pHYs", struct.pack(">IIB", per_metre, per_metre, 1))
    return out + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def pictures_document() -> bytes:
    """e3's text probe (any document will do): the pictures go at its end."""
    return e3_probe.text_document()


def pictures_script() -> str:
    lines = []
    for k in range(len(PICTURES)):
        lines += ["set e to (end of content of text object of d) - 1",
                  f"make new inline picture at (create range d start e end e) with properties "
                  f"{{file name:(item {k + 3} of argv), link to file:false, save with document:true}}",
                  "set e to (end of content of text object of d) - 1",
                  "insert paragraph at (create range d start e end e)"]
    return e3_probe._script(lines)


def bold_script() -> str:
    """e3's Bold case through Word's Bold command; the command's name written at the end."""
    key = "(find key key code (build key code key1 command_key key2 b_key))"
    return e3_probe._script([
        "set track revisions of d to true",
        e3_probe._locate("Bold"),
        e3_probe._select(5, 8),
        f"execute key binding {key}",
        f'insert text ("[command: " & (command of {key}) & "]") at end of text object of d',
    ])


def tools_column_insert() -> bytes:
    """e3's table probe with the tools' tracked column insert in ``ColIns``."""
    session = probe_tools.Session(e3_probe.table_document())
    session.call("word_set_tracking", on=True, author="Writer")
    table = next(block for block, text in session.blocks() if re.match(r"\W*ColIns\b", text))
    session.call("word_edit_table", table=table, action="insert_column", index=1, side="after")
    return session.data()


def reject_one_script(key: str) -> str:
    """Reject only the revision in the paragraph after the one whose text starts with
    ``key`` (``InsertLast cell.``, then its inserted ``InsertLast new.``)."""
    return e3_probe._script([
        e3_probe._locate(key),
        "reject (revision 1 of (text object of paragraph (k + 1) of d))",
    ])


def join_content_script() -> str:
    """e3's untracked JoinA deletion, the range's content set to nothing."""
    return e3_probe._script([
        e3_probe._locate("JoinA"),
        "set e to end of content of text object of paragraph k of d",
        'set content of (create range d start (e - 7) end (e + 6)) to ""',
    ])


def entry(outcome: oracle.Outcome) -> dict:
    if not outcome:
        return {"failed": f"{outcome.outcome}: {outcome.detail}"}
    data = outcome.path.read_bytes()
    return {"blocks": e3_probe.blocks(data)}


def main() -> None:
    oracle.ORACLE_DIR.mkdir(parents=True, exist_ok=True)
    files = [oracle.ORACLE_DIR / f"followups-{k}.png" for k in range(len(PICTURES))]
    out: dict = {}
    with oracle.session() as session:
        for path, (_, dpi) in zip(files, PICTURES):
            path.write_bytes(png(dpi))
        try:
            pictures = session.run_script(pictures_script(), pictures_document(), name="followups-pictures",
                                          tag="pictures", args=[str(p) for p in files], timeout=200)
        finally:
            for path in files:
                path.unlink(missing_ok=True)
        out["pictures"] = {**entry(pictures), "verdict": PICTURES_VERDICT}
        out["bold-command"] = entry(session.run_script(bold_script(), e3_probe.text_document(),
                                                       name="followups-bold", tag="command", timeout=200))
        tracked = session.run_script(e3_probe.table_script(), e3_probe.table_document(), name="followups-tables",
                                     tag="tracked", timeout=200)
        out["tables-reject"] = entry(session.run_script(e3_probe.review_script("reject"), tracked.path.read_bytes(),
                                                        name="followups-tables", tag="reject", timeout=200)) \
            if tracked else entry(tracked)
        out["colins-tools-reject"] = entry(session.run_script(e3_probe.review_script("reject"), tools_column_insert(),
                                                              name="followups-colins", tag="reject", timeout=200))
        out["cells-reject"] = entry(session.run_script(e3_probe.review_script("reject"), e3_probe.cells_document(),
                                                       name="followups-cells", tag="reject", timeout=200))
        out["insertlast-reject-one"] = entry(session.run_script(reject_one_script("InsertLast"),
                                                                e3_probe.cells_document(), name="followups-cells",
                                                                tag="reject-one", timeout=200))
        out["join-content"] = entry(session.run_script(join_content_script(), e3_probe.join_document(),
                                                       name="followups-join", tag="content", timeout=200))
    OBSERVATIONS.write_text(json.dumps({
        "_about": ("What Word 16.106 for Mac wrote for crates/ooxml-oracle/word/probes/followups_probe.py: the e1 and "
                   "e3 cases Phase 1 left open, measured again (pictures by density, the Bold command tracked, "
                   "Reject All of inserted columns and of an inserted last cell paragraph). Body blocks, normalised "
                   "as e3's. Regenerate with python crates/ooxml-oracle/word/probes/followups_probe.py."),
        "word": "16.106 (macOS), Dutch interface",
        **out}, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {OBSERVATIONS}")


if __name__ == "__main__":
    main()
