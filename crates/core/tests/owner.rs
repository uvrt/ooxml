//! The owner with a toy format: commits applied in version order, coalesced, snapshots
//! published and waited for; a failing commit fails the waiters instead of hanging them.

use ooxml_core::owner::{Format, Owner};
use std::sync::{Arc, Barrier};

/// A list of numbers; a commit is `"<version> <number>"`, appended; a pass sleeps a little
/// so that commits arriving meanwhile are taken by the next one.
struct Numbers {
    version: u64,
    items: Vec<i64>,
}

#[derive(Debug)]
struct Snap {
    version: u64,
    items: Arc<Vec<i64>>,
}

impl Format for Numbers {
    type Commit = (u64, i64);
    type Snap = Snap;
    type Stat = usize;

    fn parse(&self, text: &str) -> Result<(u64, i64), String> {
        let mut parts = text.split_whitespace();
        let v = parts.next().and_then(|s| s.parse().ok()).ok_or("no version")?;
        let n = parts.next().and_then(|s| s.parse().ok()).ok_or("no number")?;
        Ok((v, n))
    }
    fn version_of(commit: &(u64, i64)) -> u64 {
        commit.0
    }
    fn apply(&mut self, commits: Vec<(u64, i64)>) -> usize {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let n = commits.len();
        for (v, x) in commits {
            assert!(v > self.version, "commits in version order");
            self.version = v;
            self.items.push(x);
        }
        n
    }
    fn snapshot(&self) -> Snap {
        Snap {
            version: self.version,
            items: Arc::new(self.items.clone()),
        }
    }
    fn snap_version(snap: &Snap) -> u64 {
        snap.version
    }
}

fn start() -> Owner<Numbers> {
    Owner::start(
        Numbers {
            version: 0,
            items: Vec::new(),
        },
        "test",
    )
}

#[test]
fn commits_in_version_order_and_coalesced() {
    let owner = start();
    assert_eq!(owner.latest().version, 0);
    // sent out of order within one burst: the owner sorts them
    for v in [1u64, 3, 2, 4, 6, 5, 7, 8, 9, 10] {
        owner.commit(format!("{v} {}", v * 10));
    }
    let snap = owner.wait(10);
    assert_eq!(snap.version, 10);
    let passes: Vec<usize> = owner.shared.passes.lock().unwrap().iter().map(|p| p.commits).collect();
    assert_eq!(passes.iter().sum::<usize>(), 10);
    assert!(
        passes.len() < 10,
        "commits that arrive during a pass are coalesced: {passes:?}"
    );
    owner.stop();
}

#[test]
fn a_reader_waits_for_its_version() {
    let owner = Arc::new(start());
    let ready = Arc::new(Barrier::new(2));
    let reader = {
        let (owner, ready) = (owner.clone(), ready.clone());
        std::thread::spawn(move || {
            ready.wait();
            owner.wait(3).items.clone()
        })
    };
    ready.wait();
    for v in 1..=3 {
        owner.commit(format!("{v} {v}"));
    }
    let items = reader.join().unwrap();
    assert_eq!(*items, vec![1, 2, 3]);
    // an older version is served by the latest snapshot at once
    assert_eq!(owner.wait(1).version, 3);
    owner.stop();
}

#[test]
fn a_bad_commit_fails_the_waiters() {
    let owner = Arc::new(start());
    owner.commit("1 not-a-number".to_string());
    let waiter = {
        let owner = owner.clone();
        std::thread::spawn(move || owner.wait(1).version)
    };
    assert!(waiter.join().is_err(), "the wait panics with the owner's error");
}
