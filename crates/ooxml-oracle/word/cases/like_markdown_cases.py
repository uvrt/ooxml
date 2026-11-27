"""The like-by-Markdown probe document (``probes/like_markdown_probe.py``; Word's observations in
``observations/like-markdown-word.json``): a numbered list and a bullet list the tools wrote,
each paragraph named by its first word, to which new items are added -- by the tools'
``word_insert_markdown`` ``like`` and as a person adds them in Word (Enter at the end of the
list's last item, the new text typed; Word's list indent for an item a level down), tracked.

Must not: import pytest or docx-agent's library.
"""

from __future__ import annotations

#: The tracked edits' author, on the tools' side (Word's is its user, normalised away).
AUTHOR = "Probe Writer"

#: The document before the edits, as the tools write it from Markdown into a new document: the
#: Markdown and its style map, in order. The first bullet list is in Word's built-in List Bullet
#: (a one-level list); the second in List Paragraph, which numbers nothing, so its items are a
#: list of Word's own nine-level bullet definition; then a second list in Word's built-in List
#: Number and one in List Bullet 3 (one-level lists too, the last indented further).
BASE = [("Head stays.\n\n1. Walk first.\n2. Ford second.\n\n- Alpha point.\n- Bravo point.\n", None),
        ("- Echo point.\n- Foxtrot point.\n", [{"element": "bullet", "style": "List Paragraph"}]),
        ("Between stays.\n\n1. Mango first.\n2. Nectar second.\n", None),
        ("- Sierra point.\n- Tango point.\n", [{"element": "bullet", "style": "List Bullet 3"}]),
        ("After stays.\n", None)]

#: The cases: the sample's key, the key of the item the new ones follow, the Markdown the tools
#: write (like the sample) and the new items' keys at their levels (1 the top), as typed in Word.
CASES = {
    "count-on": {"sample": "Walk", "after": "Ford", "markdown": "1. Cross third.\n2. Rest fourth.\n",
                 "new": [("Cross third.", 1), ("Rest fourth.", 1)]},
    "level-deeper": {"sample": "Echo", "after": "Foxtrot", "markdown": "- Golf point.\n  - Hotel below.\n",
                     "new": [("Golf point.", 1), ("Hotel below.", 2)]},
    # one-level lists: Word keeps the style and indents the deeper items (F-2c)
    "one-level": {"sample": "Alpha", "after": "Bravo",
                  "markdown": "- Charlie point.\n  - Detail below.\n    - Lower still.\n",
                  "new": [("Charlie point.", 1), ("Detail below.", 2), ("Lower still.", 3)]},
    "one-level-number": {"sample": "Nectar", "after": "Nectar",
                         "markdown": "1. Olive third.\n   1. Pear below.\n      1. Quince lower.\n",
                         "new": [("Olive third.", 1), ("Pear below.", 2), ("Quince lower.", 3)]},
    "one-level-indented": {"sample": "Sierra", "after": "Tango",
                           "markdown": ("- Union point.\n  - Victor below.\n    - Whiskey lower.\n"
                                        "      - Xray lowest.\n"),
                           "new": [("Union point.", 1), ("Victor below.", 2), ("Whiskey lower.", 3),
                                   ("Xray lowest.", 4)]},
}

#: The paragraphs before the edits (Reject All gives them back).
REJECTED = ["Head", "Walk", "Ford", "Alpha", "Bravo", "Echo", "Foxtrot", "Between", "Mango", "Nectar", "Sierra",
            "Tango", "After"]

#: Word's list indent in a one-level list, each case a document of its own (``word-indents``):
#: a list -- built-in List Bullet unless named -- between ``Head stays.`` and ``After stays.``,
#: with what stands around it. ``new``: Enter at the end of ``after``, then INDENT_NEW typed,
#: each item list-indented to its level, as the tools write the same items ``like`` the
#: ``sample``; ``existing``: the item ``after`` list-indented twice where it stands (Word's
#: alone: no tool moves an existing item so). ``pieces`` the Markdown and style map in
#: order, ``indents`` a paragraph's own ``w:ind`` (left, hanging) by key, ``tab`` the
#: settings' ``w:defaultTabStop`` (new_document's 708 where none).
_BULLETS = ("- Alpha point.\n- Bravo point.\n", None)
_BULLET3 = [{"element": "bullet", "style": "List Bullet 3"}]
_SIERRA = ("- Sierra point.\n", _BULLET3)
_ONE_LEVEL = {
    "bullet": {"pieces": [_BULLETS], "sample": "Alpha", "after": "Bravo"},
    "number": {"pieces": [("1. Mango first.\n2. Nectar second.\n", None)], "sample": "Mango", "after": "Nectar",
               "numbered": True},
    "bullet3": {"pieces": [("- Sierra point.\n- Tango point.\n", _BULLET3)], "sample": "Sierra", "after": "Tango"},
}
INDENTS = {
    **{f"alone-{name}": {**case, "mode": "new"} for name, case in _ONE_LEVEL.items()},
    **{f"tab567-{name}": {**_ONE_LEVEL[name], "mode": "new", "tab": 567} for name in ("bullet", "bullet3")},
    # what follows the new items
    "next-list": {"pieces": [_BULLETS, _SIERRA], "sample": "Alpha", "after": "Bravo", "mode": "new"},
    "next-gap": {"pieces": [_BULLETS, ("Gap stays.\n", None), _SIERRA], "sample": "Alpha", "after": "Bravo",
                 "mode": "new"},
    "next-plain": {"pieces": [_BULLETS, ("Plain stays.\n", None)], "indents": {"Plain": ("926", "360")},
                   "sample": "Alpha", "after": "Bravo", "mode": "new"},
    "next-hanging": {"pieces": [_BULLETS, _SIERRA], "indents": {"Sierra": ("1000", "100")}, "sample": "Alpha",
                     "after": "Bravo", "mode": "new"},
    "before-list": {"pieces": [_SIERRA, _BULLETS], "sample": "Alpha", "after": "Bravo", "mode": "new"},
    # an existing item moved where it stands
    "existing-alone": {"pieces": [_BULLETS], "after": "Alpha", "mode": "existing"},
    "existing-after-list": {"pieces": [_SIERRA, _BULLETS], "after": "Alpha", "mode": "existing"},
    "existing-before-list": {"pieces": [("- Alpha point.\n", None), _SIERRA], "after": "Alpha", "mode": "existing"},
}
#: The items typed after the list (key, level) in the ``new`` cases.
INDENT_NEW = [("Kilo one.", 1), ("Lima two.", 2), ("Mike three.", 3), ("Nova four.", 4), ("Oscar five.", 5)]
#: How often the ``existing`` cases' item is list-indented.
INDENT_EXISTING = 2
