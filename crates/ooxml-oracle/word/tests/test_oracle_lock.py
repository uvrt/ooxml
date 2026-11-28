"""The oracle driver's machine-wide Word lock (``word/oracle.py``), without Word: a held lock means
"skip", never wait."""

from __future__ import annotations

import sys

import pytest

fcntl = pytest.importorskip("fcntl")  # flock is POSIX's; the Word oracle runs on macOS

import oracle

pytestmark = pytest.mark.skipif(sys.platform != "darwin", reason="Word's group container is macOS's")


def test_a_held_lock_is_word_in_use(monkeypatch):
    monkeypatch.setattr(oracle, "available", lambda: True)
    oracle.GROUP_CONTAINER.mkdir(parents=True, exist_ok=True)
    with open(oracle.LOCK, "a+") as other:
        fcntl.flock(other, fcntl.LOCK_EX | fcntl.LOCK_NB)
        try:
            with pytest.raises(oracle.WordBusy, match="lock"):
                with oracle.session():
                    pass
        finally:
            fcntl.flock(other, fcntl.LOCK_UN)


def test_a_running_word_is_word_in_use(monkeypatch):
    monkeypatch.setattr(oracle, "available", lambda: True)
    monkeypatch.setattr(oracle, "word_running", lambda: True)
    with pytest.raises(oracle.WordBusy, match="Word is in use"):
        with oracle.session():
            pass


def test_a_signal_ends_the_session_with_its_clean_up(monkeypatch, tmp_path):
    """SIGTERM inside a session (a ``timeout`` around a probe) runs the session's recovery,
    which quits the Word it started, before the lock is released (lane fix-final-grid)."""
    import os
    import signal

    monkeypatch.setattr(oracle, "available", lambda: True)
    monkeypatch.setattr(oracle, "word_running", lambda: False)
    monkeypatch.setattr(oracle, "ORACLE_DIR", tmp_path)
    monkeypatch.setattr(oracle, "LOCK", tmp_path / "word-oracle.lock")
    monkeypatch.setattr(oracle, "evict", lambda: None)
    recovered = []
    monkeypatch.setattr(oracle.Session, "recover", lambda self: recovered.append(True))
    before = signal.getsignal(signal.SIGTERM)
    with pytest.raises(SystemExit) as ended:
        with oracle.session():
            os.kill(os.getpid(), signal.SIGTERM)
    assert ended.value.code == 128 + signal.SIGTERM
    assert recovered == [True]
    assert signal.getsignal(signal.SIGTERM) is before


def _left_behind(monkeypatch, tmp_path, documents, windows):
    """A Word running as the process listing fakes it: `documents` open, `windows` windows."""
    running = [True]
    quit_calls = []
    monkeypatch.setattr(oracle, "available", lambda: True)
    monkeypatch.setattr(oracle, "ORACLE_DIR", tmp_path)
    monkeypatch.setattr(oracle, "LOCK", tmp_path / "word-oracle.lock")
    monkeypatch.setattr(oracle, "evict", lambda: None)
    monkeypatch.setattr(oracle, "word_running", lambda: running[0])
    monkeypatch.setattr(oracle, "_word_documents", lambda: documents)
    monkeypatch.setattr(oracle, "_word_windows", lambda: windows)
    monkeypatch.setattr(oracle.Session, "recover", lambda self: None)

    def run(command, **_kwargs):
        if "quit saving no" in " ".join(command):
            quit_calls.append(command)
            running[0] = False

    monkeypatch.setattr(oracle.subprocess, "run", run)
    return quit_calls


def test_a_word_the_oracle_left_behind_is_quit(monkeypatch, tmp_path):
    quit_calls = _left_behind(monkeypatch, tmp_path, [tmp_path / "fg-x-0123-in.docx"], 1)
    with oracle.session():
        pass
    assert len(quit_calls) == 1


def test_a_word_holding_another_document_is_in_use(monkeypatch, tmp_path):
    quit_calls = _left_behind(monkeypatch, tmp_path, [tmp_path / "fg-x-in.docx", tmp_path.parent / "mine.docx"], 2)
    with pytest.raises(oracle.WordBusy, match="Word is in use"):
        with oracle.session():
            pass
    assert quit_calls == []


def test_a_word_with_another_window_is_in_use(monkeypatch, tmp_path):
    quit_calls = _left_behind(monkeypatch, tmp_path, [tmp_path / "fg-x-in.docx"], 2)
    with pytest.raises(oracle.WordBusy, match="Word is in use"):
        with oracle.session():
            pass
    assert quit_calls == []


def test_a_word_with_no_document_is_in_use(monkeypatch, tmp_path):
    quit_calls = _left_behind(monkeypatch, tmp_path, [], 1)
    with pytest.raises(oracle.WordBusy, match="Word is in use"):
        with oracle.session():
            pass
    assert quit_calls == []
