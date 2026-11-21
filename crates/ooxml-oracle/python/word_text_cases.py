"""The text tools' cases for ``word_edits_diff.py --text`` (lane P2c): ``word_set_text``,
``replace_text``, ``word_insert_text``, ``word_delete`` and ``word_insert_markdown`` called
as tools through docx-agent's toolbox on a document as opened, for the harness to compare
with the Rust core's (``word-edits``, ``op: "tool"``).

Python's behaviour is not changed: this reads docx-agent's documents only.
"""

from __future__ import annotations


def cases(document, targets: list[str]) -> list[dict]:
    """Every text tool's cases for ``document``, its paragraphs ``targets``."""
    return _text_tool_cases(document, targets) + _insert_delete_cases(document, targets) \
        + _insert_markdown_cases(document, targets)


def _call(tool, **arguments):
    return {"op": "tool", "tool": tool, "arguments": {"doc": "d1", **arguments}}


def _text_tool_cases(document, targets: list[str]) -> list[dict]:
    """``word_set_text`` and ``replace_text`` called as tools (``--text``): a paragraph's or
    cell's text, in lines, several items in one call, and the refusals (no target, both,
    no text, a span, an id the document lacks); a word replaced where it occurs once and
    everywhere, in the body and in every story, and a text that occurs nowhere. Calls that
    need what is not ported (styled text, spacing) are answered by docx-agent on both sides."""
    call = _call
    out = [call("word_set_text", text="x"), call("word_set_text", target=targets[0] if targets else "p:00000001",
                                                  items=[]),
           call("word_set_text", items=[]), call("word_set_text", target="p:7FFFFFFE", text="Nowhere"),
           call("replace_text", find="zz-nowhere-zz", replace="x"),
           call("word_set_text", target="p:00000001..p:00000002", text="Span")]
    for target in targets:
        text = document.paragraph(target).text
        out += [call("word_set_text", target=target, text="A new sentence, written whole."),
                call("word_set_text", target=target, text=text),
                call("word_set_text", target=target, text="First line\nSecond line"),
                call("word_set_text", target=target, text=""),
                call("word_set_text", target=target, text="Some **strong** words"),
                call("word_set_text", target=target)]
        words = [w for w in text.split() if w.isalpha() and len(w) > 3]
        for word in words[:2]:
            out += [call("replace_text", find=word, replace=word.upper()),
                    call("replace_text", find=word, replace="new words", expect="all"),
                    call("replace_text", find=word, replace="x", expect="all", stories="all")]
    if len(targets) > 1:
        out.append(call("word_set_text", items=[{"target": targets[0], "text": "One"},
                                                {"target": targets[-1], "text": "Two\nThree"}]))
        out.append(call("word_set_text", items=[{"target": targets[0], "text": "One"},
                                                {"target": "p:7FFFFFFE", "text": "Two"}]))
    tables = document._index(document.package.document_part()).tables
    if tables:
        out += [call("word_set_text", target=f"{tables[0].id}/c0,0", text="Cell text"),
                call("word_set_text", target=tables[0].id, text="A table"),
                call("word_set_text", target=f"{tables[0].id}/c0,0", text="Two\nlines **here**"),
                call("word_delete", target=tables[0].id)]
    return out


