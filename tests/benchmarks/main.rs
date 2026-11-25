//! The benchmark gates, each judged by the pinned kit's `benchmarks/rust.txt`, in a target of its own so nothing else
//! runs beside them. A timing verdict from an unoptimised build means nothing, so a debug build runs every benchmark
//! briefly, its identity checks included, and judges no timing; `cargo test --release --test benchmarks` judges.
//!
//! On a platform the kit's file gives no line, or with `-- --measure <repetitions>`, every benchmark is measured that
//! many times and each sample printed, which is what a line is set from; a run on a platform with no line fails.

#[path = "../support/mod.rs"]
mod support;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode;

use weivel_rust::benchmarks::{
    BenchmarkFiles, BenchmarkOptions, LanguageBudget, NoiseSample, PairSummaries, RatioGate,
    Sampling, Slowed, Summary, benchmark_control_call, benchmark_definition_call,
    benchmark_described_call, benchmark_library_call, benchmark_same_language_call,
    default_budget_path, judge_noise, judge_ratio, load_language_budget, noise_sample, ratio_of,
};

mod bounds;

/// How many samples of each gate a run takes on a platform the kit gives no line.
const MEASURING_REPETITIONS: u32 = 10;

/// Enough batches to tell a slowed path apart at a fraction of the gate's run time.
const BRIEF: Sampling = Sampling {
    warmup_calls: 1_000,
    calls_per_batch: 1_000,
    batches: 21,
};

type Check<'a> = (&'static str, Box<dyn FnOnce() -> Result<(), String> + 'a>);

fn files() -> BenchmarkFiles {
    BenchmarkFiles {
        core: support::shared_library("weivel_core"),
        standard: support::shared_library("weivel_standard"),
        echo: support::shared_library("echo"),
        described: support::shared_library("weivel_standard_described"),
    }
}

fn platform() -> &'static str {
    weivel_rust::benchmarks::this_platform()
}

fn runner() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn report(name: &str, summary: &Summary) {
    println!(
        "weivel-benchmark {name} runner={} median_ns={:.2} lower_quartile_ns={:.2} upper_quartile_ns={:.2} p99_ns={:.2}",
        runner(),
        summary.median,
        summary.lower_quartile,
        summary.upper_quartile,
        summary.percentile_99
    );
}

fn report_pair(gate: &str, summaries: &PairSummaries) -> f64 {
    report(&format!("{gate}-bare"), &summaries.bare);
    report(gate, &summaries.weivel);
    let ratio = ratio_of(summaries);
    println!(
        "weivel-benchmark-sample {gate} {} ratio={ratio:.4}",
        platform()
    );
    ratio
}

fn report_noise(name: &str, sample: NoiseSample) {
    println!(
        "weivel-benchmark-sample {name} {} noise={:.6} deviation={:.6}",
        platform(),
        weivel_rust::benchmarks::deviation(sample.noise),
        weivel_rust::benchmarks::deviation(sample.ratio)
    );
}

fn options(sampling: Option<Sampling>, slowed: Option<Slowed>) -> BenchmarkOptions {
    BenchmarkOptions { sampling, slowed }
}

/// Runs every benchmark once, its checks before the timing included, and reports what it measured without judging it.
fn sample_every_benchmark(sampling: Option<Sampling>) -> Result<Samples, String> {
    let files = files();
    let measured = |failure: weivel_rust::benchmarks::BenchmarkFailure| failure.0;
    let same_language =
        benchmark_same_language_call(&files, &options(sampling, None)).map_err(measured)?;
    report("same-language-direct", &same_language.direct);
    report("same-language-direct-again", &same_language.direct_again);
    report(
        "same-language-through-weivel",
        &same_language.through_weivel,
    );
    let same_language = noise_sample(&same_language);
    report_noise("same-language", same_language);
    let control_call = report_pair(
        "control-call",
        &benchmark_control_call(&files, &options(sampling, None)).map_err(measured)?,
    );
    let library_call = report_pair(
        "library-call",
        &benchmark_library_call(&files, &options(sampling, None)).map_err(measured)?,
    );
    let definition =
        benchmark_definition_call(&files, &options(sampling, None)).map_err(measured)?;
    report("library-call-definition-direct", &definition.direct);
    report("library-call-definition", &definition.through_weivel);
    let definition = noise_sample(&definition);
    report_noise("library-call-definition", definition);
    let described = benchmark_described_call(&files, &options(sampling, None)).map_err(measured)?;
    report("described-call-echo", &described.echo);
    let described_call = report_pair("described-call", &described.pair);
    Ok(Samples {
        same_language,
        definition,
        control_call,
        library_call,
        described_call,
    })
}

