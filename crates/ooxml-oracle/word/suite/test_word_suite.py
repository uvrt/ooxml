"""Word on the tools' outputs, cheapest check first (``pytest -m word``, serial, a Mac with
Word): every case's output (a) opens and exports in Word without a prompt, showing the text its
case expects; (b) keeps its paraIds, texts, revision authors and dates and comments through
Word's re-save; (c) reads after Word's Accept All and Reject All as after the tools'; (d) lays
out on Word's pages (page count, and each paragraph on Word's page); (e) has Word's field results
after an update of every field.  Word's answers are recorded (``observed.py``) whatever the
verdict; ``deviations.py`` lists where the tools differ knowingly."""

from __future__ import annotations

import pytest

import answers
import compare
import edit_sets
import observed
import oracle
from deviations import reason
from word_side import WordSide, word_version

pytestmark = pytest.mark.word

CASES = edit_sets.all_cases()
IDS = [case.id for case in CASES]


@pytest.fixture(scope="module")
def word():
    try:
        with oracle.session() as session:
            yield WordSide(session)
    except oracle.WordBusy as busy:
        pytest.skip(str(busy))


@pytest.fixture(scope="module")
def store():
    found = observed.Store(word_version())
    yield found
    found.write()


@pytest.fixture(scope="module")
def outputs(box):
    """Each case's tool output, made once."""
    made: dict[str, bytes | Exception] = {}

    def output(case: edit_sets.Case) -> bytes:
        if case.id not in made:
            try:
                made[case.id] = edit_sets.run(box, case)
            except edit_sets.Refused as refused:
                made[case.id] = refused
        found = made[case.id]
        if isinstance(found, Exception):
            pytest.skip(f"the tools refused the case: {found}")
        return found

    return output


def _name(case: edit_sets.Case) -> str:
    return "suite-" + case.id.replace("/", "-")[:60]


def _opened(word, store, case, data) -> None:
    """Word must have opened the output for a later check to ask it anything."""
    if store.entry(case.id, edit_sets.digest(data)).get("opens") != "done":
        outcome = word.export(data, _name(case))
        if not outcome:
            pytest.skip(f"Word did not open the output ({outcome.outcome})")


def _verdict(case, check, ours, theirs) -> None:
    why = reason(case.id, check)
    differences = compare.judge(check, ours, theirs)
    if why:
        if differences:
            pytest.xfail(why)
        pytest.fail(f"listed as a deviation ({why}), but the tools now agree with Word", pytrace=False)
    assert not differences, "\n".join(differences)


def test_gates_cases_are_frozen_as_data():
    cases, why = edit_sets.gate_cases()
    if why:
        pytest.skip(why)
    assert cases


@pytest.mark.parametrize("case", CASES, ids=IDS)
def test_word_opens_and_exports_it(word, store, outputs, case):
    data = outputs(case)
    output = edit_sets.digest(data)
    outcome = word.export(data, _name(case))
    store.record(case.id, output, "opens", outcome.outcome)
    assert outcome, f"Word did not open it: {outcome.outcome} {outcome.detail}"
    if case.shows:
        text = " ".join(" ".join(oracle.pdf_pages(outcome.path)).split())
        missing = [want for want in case.shows if want not in text]
        assert not missing, (missing, text[:400])


@pytest.mark.parametrize("case", CASES, ids=IDS)
def test_word_resave_keeps_it(word, store, outputs, box, case):
    data = outputs(case)
    _opened(word, store, case, data)
    saved = word.resave(data, _name(case))
    assert saved is not None, "Word did not save it"
    theirs = answers.kept_facts(saved)
    store.record(case.id, edit_sets.digest(data), "kept", theirs)
    _verdict(case, "kept", answers.kept_facts(data), theirs)


@pytest.mark.parametrize("action", ["accept", "reject"])
@pytest.mark.parametrize("case", CASES, ids=IDS)
def test_word_review_reads_as_ours(word, store, outputs, box, case, action):
    data = outputs(case)
    if not answers.has_markup(data):
        pytest.skip("no tracked changes or comments")
    _opened(word, store, case, data)
    reviewed = word.reviewed(data, action, _name(case))
    assert reviewed is not None, f"Word's {action} all did not save"
    reader = answers.Reader(box)
    theirs = reader.markdown(reviewed)
    store.record(case.id, edit_sets.digest(data), action, theirs)
    _verdict(case, action, answers.ours(reader, data, action), theirs)


@pytest.mark.parametrize("case", CASES, ids=IDS)
def test_word_pages_are_ours(word, store, outputs, box, case):
    data = outputs(case)
    _opened(word, store, case, data)
    reader = answers.Reader(box)
    accepted = reader.reviewed(data, "accept")
    outcome = word.export(accepted, _name(case) + "-accepted")
    assert outcome, f"Word did not export the accepted output: {outcome.outcome}"
    text = WordSide.pdf_pages(outcome.path)
    theirs = {"count": len(text), "text": text}
    store.record(case.id, edit_sets.digest(data), "pages", theirs)
    _verdict(case, "pages", answers.ours(reader, data, "pages"), theirs)


@pytest.mark.parametrize("case", CASES, ids=IDS)
def test_word_field_results_are_ours(word, store, outputs, box, case):
    data = outputs(case)
    reader = answers.Reader(box)
    if not reader.fields(data, update=False):
        pytest.skip("no fields")
    _opened(word, store, case, data)
    updated = word.fields_updated(data, _name(case))
    assert updated is not None, "Word's field update did not save"
    theirs = reader.fields(updated, update=False)
    store.record(case.id, edit_sets.digest(data), "fields", theirs)
    _verdict(case, "fields", answers.ours(reader, data, "fields"), theirs)
