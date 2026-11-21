//! Lane E: the format-neutral core -- one owner per document: its own thread, commits
//! (change sets) taken in version order and coalesced (every commit that arrived while the
//! owner was busy is applied by its next pass, as one), immutable snapshots published after
//! each pass and shared by every reader, and a reader's wait for the first snapshot that
//! holds its version.  Nothing here knows Word: a format (`Format`) says how a commit is
//! read, how commits change its state, and what a snapshot of that state is.  Word's layout
//! is one format (`crate::engine::WordLayout`); a deck's or a workbook's would be others.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// A document kind behind an owner.
pub trait Format: Send + 'static {
    /// One change set, as the owner's thread reads it (parsed there, not by the caller).
    type Commit: Send + 'static;
    /// What readers share: never changed once published.
    type Snap: Send + Sync + 'static;
    /// What one pass did (for the log).
    type Stat: Send + 'static;

    fn parse(&self, text: &str) -> Result<Self::Commit, String>;
    fn version_of(commit: &Self::Commit) -> u64;
    /// Apply commits (in version order) as one pass.
    fn apply(&mut self, commits: Vec<Self::Commit>) -> Self::Stat;
    fn snapshot(&self) -> Self::Snap;
    fn snap_version(snap: &Self::Snap) -> u64;
    /// Lane H1: a snapshot that is no longer the latest (another was published): what a
    /// reader made of it to read it faster may go (made again if it is read).
    fn retire(_snap: &Self::Snap) {}
    /// Lane F: whether each commit is laid out and published on its own (every version
    /// read exactly: `Owner::wait_exact`) rather than coalesced with those that came with it.
    fn per_commit(&self) -> bool {
        false
    }
}

/// Lane F: the snapshots kept besides the latest, for a reader of an exact version.
pub const KEPT: usize = 64;

pub enum Msg {
    Commit(String, Instant),
    Stop,
}

/// One pass of the owner: the format's own stat, how many commits it took, the time from
/// the oldest commit's arrival to the snapshot's publication, and the parse and publish
/// times.
pub struct Pass<S> {
    pub stat: S,
    pub commits: usize,
    pub parse_ms: f64,
    pub publish_ms: f64,
    pub commit_to_publish_ms: f64,
}

pub struct Shared<F: Format> {
    pub snap: Mutex<Arc<F::Snap>>,
    /// Lane F: the latest snapshots, oldest first (at most `KEPT`).
    pub kept: Mutex<std::collections::VecDeque<Arc<F::Snap>>>,
    pub cv: Condvar,
    pub passes: Mutex<Vec<Pass<F::Stat>>>,
    pub error: Mutex<Option<String>>,
}

pub struct Owner<F: Format> {
    pub tx: Mutex<Sender<Msg>>,
    pub shared: Arc<Shared<F>>,
    pub thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

impl<F: Format> Owner<F> {
    /// The owner of a document whose state is `state` (laid out already).
    pub fn start(state: F, _name: &str) -> Owner<F> {
        let first = Arc::new(state.snapshot());
        let shared = Arc::new(Shared {
            kept: Mutex::new(std::collections::VecDeque::from(vec![first.clone()])),
            snap: Mutex::new(first),
            cv: Condvar::new(),
            passes: Mutex::new(Vec::new()),
            error: Mutex::new(None),
        });
        let (tx, rx) = channel();
        let s2 = shared.clone();
        let thread = std::thread::spawn(move || run(state, rx, s2));
        Owner {
            tx: Mutex::new(tx),
            shared,
            thread: Mutex::new(Some(thread)),
        }
    }

    /// Hand a commit to the owner's thread; returns at once.
    pub fn commit(&self, text: String) {
        self.tx.lock().unwrap().send(Msg::Commit(text, Instant::now())).unwrap();
    }