/// One sample of every gate.
struct Samples {
    same_language: NoiseSample,
    definition: NoiseSample,
    control_call: f64,
    library_call: f64,
    described_call: f64,
}

fn judge(budget: Option<&LanguageBudget>, samples: &Samples) -> Vec<Result<(), String>> {
    let here = platform();
    vec![
        judge_noise(budget, here, samples.same_language)
            .map_err(|why| format!("same-language: {why}")),
        judge_noise(budget, here, samples.definition)
            .map_err(|why| format!("library-call definition: {why}")),
        judge_ratio(budget, RatioGate::ControlCall, here, samples.control_call),
        judge_ratio(budget, RatioGate::LibraryCall, here, samples.library_call),
        judge_ratio(
            budget,
            RatioGate::DescribedCall,
            here,
            samples.described_call,
        ),
    ]
}

fn every_verdict(verdicts: Vec<Result<(), String>>) -> Result<(), String> {
    let failures: Vec<String> = verdicts.into_iter().filter_map(Result::err).collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn has_every_line(budget: &LanguageBudget) -> bool {
    RatioGate::ALL
        .iter()
        .all(|gate| budget.ratio_bound(*gate, platform()).is_some())
        && budget.noise_bounds(platform()).is_some()
}

fn ratio_bound_here(budget: &LanguageBudget, gate: RatioGate) -> Result<f64, String> {
    budget
        .ratio_bound(gate, platform())
        .ok_or_else(|| format!("no bound for {} on {}", gate.kit_name(), platform()))
}

/// A ratio gate is red once its weivel path is slowed past the bound by twice the bare call's time over it.
fn slowed_ratio_is_red(
    budget: &LanguageBudget,
    gate: RatioGate,
    benchmark: impl Fn(&BenchmarkOptions) -> Result<PairSummaries, String>,
) -> Result<(), String> {
    let bound = ratio_bound_here(budget, gate)?;
    let measured = benchmark(&options(Some(BRIEF), None))?;
    let nanoseconds = (measured.bare.median * bound * 2.0).ceil() as u64;
    let slowed = benchmark(&options(
        Some(BRIEF),
        Some(Slowed {
            path: "weivel",
            nanoseconds,
        }),
    ))?;
    match judge_ratio(Some(budget), gate, platform(), ratio_of(&slowed)) {
        Ok(()) => Err(format!(
            "{} slowed by {nanoseconds} ns a call still passed",
            gate.kit_name()
        )),
        Err(_) => Ok(()),
    }
}

/// A pair of identical cost is red once its path through weivel is slowed.
fn slowed_noise_is_red(
    budget: &LanguageBudget,
    name: &str,
    nanoseconds: u64,
    benchmark: impl Fn(&BenchmarkOptions) -> Result<NoiseSample, String>,
) -> Result<(), String> {
    let slowed = benchmark(&options(
        Some(BRIEF),
        Some(Slowed {
            path: "through_weivel",
            nanoseconds,
        }),
    ))?;
    match judge_noise(Some(budget), platform(), slowed) {
        Ok(()) => Err(format!(
            "{name} slowed by {nanoseconds} ns a call still passed"
        )),
        Err(_) => Ok(()),
    }
}

fn described_pair(options: &BenchmarkOptions) -> Result<PairSummaries, String> {
    benchmark_described_call(&files(), options)
        .map(|described| described.pair)
        .map_err(|failure| failure.0)
}

fn red_proofs(budget: &LanguageBudget) -> Vec<Check<'static>> {
    let budgets: Vec<LanguageBudget> = (0..5).map(|_| budget.clone()).collect();
    let [
        same_budget,
        definition_budget,
        control_budget,
        library_budget,
        described_budget,
    ] = <[LanguageBudget; 5]>::try_from(budgets).expect("five copies");
    let same_language = |options: &BenchmarkOptions| {
        benchmark_same_language_call(&files(), options)
            .map(|summaries| noise_sample(&summaries))
            .map_err(|failure| failure.0)
    };
    let definition = |options: &BenchmarkOptions| {
        benchmark_definition_call(&files(), options)
            .map(|summaries| noise_sample(&summaries))
            .map_err(|failure| failure.0)
    };
    vec![
        (
            "the same-language gate is red once the path through weivel is slowed",
            Box::new(move || {
                slowed_noise_is_red(&same_budget, "same-language", 200, same_language)
            }),
        ),
        (
            "the definition row is red once the path through weivel is slowed",
            Box::new(move || {
                slowed_noise_is_red(&definition_budget, "definition", 2_000, definition)
            }),
        ),
        (
            "the control-call gate is red once the weivel call is slowed past its bound",
            Box::new(move || {
                slowed_ratio_is_red(
                    &control_budget,
                    RatioGate::ControlCall,
                    |options: &BenchmarkOptions| {
                        benchmark_control_call(&files(), options).map_err(|failure| failure.0)
                    },
                )
            }),
        ),
        (
            "the library-call gate is red once the weivel call is slowed past its bound",
            Box::new(move || {
                slowed_ratio_is_red(
                    &library_budget,
                    RatioGate::LibraryCall,
                    |options: &BenchmarkOptions| {
                        benchmark_library_call(&files(), options).map_err(|failure| failure.0)
                    },
                )
            }),
        ),
        (
            "the described-call gate is red once the weivel call is slowed past its bound",
            Box::new(move || {
                slowed_ratio_is_red(&described_budget, RatioGate::DescribedCall, described_pair)
            }),
        ),
    ]
}

