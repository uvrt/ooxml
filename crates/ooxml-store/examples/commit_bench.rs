//! Measures the commit critical section (design section 15, acceptance 3): a document of
//! 70,273 blocks (the 3k document's block count) held as a persistent vector (three levels
//! of 64-wide arrays: a clone shares everything, a change copies three arrays), writers that
//! each read one block and set its text, 1 and 24 threads, on a hot set of 200 blocks and
//! on all blocks, with no other work and (as spike 2) with 500 µs of preparing work per edit.
//! Prints JSON: per run, commits, conflicts, exclusive runs, and the time
//! inside the commit lock per commit (p50, p99, max), best of `--rounds` (default 3).
//!
//! `cargo run --release -p ooxml-store --example commit_bench`

use ooxml_store::{Caller, Clock, Command, DocModel, Error, KeySet, Store, StoreConfig, Txn};
use std::sync::Arc;
use std::time::{Duration, Instant};

const BLOCKS: u32 = 70_273;

/// Width of each level of [`Blocks`].
const WIDTH: usize = 64;

type Chunk = Arc<[Arc<str>]>;
type Middle = Arc<[Chunk]>;

/// Block texts as a persistent vector of three levels (up to 64^3 blocks).
#[derive(Clone)]
struct Blocks(Arc<[Middle]>);

impl Blocks {
    fn new(texts: impl Iterator<Item = Arc<str>>) -> Self {
        let texts: Vec<Arc<str>> = texts.collect();
        let chunks: Vec<Chunk> = texts.chunks(WIDTH).map(Arc::from).collect();
        Self(chunks.chunks(WIDTH).map(Arc::from).collect())
    }

    fn get(&self, at: usize) -> Option<&Arc<str>> {
        self.0
            .get(at / (WIDTH * WIDTH))?
            .get(at / WIDTH % WIDTH)?
            .get(at % WIDTH)
    }

    fn set(&mut self, at: usize, text: Arc<str>) -> bool {
        let (top, mid, low) = (at / (WIDTH * WIDTH), at / WIDTH % WIDTH, at % WIDTH);
        let Some(middle) = self.0.get(top) else { return false };
        let Some(chunk) = middle.get(mid) else { return false };
        if low >= chunk.len() {
            return false;
        }
        let mut chunk = chunk.to_vec();
        let mut middle = middle.to_vec();
        let mut root = self.0.to_vec();
        if let (Some(slot), Some(m), Some(r)) = (chunk.get_mut(low), middle.get_mut(mid), root.get_mut(top)) {
            *slot = text;
            *m = Arc::from(chunk);
            *r = Arc::from(middle);
        }
        self.0 = Arc::from(root);
        true
    }
}

#[derive(Debug, Clone)]
struct SetText(u32, Arc<str>);

#[derive(Debug, thiserror::Error)]
#[error("no block {0}")]
struct NoBlock(u32);

impl DocModel for Blocks {
    type Key = u32;
    type Change = SetText;
    type Error = NoBlock;
    type Setup = ();
    type Delta = ();

    fn delta(_: &Self, _: &Self) {}

    fn apply(&mut self, change: &SetText) -> Result<(), NoBlock> {
        if self.set(change.0 as usize, Arc::clone(&change.1)) {
            Ok(())
        } else {
            Err(NoBlock(change.0))
        }
    }

    fn inverse(&self, change: &SetText) -> Result<SetText, NoBlock> {
        let old = self.get(change.0 as usize).ok_or(NoBlock(change.0))?;
        Ok(SetText(change.0, Arc::clone(old)))
    }

    fn agrees(&self, other: &Self, keys: &KeySet<u32>) -> bool {
        match keys.keys() {
            None => (0..BLOCKS as usize).all(|at| self.get(at) == other.get(at)),
            Some(keys) => keys.iter().all(|k| self.get(*k as usize) == other.get(*k as usize)),
        }
    }

    fn written(change: &SetText, into: &mut KeySet<u32>) {
        into.insert(change.0);
    }
}

/// Set block `.0`'s text, after `.1` of busy work standing in for an edit's preparation.
struct Edit(u32, Duration);

impl Command<Blocks> for Edit {
    type Out = ();
    type Error = Error<Blocks>;