def _insert_delete_cases(document, targets: list[str]) -> list[dict]:
    """``word_insert_text`` and ``word_delete`` called as tools (``--text``): text beside a
    paragraph, a range, a position and a word found once, several items, refs; text, a range,
    a block and a span deleted, the space a deletion leaves doubled, a heading whose content
    stays; and the refusals (no place, both forms, an address as ref, an at that is no side)."""
    call = _call
    out = [call("word_insert_text", text="x"), call("word_insert_text", text="x", items=[{"text": "y"}]),
           call("word_insert_text", items=[]), call("word_insert_text", text="x", ref="p:00000001@0"),
           call("word_insert_text", text="x", at="end"), call("word_insert_text", find="zz-nowhere-zz", text="x"),
           call("word_delete"), call("word_delete", find="zz-nowhere-zz"), call("word_delete", target="p:7FFFFFFE")]
    for target in targets:
        text = document.paragraph(target).text
        out += [call("word_insert_text", target=target, text=" added"),
                call("word_insert_text", target=target, where="before", text="Start: ", ref="first"),
                call("word_insert_text", at=f"{target}@0", text="«New» "),
                call("word_insert_text", at=f"before:{target}", where="after", text="x"),
                call("word_delete", target=target),
                call("word_delete", target=target, force=True)]
        if len(text) > 4:
            out += [call("word_insert_text", at=f"after:{target}@1:3", text="--"),
                    call("word_delete", target=f"{target}@1:3"),
                    call("word_delete", target=f"{target}@0:2", collapse_space=True)]
        words = [w for w in text.split() if w.isalpha() and len(w) > 3]
        for word in words[:2]:
            out += [call("word_insert_text", find=word, text=" (more)"),
                    call("word_insert_text", find=word, where="before", text="very "),
                    call("word_delete", find=word), call("word_delete", find=word, collapse_space=True),
                    call("word_delete", find=word, target=target)]
    if len(targets) > 1:
        out += [call("word_insert_text", items=[{"target": targets[0], "text": "One "},
                                                 {"at": f"{targets[-1]}@0", "text": "Two ", "ref": "two"}]),
                call("word_insert_text", items=[{"target": targets[0], "text": "One"}, {"text": "Two"}]),
                call("word_delete", target=f"{targets[0]}..{targets[1]}"),
                call("word_delete", target=f"{targets[0]}..{targets[-1]}", force=True)]
    return out


#: insert_markdown's Markdown called as the tool (raw HTML refused, emphasis in its styles).
_MARKDOWN = [
    ("# Title\n\nSome *em*, **strong**, `code`, [a link](https://example.com/a).\n\n- one\n- two\n  - nested\n\n"
     "1. first\n2. second\n"),
    "> quoted\n>\n> - in a quote\n\n```\ncode line\n```\n\n---\n\n: A caption\n",
    "| A | B |\n|:-:|--:|\n| 1 | 2 |\n\nTable: numbers\n",
    "Plain line\n",
]


def _insert_markdown_cases(document, targets: list[str]) -> list[dict]:
    """``word_insert_markdown`` called as a tool (``--text``): Markdown at the end, after,
    before and at a paragraph, refs naming one block and several, a style map; the refusals
    (no content, two contents, an address as ref, ``markdown_from``, raw HTML, a place or
    style the document lacks); and what the host answers (from, blob, like, a picture, a
    footnote, ``replace:``)."""
    call = _call
    out = [call("word_insert_markdown", at="end"),
           call("word_insert_markdown", at="end", markdown="x", blob="b1"),
           call("word_insert_markdown", at="end", markdown="x", ref="p:00000001"),
           call("word_insert_markdown", at="end", markdown_from="d:1"),
           call("word_insert_markdown", at="end", markdown="<div>x</div>\n"),
           call("word_insert_markdown", at="after:p:7FFFFFFE", markdown="x"),
           call("word_insert_markdown", at="end:nowhere", markdown="x"),
           call("word_insert_markdown", at="end", markdown=""),
           call("word_insert_markdown", at="end", markdown="<!-- only a comment -->\n"),
           call("word_insert_markdown", at="end", markdown="# H\n", style_map=[{"element": "h1", "style": "Nope"}]),
           call("word_insert_markdown", at="end", markdown="# Head\n\nbody\n",
                style_map=[{"element": "h1", "style": "Title"}, {"element": "paragraph", "style": "Body Text"}],
                ref="made"),
           call("word_insert_markdown", at="end", markdown="One\n", ref="one"),
           call("word_insert_markdown", at="end", **{"from": "d:1"}),
           call("word_insert_markdown", at="end", blob="b1"),
           call("word_insert_markdown", at="end", markdown="![a](b1)\n"),
           call("word_insert_markdown", at="end", markdown="Text[^1]\n\n[^1]: note\n")]
    out += _markdown_blob_cases(targets)
    for markdown in _MARKDOWN:
        out.append(call("word_insert_markdown", at="end", markdown=markdown))
    for target in targets[:2]:
        out += [call("word_insert_markdown", at=f"after:{target}", markdown=_MARKDOWN[0]),
                call("word_insert_markdown", at=f"before:{target}", markdown=_MARKDOWN[2], ref="t"),
                call("word_insert_markdown", at=target, markdown=_MARKDOWN[1]),
                call("word_insert_markdown", at=f"after:{target}", markdown="- a\n- b\n", like=[target]),
                call("word_insert_markdown", at=f"after:{target}", markdown="- a\n",
                     style_map=[{"element": "bullet", "style": target}]),
                call("word_insert_markdown", at=f"replace:{target}", markdown="New text\n")]
    tables = document._index(document.package.document_part()).tables
    if tables:
        out.append(call("word_insert_markdown", at=f"after:{tables[0].id}", markdown=_MARKDOWN[2]))
    return out