fn run(checks: Vec<Check<'_>>) -> bool {
    let mut passed = true;
    for (name, check) in checks {
        let outcome = catch_unwind(AssertUnwindSafe(check))
            .unwrap_or_else(|panic| Err(format!("panicked: {panic:?}")));
        match outcome {
            Ok(()) => println!("test {name} ... ok"),
            Err(why) => {
                println!("test {name} ... FAILED: {why}");
                passed = false;
            }
        }
    }
    passed
}

fn requested_repetitions() -> Option<u32> {
    let arguments: Vec<String> = std::env::args().collect();
    let position = arguments
        .iter()
        .position(|argument| argument == "--measure")?;
    arguments.get(position + 1)?.parse().ok()
}

fn measure(repetitions: u32) -> Result<(), String> {
    for repetition in 1..=repetitions {
        println!("weivel-benchmark-repetition {repetition} of {repetitions}");
        sample_every_benchmark(None)?;
    }
    Ok(())
}

fn gating_checks(budget: Result<LanguageBudget, String>) -> Vec<Check<'static>> {
    let budget = match budget {
        Ok(budget) if has_every_line(&budget) => budget,
        unbounded => {
            let why = match unbounded {
                Ok(_) => format!(
                    "the kit's budget gives {} no line for every gate",
                    platform()
                ),
                Err(why) => why,
            };
            return vec![(
                "every gate has a bound here, so its samples are judged",
                Box::new(move || {
                    measure(MEASURING_REPETITIONS)?;
                    Err(format!(
                        "{why}, so the samples above are measured and nothing passes"
                    ))
                }),
            )];
        }
    };
    let mut checks = red_proofs(&budget);
    checks.insert(
        0,
        (
            "every gate holds the kit's bound on this platform",
            Box::new(move || {
                let samples = sample_every_benchmark(None)?;
                every_verdict(judge(Some(&budget), &samples))
            }),
        ),
    );
    checks
}

fn main() -> ExitCode {
    let mut checks = bounds::checks();
    if let Some(repetitions) = requested_repetitions() {
        checks.push((
            "measures every benchmark",
            Box::new(move || measure(repetitions)),
        ));
    } else if cfg!(debug_assertions) {
        println!(
            "weivel-benchmark: an unoptimised build judges no timing; every benchmark runs briefly instead"
        );
        checks.push((
            "every benchmark measures, its identity checks passing",
            Box::new(|| sample_every_benchmark(Some(BRIEF)).map(|_| ())),
        ));
    } else {
        let budget = load_language_budget(&default_budget_path()).map_err(|refusal| refusal.0);
        checks.extend(gating_checks(budget));
    }
    if run(checks) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
