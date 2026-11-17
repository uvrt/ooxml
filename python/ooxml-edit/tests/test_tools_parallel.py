"""Round 10, O1 and O3: how many workers, warmed before their first task, several tasks of
one call at once (``run_all``), images kept by content (``ContentStore``), the bytes of a
version made once, and the image budget asked before rendering."""

from __future__ import annotations

import threading

import pytest

from ooxml_edit.tools import (ContentStore, InProcess, Limits, SubprocessPool, ToolError,
                              WorkerPool, available_cpus, content_key, default_workers,
                              run_all)
from ooxml_edit.tools import worker as worker_module
from ooxml_edit.tools.rendercache import ENTRY_OVERHEAD, code_stamp

import gates
import tools_toys


# -- how many workers ----------------------------------------------------------------------


def _cgroup(tmp_path, files: dict[str, str]):
    for name, text in files.items():
        path = tmp_path / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
    return str(tmp_path)


def test_a_cgroup_v2_quota_is_read(tmp_path, monkeypatch):
    monkeypatch.setattr(worker_module, "_cgroup_paths", lambda: ("/", None))
    assert worker_module.cgroup_cpu_quota(_cgroup(tmp_path, {"cpu.max": "250000 100000\n"})) == 2.5


def test_no_v2_quota_reads_as_none(tmp_path, monkeypatch):
    monkeypatch.setattr(worker_module, "_cgroup_paths", lambda: ("/", None))
    assert worker_module.cgroup_cpu_quota(_cgroup(tmp_path, {"cpu.max": "max 100000\n"})) is None


def test_the_smallest_quota_along_the_cgroup_and_above_it_holds(tmp_path, monkeypatch):
    monkeypatch.setattr(worker_module, "_cgroup_paths", lambda: ("/pod/app", None))
    root = _cgroup(tmp_path, {"cpu.max": "max 100000", "pod/cpu.max": "100000 100000",
                              "pod/app/cpu.max": "300000 100000"})
    assert worker_module.cgroup_cpu_quota(root) == 1.0


def test_a_cgroup_v1_quota_is_read(tmp_path, monkeypatch):
    monkeypatch.setattr(worker_module, "_cgroup_paths", lambda: (None, "/docker/abc"))
    root = _cgroup(tmp_path, {"cpu,cpuacct/docker/abc/cpu.cfs_quota_us": "150000",
                              "cpu,cpuacct/docker/abc/cpu.cfs_period_us": "100000"})
    assert worker_module.cgroup_cpu_quota(root) == 1.5


def test_no_cgroup_file_system_is_no_quota(tmp_path, monkeypatch):
    monkeypatch.setattr(worker_module, "_cgroup_paths", lambda: (None, None))
    assert worker_module.cgroup_cpu_quota(str(tmp_path / "absent")) is None


def test_the_default_workers_follow_the_quota_never_the_hosts_count(monkeypatch):
    monkeypatch.setattr(worker_module, "cgroup_cpu_quota", lambda: 1.5)
    monkeypatch.setattr(worker_module.os, "cpu_count", lambda: 64)
    assert available_cpus() <= 2
    assert default_workers() <= 2
    monkeypatch.setattr(worker_module, "cgroup_cpu_quota", lambda: None)
    assert 1 <= default_workers() <= worker_module.MAX_DEFAULT_WORKERS
    assert available_cpus() >= 1


# -- warming, and deadlines that do not count it -------------------------------------------


@pytest.mark.parametrize("make", [
    lambda: WorkerPool(1, warm_up=[tools_toys.slow_warm_up]),
    lambda: SubprocessPool(1, warm_up=[tools_toys.slow_warm_up]),
], ids=["processes", "subprocesses"])
def test_a_new_worker_warms_up_first_and_the_warm_up_is_no_tasks_time(make):
    with make() as pool:
        # The warm-up takes 2 s; the task's deadline is 1 s, and it is met.
        assert pool.run(tools_toys.warmed, timeout=1.0) == "1"
        assert pool.killed == []


def test_warm_starts_the_workers_ahead_of_use(gate, monkeypatch):
    monkeypatch.setenv(tools_toys.WARM_GATE, gate.path)
    with WorkerPool(2, warm_up=[tools_toys.gated_warm_up]) as pool:
        try:
            pool.warm()                                # in the background: the warm-ups held
            assert "warm out" not in gate.order() and pool.starting == 2
        finally:
            gate.open()
        pool.warm(wait=True)
        assert len(pool.pids()) == 2 and pool.starting == 0
        pids = sorted(pool.pids())
        assert pool.run(tools_toys.warmed, timeout=1.0) == "1"
        # Warm already: no worker started, nor warmed up, for the task.
        assert sorted(pool.pids()) == pids and gate.order().count("warm in") == 2


