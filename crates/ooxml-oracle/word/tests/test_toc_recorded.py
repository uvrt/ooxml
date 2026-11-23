"""Which paragraphs a table of contents lists, and at which level, held to Word (production
round 21, T1): ``probes/toc_probe.py``'s made-up documents, each with a ``TOC`` field of
``\\o``, ``\\t`` and ``\\u`` alone and combined, updated through the tools and compared with the
entries Word wrote for the same document (``observations/toc-word.json``)."""

from __future__ import annotations

import pytest
from ooxml_common.locales import Locale
from recorded import Opened, observed  # noqa: I001  (puts the probes on the path)

import toc_probe  # noqa: E402

OBSERVED = observed("toc-word.json")
#: Every case is read (index.py: none left out).
READS = set(OBSERVED)
#: The Dutch Mac's regional settings, which the measuring Word ran under.
DUTCH = Locale(decimal_separator=",", thousands_separator=".", date_order="dmy", language="nl-NL",
               list_separator=";")
#: Dutch Word matches ``\t``'s names against its own display names: a built-in heading is "Kop 2"
#: there, "Heading 2" in English Word, and Dutch Word listed nothing for "Heading 2".  The Word
#: that wrote a field cannot be known from the file, so the tools take a name in the document's
#: language *and* in English: in these two cases they list what English Word lists for
#: "Heading 2" (``t-heading-en``), where Dutch Word listed nothing (feedback-round21,
#: measurements-D).
ENGLISH_TOO = {"t-heading-en-name-nl": "t-heading-en"}


def updated(box, case: str) -> list[list]:
    """The entries the tools' update writes for ``case``'s document."""
    word = OBSERVED[case]
    locale = {"locale": DUTCH} if word["locale"] == "nl-NL" else {}
    document = Opened(box, toc_probe.make(word["body"], word["instruction"]), **locale)
    document.call("word_fields", action="update")
    return toc_probe.entries(document.data())


@pytest.mark.parametrize("case", sorted(OBSERVED))
def test_the_entries_are_words(box, case):
    expected = OBSERVED[ENGLISH_TOO.get(case, case)]["entries"]
    if case == "t-overrides-style-level":
        # The Dutch document's "Heading 2" is English Word's name for Kop 2, taken too: Word's
        # own entries here are the Leveled ones (level 5), ours those and Kop 2's.
        assert [entry for entry in updated(box, case) if entry[0] == 5] == expected
        return
    assert updated(box, case) == expected
