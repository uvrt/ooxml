"""Lane P2c6: a document's gate -- served calls side by side, the document's lock alone."""

import threading
import time

import pytest

from ooxml_edit.tools.gate import Gate, GatedLock


def test_shared_holders_run_side_by_side():
    gate = Gate()
    inside = threading.Barrier(3, timeout=5)

    def hold():
        with gate.shared():
            inside.wait()       # all three inside at once

    threads = [threading.Thread(target=hold) for _ in range(3)]
    for t in threads:
        t.start()
    for t in threads:
        t.join(5)
    assert not any(t.is_alive() for t in threads)


def test_the_lock_waits_for_shared_holders_and_holds_new_ones_back():
    gate = Gate()
    lock = GatedLock(gate)
    order: list[str] = []
    entered = threading.Event()
    release = threading.Event()

    def served():
        with gate.shared():
            entered.set()
            release.wait(5)
            order.append("served")

    def locked():
        with lock:
            order.append("locked")

    first = threading.Thread(target=served)
    first.start()
    entered.wait(5)
    waiter = threading.Thread(target=locked)
    waiter.start()
    time.sleep(0.05)
    assert not lock.acquire(blocking=False)       # a served call holds the gate
    late = threading.Thread(target=lambda: (gate.shared().__enter__(), order.append("late")))
    late.start()
    time.sleep(0.05)
    assert order == []                            # the late one waits behind the lock
    release.set()
    for t in (first, waiter, late):
        t.join(5)
    assert order == ["served", "locked", "late"]


def test_the_lock_is_reentrant_and_lets_the_gate_go_last():
    gate = Gate()
    lock = GatedLock(gate)
    with lock:
        with lock:
            pass
        assert not gate.acquire_exclusive(blocking=False)
    assert gate.acquire_exclusive(blocking=False)
    gate.release_exclusive()


def test_taking_the_lock_while_holding_the_gate_shared_is_an_error():
    gate = Gate()
    lock = GatedLock(gate)
    with gate.shared():
        with pytest.raises(RuntimeError):
            lock.acquire()
