//! Timing paths side by side: warm-up calls first, then rounds of one timed batch per path.

use std::hint::black_box;
use std::time::{Duration, Instant};

/// How a path is sampled: warm-up calls first, then batches of calls, each batch timed as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sampling {
    /// Calls made of each path before any is timed.
    pub warmup_calls: u64,
    /// Calls in one timed batch.
    pub calls_per_batch: u64,
    /// Rounds, each timing one batch of every path.
    pub batches: u32,
}

impl Sampling {
    /// How many calls a sampling makes of each path, warm-up included.
    pub fn calls_per_path(&self) -> u64 {
        self.warmup_calls + u64::from(self.batches) * self.calls_per_batch
    }
}

/// The nanoseconds one call took over every batch, as each batch's time divided by its calls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    /// The fastest batch.
    pub minimum: f64,
    /// The batch a quarter of the way up.
    pub lower_quartile: f64,
    /// The middle batch.
    pub median: f64,
    /// The batch three quarters of the way up.
    pub upper_quartile: f64,
    /// The batch 99 percent of the way up.
    pub percentile_99: f64,
    /// The slowest batch.
    pub maximum: f64,
}

/// A measured path, handed the call's index so no call is the same as the one before it.
pub type Path<'a> = &'a mut dyn FnMut(u64);

/// A path's name and the path.
pub type NamedPath<'a> = (&'static str, Path<'a>);

/// A path to slow down, by spinning before each of its calls: what a red proof of a gate needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slowed {
    /// The name of the path to slow.
    pub path: &'static str,
    /// How long to spin before each of its calls.
    pub nanoseconds: u64,
}

fn spin(nanoseconds: u64) {
    let end = Instant::now() + Duration::from_nanos(nanoseconds);
    while Instant::now() < end {
        std::hint::spin_loop();
    }
}

fn time_batch(path: &mut dyn FnMut(u64), calls: u64, spun: Option<u64>) -> f64 {
    let start = Instant::now();
    for call in 0..calls {
        if let Some(nanoseconds) = spun {
            spin(nanoseconds);
        }
        path(black_box(call));
    }
    start.elapsed().as_nanos() as f64 / calls as f64
}

/// The names in the order round `round` samples them: each round starts one name later than the round before.
pub fn round_order(count: usize, round: usize) -> Vec<usize> {
    (0..count).map(|index| (round + index) % count).collect()
}

/// Samples every path in turn, one batch of each per round, so a change in the machine's speed during the run falls on
/// every path alike, and rotates which path goes first each round, so no path always follows the same one. Answers
/// each path's batch times, one per round, in the order the paths were given.
pub fn sample_interleaved(
    paths: &mut [NamedPath<'_>],
    sampling: Sampling,
    slowed: Option<Slowed>,
) -> Vec<Vec<f64>> {
    let spun: Vec<Option<u64>> = paths
        .iter()
        .map(|(name, _)| {
            slowed
                .filter(|slowed| slowed.path == *name)
                .map(|slowed| slowed.nanoseconds)
        })
        .collect();
    for (index, (_, path)) in paths.iter_mut().enumerate() {
        time_batch(*path, sampling.warmup_calls, spun[index]);
    }
    let mut samples = vec![Vec::with_capacity(sampling.batches as usize); paths.len()];
    for round in 0..sampling.batches as usize {
        for index in round_order(paths.len(), round) {
            let time = time_batch(paths[index].1, sampling.calls_per_batch, spun[index]);
            samples[index].push(time);
        }
    }
    samples
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let rank = (fraction * sorted.len() as f64).ceil() as usize;
    sorted[rank.max(1) - 1]
}

/// Summarises batch times.
///
/// # Panics
/// When `samples` is empty: nothing measured is never a summary.
pub fn summarise(samples: &[f64]) -> Summary {
    assert!(!samples.is_empty(), "nothing was measured");
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    Summary {
        minimum: sorted[0],
        lower_quartile: percentile(&sorted, 0.25),
        median: percentile(&sorted, 0.5),
        upper_quartile: percentile(&sorted, 0.75),
        percentile_99: percentile(&sorted, 0.99),
        maximum: sorted[sorted.len() - 1],
    }
}

/// The ratio of `numerator` over `denominator` in each round, summarised: the batches of one round ran side by side.
pub fn summarise_round_ratios(numerator: &[f64], denominator: &[f64]) -> Summary {
    let ratios: Vec<f64> = numerator
        .iter()
        .zip(denominator)
        .map(|(time, reference)| time / reference)
        .collect();
    summarise(&ratios)
}
