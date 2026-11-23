"""Every observation of Word's (``observations/<name>.json``): the probe that measured it, the
tests that hold the tools to it, and for each of its cases no test reads, why.

A reader in ``word/tests`` (the recorded-Word tier) declares ``READS``, the observation's
top-level cases it compares with; ``tests/test_index.py`` checks that those and ``unread`` are
together every case of the observation (its notes -- ``_about``, ``source``, ``word`` -- aside).
Readers elsewhere (docx-agent's tests, the Rust crates' integration tests) are listed by path:
they name the observation they read, and their cases are theirs to cover.

Paths are from the repository's root.  Must not: import anything.
"""

from __future__ import annotations

#: The keys of an observation that are its notes, not Word's answers.
NOTES = frozenset({"_about", "source", "word"})

#: A case no tool can reach yet, to be read when one does.
_NOT_YET = "not read yet: the same edit through the tools is still to be written"

OBSERVATIONS: dict[str, dict] = {
    "charts-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/charts_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_charts_recorded.py"],
        "unread": {
            "titles": "how Word draws each title form (face, size, colour off its PDF): the drawing's, "
                      "held by the renderers, not an edit's output",
            "smartart": "Word's redraw of diagrams whose drawing and data disagree; no tool edits a "
                        "diagram since edit_smartart was withdrawn",
            "dictionary": "Word's AppleScript dictionary: what the probe could ask Word, not an answer",
        },
    },
    "e1-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/e1_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_e1_recorded.py"],
        "unread": {
            "style_ids_localised": "the Dutch interface's ids for the built-in styles; the tools write English "
                                   "Word's (the readers translate Word's with them)",
        },
    },
    "e2-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/e2_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_e2_recorded.py"],
        "unread": {},
    },
    "e3-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/e3_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_e3_recorded.py"],
        "unread": {
            "join-untracked": _NOT_YET + " (a deletion across a paragraph mark, tracking off)",
            "cells-pdf": "Word's PDF text of the recorded cell revisions: what Word draws, the renderers' "
                         "concern",
        },
    },
    "e4-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/e4_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_e4_recorded.py"],
        "unread": {
            "notes2": "a section's note numbering (format, start) and the endnotes' place: no tool sets them",
        },
    },
    "e5-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/e5_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_e5_recorded.py"],
        "unread": {
            "tables-tracked": _NOT_YET + " (the table cases with tracking on)",
            "merges-tracked": _NOT_YET + " (the merge cases with tracking on)",
            "mergeforms-accept": _NOT_YET + " (Word's Accept All of tracked merge forms)",
            "mergeforms-reject": _NOT_YET + " (Word's Reject All of tracked merge forms)",
            "drawings-tracked": _NOT_YET + " (the drawing cases with tracking on)",
            "controls": _NOT_YET + " (content controls, word_controls)",
            "controls-tracked": _NOT_YET + " (content controls with tracking on)",
            "controls-saved": _NOT_YET + " (a bound control's value after Word's save)",
        },
    },
    "e6-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/e6_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_e6_recorded.py"],
        "unread": {
            "template-dotx": "Word's File > New from a template keeps its body; new_document takes a template's "
                             "styles, theme and layouts, not its content, by design",
            "template-docx": "as template-dotx, the template written as a .docx",
            "resave-11": "Word's plain save of a mode 11 document: the baseline Convert is told apart from",
            "resave-all-11": "as resave-11, every legacy option set",
            "resave-all-14": "as resave-11, mode 14",
            "resave-14-sample": "as resave-11, a corpus document",
            "copy-formatted-tracked": _NOT_YET + " (a copy between documents with tracking on)",
        },
    },
    "language-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/language_probe.py",
        "readers": ["python/docx-agent/tests/test_language.py"],
    },
    "merged-rows-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/merged_rows_probe.py",
        "readers": ["python/docx-agent/tests/test_merged_rows_r59.py"],
    },
    "paraid-durability.json": {
        "probe": "crates/ooxml-oracle/word/probes/paraid_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_paraid_recorded.py"],
        "unread": {
            "mixed": "docx-agent's retired paragraph stamping beside other tools' ids; the tools stamp one way "
                     "(decided, factors)",
            "word-created": "a document Word made, with no paraIds: its stamping is decided's 'Word-created' case",
        },
    },
    "row-format-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/row_format_probe.py",
        "readers": ["python/docx-agent/tests/test_rows_r58.py"],
    },
    "rows-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/rows_probe.py",
        "readers": ["python/docx-agent/tests/test_own_rows.py",
                    "crates/ooxml-tools-word/tests/integration/tracked_rows.rs"],
    },
    "separator-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/separator_probe.py",
        "readers": ["python/docx-agent/tests/test_own_rows.py"],
    },
    "styleref-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/styleref_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_styleref_recorded.py"],
        "unread": {},
    },
    "table-regions-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/table_regions_probe.py",
        "readers": ["python/docx-agent/tests/test_table_regions_r58.py"],
    },
    "theme-tints-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/theme_tint_probe.py",
        "readers": ["python/docx-agent/tests/test_declared_styles.py"],
    },
    "toc-numbers-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/toc_numbers_probe.py",
        "readers": ["python/docx-agent/tests/test_toc_numbers.py",
                    "crates/ooxml-tools-word/tests/integration/structure.rs"],
    },
    "toc-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/toc_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_toc_recorded.py"],
        "unread": {},
    },
    "tracking-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/tracking_probe.py",
        "readers": ["python/docx-agent/tests/test_own_insertions.py"],
    },
    "trial-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/trial_probe.py",
        "readers": ["crates/ooxml-oracle/word/tests/test_trial_recorded.py"],
        "unread": {
            "document-as-docm": "no tool writes a macro-enabled document",
            "document-saved-docm": "no tool writes a macro-enabled document",
            "word-saves-docm": "what Word's own Save As .docm writes: no tool writes one",
        },
    },
    "typography-word.json": {
        "probe": "crates/ooxml-oracle/word/probes/typography_probe.py",
        "readers": ["python/docx-agent/tests/test_typography.py"],
    },
}