def _markdown_blob_cases(targets: list[str]) -> list[dict]:
    """``word_insert_markdown`` with the session's blobs: the Markdown a blob holds (and one
    that is not UTF-8), pictures by a blob's handle and by its name with a title, in a list
    and a table, one wider than the text (scaled to it), from a ``data:`` URI, a remote
    one, a blob that is no image, one the session lacks and bytes that are no picture, a
    picture and a place the document lacks."""
    import base64

    from word_object_cases import _PICTURES, _blob, _png

    png, wide = _PICTURES["png"], _blob(_png(1000, 10), "wide.png", "image/png")
    text = _blob("# From a blob\n\nIts *text* -- caf\u00e9.\n".encode(), "notes.md", "text/markdown")

    def with_blobs(blobs, **arguments) -> dict:
        return {**_call("word_insert_markdown", **arguments), "blobs": blobs}

    data = "data:image/png;base64," + base64.b64encode(bytes.fromhex(png["hex"])).decode()
    out = [with_blobs([text], at="end", blob="b1"),
           with_blobs([_blob(b"\xff\xfe bad", "bad.bin", "application/octet-stream")], at="end", blob="b1"),
           with_blobs([text], at="end", blob="b2"),
           with_blobs([png], at="end", markdown="A picture: ![a picture](b1)\n"),
           with_blobs([png], at="end", markdown='![named](a.png "A title")\n\n- ![](b1) in a list\n'),
           with_blobs([png], at="end", markdown="| A |\n|---|\n| ![cell](b1) |\n"),
           with_blobs([wide], at="end", markdown="![wide](b1)\n"),
           with_blobs([png], at="end", markdown=f"![inline]({data})\n"),
           with_blobs([png], at="end", markdown="![x](data:;base64,a)\n"),
           with_blobs([png], at="end", markdown="![far](https://example.com/a.png)\n"),
           with_blobs([text], at="end", markdown="![text](b1)\n"),
           with_blobs([png], at="end", markdown="![none](b7)\n"),
           with_blobs([_PICTURES["text"]], at="end", markdown="![g](g.txt)\n"),
           with_blobs([_PICTURES["headless"]], at="end", markdown="![h](b1)\n"),
           with_blobs([png], at="after:p:7FFFFFFE", markdown="![none](b7)\n"),
           with_blobs([png], at="end", markdown="[![linked](b1)](https://example.com/)\n")]
    for target in targets[:1]:
        out.append(with_blobs([png, wide], at=f"after:{target}", markdown="![one](b1) and ![two](b2)\n"))
    return out
