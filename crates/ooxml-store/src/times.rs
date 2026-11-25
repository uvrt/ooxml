//! Owns: how long the commit lock is held and waited for, every time it is taken
//! ([`LockTimes`]): two log-linear histograms kept with the state under the lock, so recording
//! costs no atomic and no allocation; read with [`crate::Store::lock_times`].
//!
//! Must not: read the clock (the store hands it the times); be process-wide.

use std::time::Duration;

/// Sub-buckets per power of two: a bucket is at most 1/4 of its lower bound wide (a
/// percentile is within 25% of the true value, above it).
const SUB: u32 = 4;
/// The highest power of two kept apart: times from 2^41 ns (about 37 minutes) go in the last
/// bucket.
const TOP: u32 = 40;
const BUCKETS: usize = (TOP * SUB) as usize;

/// Times, counted in log-linear buckets: `count`, `total`, the exact `max`, and percentiles
/// read as the upper bound of the bucket they fall in.
#[derive(Clone, PartialEq, Eq)]
pub struct Histogram {
    buckets: Box<[u64]>,
    count: u64,
    total: Duration,
    max: Duration,
}

impl std::fmt::Debug for Histogram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Histogram")
            .field("count", &self.count)
            .field("p50", &self.percentile(0.5))
            .field("p99", &self.percentile(0.99))
            .field("max", &self.max)
            .finish()
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            buckets: vec![0; BUCKETS].into_boxed_slice(),
            count: 0,
            total: Duration::ZERO,
            max: Duration::ZERO,
        }
    }
}

impl Histogram {
    /// Count `time`.
    pub(crate) fn record(&mut self, time: Duration) {
        let ns = u64::try_from(time.as_nanos()).unwrap_or(u64::MAX);
        if let Some(bucket) = self.buckets.get_mut(bucket_of(ns)) {
            *bucket += 1;
        }
        self.count += 1;
        self.total = self.total.saturating_add(time);
        self.max = self.max.max(time);
    }

    /// How many times were counted.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }

    /// The times summed.
    #[must_use]
    pub fn total(&self) -> Duration {
        self.total
    }

    /// The longest time.
    #[must_use]
    pub fn max(&self) -> Duration {
        self.max
    }

    /// The time `q` (0 to 1) of the counted times are at or below: the upper bound of its
    /// bucket, never above [`Histogram::max`]; zero when none were counted.
    #[must_use]
    pub fn percentile(&self, q: f64) -> Duration {
        if self.count == 0 {
            return Duration::ZERO;
        }
        // the rank of the time asked for, from 1
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let rank = ((q.clamp(0.0, 1.0) * self.count as f64).ceil() as u64).max(1);
        let mut seen = 0;
        for (index, count) in self.buckets.iter().enumerate() {
            seen += count;
            if seen >= rank {
                return Duration::from_nanos(upper_of(index)).min(self.max);
            }
        }
        self.max
    }

    /// The counts of `other` added.
    pub fn merge(&mut self, other: &Histogram) {
        for (mine, theirs) in self.buckets.iter_mut().zip(other.buckets.iter()) {
            *mine += theirs;
        }
        self.count += other.count;
        self.total = self.total.saturating_add(other.total);
        self.max = self.max.max(other.max);
    }
}

/// The bucket of `ns` nanoseconds: one each below `2 * SUB`, then [`SUB`] per power of two.
fn bucket_of(ns: u64) -> usize {
    let first = u64::from(2 * SUB);
    if ns < first {
        return usize::try_from(ns).unwrap_or(0);
    }
    let power = 63 - ns.leading_zeros(); // 2^power <= ns, power >= 3
    if power > TOP {
        return BUCKETS - 1;
    }
    let shift = power - SUB.trailing_zeros();
    #[allow(clippy::cast_possible_truncation)]
    let sub = ((ns >> shift) as u32) & (SUB - 1);
    ((power - 1) * SUB + sub) as usize
}

/// The largest time (ns) in bucket `index`.
fn upper_of(index: usize) -> u64 {
    let index = u32::try_from(index).unwrap_or(u32::MAX);
    if index < 2 * SUB {
        return u64::from(index);
    }
    let (power, sub) = (index / SUB + 1, index % SUB);
    let shift = power - SUB.trailing_zeros();
    (1u64 << power) + (u64::from(sub) << shift) + (1u64 << shift) - 1
}

/// How long the commit lock was held and waited for, each time it was taken (commits,
/// history operations, groups and the reads of the history that take it), and how long the
/// gate was held and waited for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LockTimes {
    /// From taking the lock to letting it go.
    pub held: Histogram,
    /// From asking for the lock to taking it.
    pub waited: Histogram,
    /// How long a command that conflicted held the gate, preparing and committing behind it
    /// (the commit lock is not held meanwhile).
    pub gated: Histogram,
    /// How long a commit waited for the gate to open, and a command for the gate to be its.
    pub gate_waited: Histogram,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bucket_holds_its_bounds() {
        for ns in [
            0u64,
            1,
            3,
            4,
            5,
            7,
            8,
            100,
            1_000,
            65_535,
            1 << 20,
            123_456_789,
            1 << 39,
        ] {
            let index = bucket_of(ns);
            assert!(
                upper_of(index) >= ns,
                "{ns}: bucket {index} ends at {}",
                upper_of(index)
            );
            if index > 0 {
                assert!(upper_of(index - 1) < ns, "{ns}: bucket {index} starts too late");
            }
        }
    }

    #[test]
    fn percentiles_read_the_bucket_above() {
        let mut h = Histogram::default();
        for us in 1..=100u64 {
            h.record(Duration::from_micros(us));
        }
        assert_eq!(h.count(), 100);
        assert_eq!(h.max(), Duration::from_micros(100));
        let p50 = h.percentile(0.5);
        assert!(
            p50 >= Duration::from_micros(50) && p50 <= Duration::from_micros(63),
            "{p50:?}"
        );
        assert_eq!(h.percentile(1.0), Duration::from_micros(100));
        assert_eq!(Histogram::default().percentile(0.5), Duration::ZERO);
    }
}
