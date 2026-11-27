"""ooxml-edit reads ooxml-common's tables and number format codes (ooxml-edit 0.64.0 re-exports
them instead of keeping a copy): each agrees with ooxml-common's, so a copy creeping back in fails
here.  Moved from ooxml-common's suite (Phase 3 E2), whose legs no longer install ooxml-edit."""

from __future__ import annotations

import datetime as dt

import pytest

from ooxml_common import dates, numfmt
from ooxml_common.locales import Locale
from ooxml_edit import dates as edit_dates

# ooxml-edit's cases (its tests/test_dates.py), with Excel's answer.
EDIT_CASES = [
    (r"[$-413]d\ mmmm\ yyyy", True), ("m/d/yyyy", True), ("dd/mm/yy;@", True), ("h:mm", True),
    ("[h]:mm", True), ("General", False), ("@", False), ("#,##0.0", False), ('"days" 0', False),
    (r"\d0", False), ("[Red]0.0", False), ("[$€-413] #,##0.0", False), ("", False), (None, False),
]


def test_ooxml_edit_reads_every_code_as_excel_showed_it():
    differs = {code for code, kind, _ in numfmt.MEASURED
               if edit_dates.is_date_code(code) is not (kind in numfmt.DATE_KINDS)}
    assert differs == set()
    for code, is_date in EDIT_CASES:
        assert edit_dates.is_date_code(code) is is_date
    assert edit_dates.is_date_code("0;[Red]d", 1)


def test_ooxml_edits_names_are_these():
    # ooxml-edit 0.64.0 re-exports the tables instead of keeping a copy.
    assert dict(dates.LCIDS) == dict(edit_dates.LCIDS)
    assert dict(dates.LANGUAGES) == dict(edit_dates.LANGUAGES)
    assert {tag: tuple(names) for tag, names in dates.NAMES.items()} == \
        {tag: tuple(names) for tag, names in edit_dates.NAMES.items()}


@pytest.mark.parametrize("pattern", ["d mmmm yyyy", "dd/mm/yyyy", "ddd d mmm yy", "dddd, mmmmm",
                                     'd "de" mmmm', "yyyy-mm-dd", "m.d.yyyy"])
def test_a_code_ooxml_edit_keeps_prints_what_it_writes(pattern):
    regions = ["en-BE", "fr-BE", "de-CH", "en-IE", "fr-CH", "it-CH", "nl", "fr", "es", "it"]
    for tag in [*edit_dates.NAMES, *regions]:
        locale = Locale(language=tag)
        for day in (dt.date(2026, 1, 5), dt.date(2026, 3, 2), dt.date(2026, 12, 25)):
            code = edit_dates.format_code(pattern, locale)
            assert dates.format_serial(edit_dates.serial(day), code) == \
                edit_dates.format_date(day, pattern, locale), (tag, code)
