"""The kinds of text a per-kind minimum text size tells apart, held to Word
(``probes/min_text_size_probe.py``, ``observations/min-text-size-word.json``, Word 16.106 for Mac;
round 60, R140): the probe document is the one the Rust host's reader opens, Word opens it, and
each paragraph's story, size and style are what the document says -- a caption the style Word's
own insert caption uses. The tools' answers are held to the same observation in
``crates/ooxml-oracle/tests/integration/host_min_text_size.rs`` (the per-kind setting is the Rust
host's)."""

from __future__ import annotations

from recorded import observed  # noqa: I001  (puts the probes on the path)

from min_text_size_cases import PARAGRAPHS, document  # noqa: E402

import probe_paths  # noqa: E402

OBSERVED = observed("min-text-size-word.json")
#: The observation's cases these tests compare with.
READS = {"word-kinds", "word-caption", "tools-opened"}
#: The committed copy the Rust reader opens.
COPY = probe_paths.OBSERVATIONS.parents[1] / "tests/data/word-probes/min_text_size_probe/kinds.docx"
#: Word's story types, by the cases' stories.
STORIES = {"body": "main text story", "header": "primary header story", "footer": "primary footer story",
           "footnote": "footnotes story", "endnote": "endnotes story"}


def test_the_rust_readers_copy_is_the_probes_document():
    assert COPY.read_bytes() == document()


def test_word_opens_the_document_without_a_prompt():
    assert OBSERVED["tools-opened"]["outcome"] == "done", OBSERVED["tools-opened"]


def test_each_paragraphs_story_and_size_are_words():
    kinds = OBSERVED["word-kinds"]
    assert set(kinds) == set(PARAGRAPHS)
    for key, (_, size, story, _, _) in PARAGRAPHS.items():
        assert kinds[key]["story"] == STORIES[story], key
        assert kinds[key]["size"] == size, key


def test_a_caption_is_in_the_style_words_own_caption_takes():
    """Word's insert caption gives its caption the document's style named ``caption`` (a Dutch
    Word's id ``Bijschrift``): 0.1's rule -- the style's id, else its name, is ``caption`` -- names
    the same style, and Word reads the probe's captions as in it, and nothing else."""
    made = OBSERVED["word-caption"]
    assert (made["style_id"], made["style_name"]) == ("Bijschrift", "caption")
    captions = {key for key, fact in OBSERVED["word-kinds"].items() if fact["style"] == made["style_id"]}
    assert captions == {key for key, (_, _, _, style, _) in PARAGRAPHS.items() if style == "Bijschrift"}