def test_warm_waits_for_a_worker_that_is_started_but_not_idle(monkeypatch):
    """A worker that has started is still counted until it is idle: a ``warm(wait=True)``
    after a background ``warm()`` returns only when every worker is ready."""
    held = threading.Event()
    entered = threading.Semaphore(0)
    with WorkerPool(2) as pool:
        deliver = pool._deliver

        def slow(worker):                   # between "started" and "idle"
            entered.release()
            assert held.wait(60)
            return deliver(worker)

        monkeypatch.setattr(pool, "_deliver", slow)
        waiter = threading.Thread(target=pool.warm, kwargs={"wait": True}, daemon=True)
        try:
            pool.warm()
            assert entered.acquire(timeout=60) and entered.acquire(timeout=60)
            assert pool.pids() == [] and pool.starting == 2
            waiter.start()
            waiter.join(0.3)
            assert waiter.is_alive()        # both are started, neither is ready
        finally:
            held.set()
        waiter.join(60)
        assert not waiter.is_alive()
        assert len(pool.pids()) == 2 and pool.starting == 0


@pytest.mark.parametrize("warm_up", [tools_toys.failing_warm_up, tools_toys.raise_tool_error])
def test_a_warm_up_that_fails_leaves_the_worker_serving(warm_up, caplog):
    with WorkerPool(1, warm_up=[warm_up]) as pool:
        assert pool.run(tools_toys.worker_pid, timeout=60)
    assert any("warm-up" in record.getMessage() for record in caplog.records)


def test_the_in_process_runner_has_nothing_to_warm():
    InProcess(1).warm(wait=True)


# -- several tasks at once -----------------------------------------------------------------


def test_run_all_runs_at_once_and_keeps_the_order(gate):
    with WorkerPool(3) as pool:
        pool.warm(wait=True)
        # The gate opens once all three are held at it: one after another, never.
        opener = gates.open_when(gate, lambda: len(gate.order()) == 3)
        results = run_all(pool, tools_toys.gated_value,
                          [(gate.path, "a", "a"), (gate.path, "b", "b"), (gate.path, "c", "c")], [90, 90, 90])
        opener.join()
    assert results == ["a", "b", "c"]
    assert sorted(gate.order()[:3]) == ["a in", "b in", "c in"], gate.order()


def test_run_all_puts_an_exception_in_its_place():
    with WorkerPool(2) as pool:
        results = run_all(pool, tools_toys.sleep_then, [(0.0, 1), (5.0, 2)], [30, 0.5])
    assert results[0] == 1
    assert isinstance(results[1], ToolError) and results[1].code == "timeout"


def test_the_deadline_of_a_task_grows_with_its_views():
    limits = Limits(render_timeout=20.0)
    assert limits.render_timeout_for(1) == 20.0
    assert limits.render_timeout_for(4) == 20.0
    assert limits.render_timeout_for(5) == 40.0
    assert limits.render_timeout_for(30) == 160.0


# -- the store ----------------------------------------------------------------------------


def test_a_content_key_tells_parts_apart():
    assert content_key("ab", "c") != content_key("a", "bc")
    assert content_key(b"x", 1, [2]) == content_key(b"x", 1, [2])
    assert len(content_key()) == 64


def test_the_store_is_bounded_in_bytes_least_recently_used_first():
    store = ContentStore(3 * (100 + ENTRY_OVERHEAD))
    for name in "abc":
        store.put(name, b"x" * 100, {"name": name})
    assert store.get("a") == (b"x" * 100, {"name": "a"})   # a is used: b goes next
    store.put("d", b"y" * 100)
    assert "b" not in store and {"a", "c", "d"} <= set(store.entries())
    assert store.size <= store.max_bytes
    store.put("huge", b"z" * 10_000)                       # larger than the budget: not kept
    assert "huge" not in store and len(store) == 3


def test_the_store_is_safe_to_share_between_threads():
    store = ContentStore(50 * (10 + ENTRY_OVERHEAD))

    def work(offset: int) -> None:
        for number in range(500):
            store.put(f"{offset}-{number}", b"0123456789")
            store.get(f"{offset}-{number - 1}")

    threads = [threading.Thread(target=work, args=(offset,)) for offset in range(8)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    assert len(store) == 50 and store.size == 50 * (10 + ENTRY_OVERHEAD)


def test_a_code_stamp_names_a_module_that_does_not_import():
    stamp = code_stamp("ooxml_edit", "no_such_module_here")
    assert stamp[-1] == ["no_such_module_here", None]
    assert stamp is code_stamp("ooxml_edit", "no_such_module_here")   # read once


# -- asked before rendering -----------------------------------------------------------------


def test_the_image_budget_is_asked_without_counting():
    from ooxml_edit.tools import Session

    session = Session(limits=Limits(image_budget=2, image_budget_per_round=1))
    with pytest.raises(ToolError) as raised:
        session.check_images(2)
    assert raised.value.details["budget"] == "round"
    session.check_images(1)
    assert session.images_used == 0
    session.reserve_images(1)
    with pytest.raises(ToolError):
        session.check_images(1)
    session.new_round()
    session.check_images(1)
    session.reserve_images(1)
    with pytest.raises(ToolError) as raised:
        session.check_images(1)
    assert raised.value.details["budget"] == "session"
