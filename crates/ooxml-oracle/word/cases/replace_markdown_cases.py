"""The replace-by-Markdown probe document (``probes/replace_markdown_probe.py``; Word's
observations in ``observations/replace-markdown-word.json``): four paragraphs named by their
first word, the two in the middle with settings of their own, replaced by two new ones -- as the
tools' ``word_insert_markdown`` at ``replace:`` makes it and as a person makes it in Word (the two
paragraphs selected whole, the new text typed over them), tracked.

Must not: import pytest or docx-agent's library.
"""

from __future__ import annotations

from probe_package import package as _package

#: The tracked edits' author, on the tools' side (Word's is its user, normalised away).
AUTHOR = "Probe Writer"

#: The paragraphs: (key, text, its own w:pPr children).
PARAGRAPHS = [
    ("Head", "Head stays.", ""),
    ("First", "First body.", '<w:keepNext/><w:pageBreakBefore/><w:ind w:left="720"/>'),
    ("Second", "Second body.", '<w:keepLines/><w:jc w:val="center"/>'),
    ("After", "After stays.", ""),
]
#: The replaced paragraphs' keys, and the Markdown (the text typed) that replaces them.
REPLACED = ("First", "Second")
NEW = ("Newone first.", "Newtwo second.")
MARKDOWN = "\n\n".join(NEW)

#: Each key's text once accepted, and once rejected.
ACCEPTED = {"Head": "Head stays.", "Newone": "Newone first.", "Newtwo": "Newtwo second.", "After": "After stays."}
REJECTED = {key: text for key, text, _ in PARAGRAPHS}
#: The settings each new paragraph keeps, accepted: the replaced one's at its position.
KEPT = {"Newone": ["keepNext", "pageBreakBefore", "ind"], "Newtwo": ["keepLines", "jc"]}


def _paragraph(k: int, text: str, properties: str) -> str:
    props = f"<w:pPr>{properties}</w:pPr>" if properties else ""
    return (f'<w:p w14:paraId="{0x51000000 + k:08X}" w14:textId="77777777">{props}'
            f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>')


def document() -> bytes:
    return _package("".join(_paragraph(k + 1, text, props) for k, (_, text, props) in enumerate(PARAGRAPHS)))
