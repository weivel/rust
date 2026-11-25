//! The Rust half of the benchmarks, as the core's `docs/contract/benchmarks.md` asks of a language's `benchmarks`
//! module, each gate judged as the conformance kit's `docs/benchmarks.md` defines it and bounded by the pinned kit's
//! `benchmarks/rust.txt`.
//!
//! The same-language gate is the identity check: a consumer's get answers the provider's own object, and the timing
//! beside it, three paths running the very same compiled call, is judged only against the platform's noise. The
//! library call's definition row is judged the same way, its identity the address the definition's symbol answered.
//! `control-call`, `library-call` and `described-call` are ratio gates over the same call made past the layer.

use std::fmt;

mod bare;
#[forbid(unsafe_code)]
mod budget;
#[forbid(unsafe_code)]
mod gates;
mod hand;
#[forbid(unsafe_code)]
mod measure;
#[forbid(unsafe_code)]
mod pairs;
#[forbid(unsafe_code)]
mod world;

pub use budget::{
    BudgetRefusal, KIT_PLATFORMS, LanguageBudget, NoiseBounds, RatioGate, default_budget_path,
    deviation, load_language_budget, read_language_budget, same_language_bound, this_platform,
    too_noisy_to_judge, within_ratio,
};
pub use gates::{NoiseSample, judge_noise, judge_ratio, noise_sample, ratio_of};
pub use hand::{HandPoint, HandSample, HandValue};
pub use measure::{
    NamedPath, Path, Sampling, Slowed, Summary, round_order, sample_interleaved, summarise,
    summarise_round_ratios,
};
pub use pairs::{
    Arithmetic, BenchmarkOptions, DescribedCallSummaries, PairSummaries, SameLanguageSummaries,
    benchmark_control_call, benchmark_definition_call, benchmark_described_call,
    benchmark_library_call, benchmark_same_language_call, default_sampling,
};
pub use world::BenchmarkFiles;

/// Why a benchmark could not measure: a world that would not compose, or a check before the timing that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkFailure(pub String);

impl fmt::Display for BenchmarkFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BenchmarkFailure {}
