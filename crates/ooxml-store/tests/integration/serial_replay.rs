//! Random transactions from several threads equal a serial replay in commit order: the same
//! outputs and, version by version, the same documents. Ids drawn are never given twice.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common;

use common::{still, Cells, Mix, Rng};
use ooxml_store::{Caller, Seq, Snapshot, Store, StoreConfig};
use std::collections::{BTreeMap, BTreeSet};

struct Record {
    seq: Seq,
    command: Mix,
    out: (i64, u64),
    version: Snapshot<Cells>,
}

fn commands(seed: u64, count: usize, cells: u64) -> Vec<Mix> {
    let mut rng = Rng(seed);
    (0..count)
        .map(|_| {
            let reads = (0..rng.below(3)).map(|_| rng.below(cells) as u32).collect();
            Mix {
                reads,
                write: rng.below(cells) as u32,
                add: rng.below(1000) as i64,
                dawdle: rng.below(4) == 0,
            }
        })
        .collect()
}

fn run_threads(
    threads: u64,
    per_thread: usize,
    cells: u64,
    seed: u64,
    config: StoreConfig,
) -> (Vec<Record>, Store<Cells>) {
    let store = Store::new(Cells::default(), config, still());
    let mut records = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let store = &store;
                scope.spawn(move || {
                    let caller = Caller::named(&format!("writer {t}"));
                    commands(seed ^ (t + 1).wrapping_mul(0x1234_5678), per_thread, cells)
                        .into_iter()
                        .map(|command| {
                            let done = store.run(&caller, &command).unwrap();
                            assert!(done.changed);
                            Record {
                                seq: done.version.seq(),
                                command,
                                out: done.out,
                                version: done.version,
                            }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for handle in handles {
            records.extend(handle.join().unwrap());
        }
    });
    records.sort_by_key(|r| r.seq);
    (records, store)
}

fn check_against_replay(records: &[Record]) {
    // Every number given once, in order.
    let seqs: Vec<u64> = records.iter().map(|r| r.seq.0).collect();
    assert_eq!(seqs, (1..=records.len() as u64).collect::<Vec<_>>());
    // Ids never given twice.
    let ids: BTreeSet<u64> = records.iter().map(|r| r.out.1).collect();
    assert_eq!(ids.len(), records.len());
    // Serially, in commit order: the same outputs and the same document at every version.
    let serial = Store::new(Cells::default(), StoreConfig::default(), still());
    let caller = Caller::anonymous();
    for record in records {
        let done = serial.run(&caller, &record.command).unwrap();
        assert_eq!(done.version.seq(), record.seq);
        assert_eq!(done.out.0, record.out.0, "output at {}", record.seq);
        assert_eq!(done.version.doc(), record.version.doc(), "document at {}", record.seq);
    }
}

#[test]
fn threads_equal_a_serial_replay_in_commit_order() {
    for seed in [1u64, 2, 3] {
        let (records, store) = run_threads(8, 150, 24, seed, StoreConfig::default());
        check_against_replay(&records);
        assert_eq!(store.snapshot().doc(), records.last().unwrap().version.doc());
        let stats = store.stats();
        assert_eq!(stats.commits, records.len() as u64);
        assert!(stats.conflicts > 0, "a hot set of 24 cells should conflict: {stats:?}");
    }
}

#[test]
fn exclusive_runs_after_conflicts_keep_the_order() {
    // One optimistic try: every conflict goes to the run under the lock.
    let config = StoreConfig {
        optimistic_tries: 1,
        ..StoreConfig::default()
    };
    let (records, store) = run_threads(6, 120, 6, 9, config);
    check_against_replay(&records);
    assert!(store.stats().exclusive > 0);
}

#[test]
fn per_caller_steps_are_listed_with_their_reads_and_writes() {
    let (records, store) = run_threads(3, 10, 8, 4, StoreConfig::default());
    let steps = store.steps();
    assert_eq!(steps.len(), records.len());
    let by_seq: BTreeMap<Seq, &Record> = records.iter().map(|r| (r.seq, r)).collect();
    for info in &steps {
        let record = by_seq[&info.step];
        assert!(info.writes.contains(&record.command.write));
        assert!(record.command.reads.iter().all(|cell| info.reads.contains(cell)));
        assert!(info.caller.as_deref().unwrap().starts_with("writer "));
    }
    let latest = store.latest_step_by("writer 0").unwrap();
    assert_eq!(
        steps
            .iter()
            .rev()
            .find(|s| s.caller.as_deref() == Some("writer 0"))
            .unwrap()
            .step,
        latest
    );
}
