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
#: list of Word's own nine-level bullet definition.
BASE = [("Head stays.\n\n1. Walk first.\n2. Ford second.\n\n- Alpha point.\n- Bravo point.\n", None),
        ("- Echo point.\n- Foxtrot point.\n", [{"element": "bullet", "style": "List Paragraph"}]),
        ("After stays.\n", None)]

#: The cases: the sample's key, the key of the item the new ones follow, the Markdown the tools
#: write (like the sample) and the new items' keys at their levels (1 the top), as typed in Word.
CASES = {
    "count-on": {"sample": "Walk", "after": "Ford", "markdown": "1. Cross third.\n2. Rest fourth.\n",
                 "new": [("Cross third.", 1), ("Rest fourth.", 1)]},
    "level-deeper": {"sample": "Echo", "after": "Foxtrot", "markdown": "- Golf point.\n  - Hotel below.\n",
                     "new": [("Golf point.", 1), ("Hotel below.", 2)]},
    # one level only: 0.1's rule writes the deeper item without like (and says so)
    "one-level": {"sample": "Alpha", "after": "Bravo", "markdown": "- Charlie point.\n  - Detail below.\n",
                  "new": [("Charlie point.", 1), ("Detail below.", 2)]},
}

#: The paragraphs before the edits (Reject All gives them back).
REJECTED = ["Head", "Walk", "Ford", "Alpha", "Bravo", "Echo", "Foxtrot", "After"]