    /// The first snapshot of version `version` or later (waits for it).
    pub fn wait(&self, version: u64) -> Arc<F::Snap> {
        let mut g = self.shared.snap.lock().unwrap();
        while F::snap_version(&g) < version {
            if let Some(e) = self.shared.error.lock().unwrap().as_ref() {
                panic!("owner failed: {e}");
            }
            g = self.shared.cv.wait(g).unwrap();
        }
        g.clone()
    }

    /// Lane F: the snapshot of exactly `version` (waits for it); `None` where a later one
    /// was published without it (coalesced) or it is no longer kept.
    pub fn wait_exact(&self, version: u64) -> Option<Arc<F::Snap>> {
        let latest = self.wait(version);
        if F::snap_version(&latest) == version {
            return Some(latest);
        }
        let kept = self.shared.kept.lock().unwrap();
        kept.iter().rev().find(|s| F::snap_version(s) == version).cloned()
    }

    pub fn latest(&self) -> Arc<F::Snap> {
        self.shared.snap.lock().unwrap().clone()
    }

    pub fn stop(&self) {
        let _ = self.tx.lock().unwrap().send(Msg::Stop);
        if let Some(t) = self.thread.lock().unwrap().take() {
            let _ = t.join();
        }
    }
}

fn fail<F: Format>(shared: &Shared<F>, msg: String) {
    *shared.error.lock().unwrap() = Some(msg);
    shared.cv.notify_all();
}

fn run<F: Format>(mut state: F, rx: Receiver<Msg>, shared: Arc<Shared<F>>) {
    loop {
        let first = match rx.recv() {
            Ok(m) => m,
            Err(_) => return,
        };
        // everything that arrived meanwhile: one pass
        let mut msgs = vec![first];
        while let Ok(m) = rx.try_recv() {
            msgs.push(m);
        }
        let mut stop = false;
        let mut commits = Vec::new();
        let tp = Instant::now();
        let mut oldest: Option<Instant> = None;
        for m in msgs {
            match m {
                Msg::Stop => stop = true,
                Msg::Commit(s, at) => {
                    oldest = Some(oldest.map_or(at, |o: Instant| o.min(at)));
                    match state.parse(&s) {
                        Ok(c) => commits.push(c),
                        Err(e) => return fail(&shared, format!("commit: {e}")),
                    }
                }
            }
        }
        commits.sort_by_key(|c| F::version_of(c));
        let parse_ms = ms(tp);
        let groups: Vec<Vec<F::Commit>> = if state.per_commit() {
            commits.into_iter().map(|c| vec![c]).collect()
        } else if commits.is_empty() {
            Vec::new()
        } else {
            vec![commits]
        };
        for commits in groups {
            let n = commits.len();
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.apply(commits))) {
                Ok(stat) => {
                    let tpub = Instant::now();
                    let snap = Arc::new(state.snapshot());
                    {
                        let mut kept = shared.kept.lock().unwrap();
                        kept.push_back(snap.clone());
                        while kept.len() > KEPT {
                            kept.pop_front();
                        }
                    }
                    // the pass is in the log before any reader sees its snapshot: the log is
                    // held while the snapshot is published, so a reader woken by it (or one
                    // that reads the latest at once) finds the pass that made it
                    let mut passes = shared.passes.lock().unwrap();
                    let previous = std::mem::replace(&mut *shared.snap.lock().unwrap(), snap);
                    F::retire(&previous);
                    shared.cv.notify_all();
                    let publish_ms = ms(tpub);
                    let waited = oldest.map_or(0.0, |o| o.elapsed().as_secs_f64() * 1000.0);
                    passes.push(Pass {
                        stat,
                        commits: n,
                        parse_ms,
                        publish_ms,
                        commit_to_publish_ms: waited,
                    });
                    // a log for whoever reads it (`stats`), not a history: the oldest go
                    if passes.len() > 4096 {
                        passes.drain(..2048);
                    }
                }
                Err(e) => {
                    let msg = e
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_default();
                    return fail(&shared, msg);
                }
            }
        }
        if stop {
            return;
        }
    }
}