    fn validate(&self, _doc: &Blocks) -> Result<(), Error<Blocks>> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, Blocks>) -> Result<(), Error<Blocks>> {
        tx.read(self.0);
        let until = Instant::now() + self.1;
        while Instant::now() < until {
            std::hint::spin_loop();
        }
        let len = tx.doc().get(self.0 as usize).map_or(0, |text| text.len());
        tx.write(SetText(
            self.0,
            Arc::from(format!("block {} edited, was {len} bytes", self.0)),
        ))
    }
}

struct System(Instant);

impl Clock for System {
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
}

struct Run {
    threads: u64,
    work: Duration,
    hot: u32,
    commits: u64,
    conflicts: u64,
    exclusive: u64,
    held_ns: Vec<u64>,
    wall: Duration,
}

fn run(
    threads: u64,
    hot: u32,
    per_thread: u32,
    work: Duration,
) -> Result<Run, Box<dyn std::error::Error + Send + Sync>> {
    let doc = Blocks::new((0..BLOCKS).map(|i| Arc::from(format!("block {i}"))));
    let store = Store::new(doc, StoreConfig::default(), Arc::new(System(Instant::now())));
    let started = Instant::now();
    let mut held_ns = Vec::new();
    std::thread::scope(|scope| -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let store = &store;
                scope.spawn(move || -> Result<Vec<u64>, Error<Blocks>> {
                    let caller = Caller::named(&format!("writer {t}"));
                    let mut x = 0x9E37_79B9_7F4A_7C15u64 ^ t;
                    let mut held = Vec::new();
                    for _ in 0..per_thread {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        let block = u32::try_from(x % u64::from(hot)).unwrap_or(0);
                        let done = store.run(&caller, &Edit(block, work))?;
                        held.push(u64::try_from(done.timing.held.as_nanos()).unwrap_or(u64::MAX));
                    }
                    Ok(held)
                })
            })
            .collect();
        for handle in handles {
            held_ns.extend(handle.join().map_err(|_| "a writer panicked")??);
        }
        Ok(())
    })?;
    let wall = started.elapsed();
    let stats = store.stats();
    held_ns.sort_unstable();
    Ok(Run {
        threads,
        work,
        hot,
        commits: stats.commits,
        conflicts: stats.conflicts,
        exclusive: stats.exclusive,
        held_ns,
        wall,
    })
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let len = sorted.len();
    if len == 0 {
        return 0;
    }
    let rank = ((len - 1) as f64 * p).round() as usize;
    sorted.get(rank).copied().unwrap_or(0)
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let rounds: u32 = std::env::args()
        .skip_while(|a| a != "--rounds")
        .nth(1)
        .map_or(Ok(3), |r| r.parse())?;
    let mut rows = Vec::new();
    let none = Duration::ZERO;
    let spike = Duration::from_micros(500);
    let configs = [
        (1u64, BLOCKS, none, 24_000u32),
        (24, BLOCKS, none, 24_000),
        (24, 200, none, 24_000),
        (1, 200, spike, 2_400),
        (24, 200, spike, 2_400),
    ];
    for (threads, hot, work, edits) in configs {
        let mut best: Option<Run> = None;
        for _ in 0..rounds {
            let r = run(threads, hot, edits / u32::try_from(threads)?, work)?;
            if best
                .as_ref()
                .is_none_or(|b| percentile(&r.held_ns, 0.5) < percentile(&b.held_ns, 0.5))
            {
                best = Some(r);
            }
        }
        if let Some(r) = best {
            rows.push(format!(
                "{{\"threads\": {}, \"prepare_us\": {}, \"hot_blocks\": {}, \"commits\": {}, \"conflicts\": {}, \"exclusive\": {}, \
                 \"held_ns_p50\": {}, \"held_ns_p99\": {}, \"held_ns_max\": {}, \"wall_ms\": {:.1}}}",
                r.threads,
                r.work.as_micros(),
                r.hot,
                r.commits,
                r.conflicts,
                r.exclusive,
                percentile(&r.held_ns, 0.5),
                percentile(&r.held_ns, 0.99),
                r.held_ns.last().copied().unwrap_or(0),
                r.wall.as_secs_f64() * 1e3,
            ));
        }
    }
    println!(
        "{{\"blocks\": {BLOCKS}, \"rounds\": {rounds}, \"best_by\": \"held_ns_p50\", \"runs\": [\n  {}\n]}}",
        rows.join(",\n  ")
    );
    Ok(())
}
