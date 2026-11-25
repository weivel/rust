//! The reading of the kit's bounds and the arithmetic of the verdicts, which no timing decides.

use weivel_rust::benchmarks::{
    NoiseBounds, NoiseSample, RatioGate, default_budget_path, judge_noise, judge_ratio,
    read_language_budget, round_order, same_language_bound, too_noisy_to_judge,
};

use super::Check;

const BUDGET: &str = "control-call linux 1.25
control-call windows 1.5
control-call macos 1.75
library-call linux 2
library-call windows 2.5
library-call macos 3
described-call linux 4
described-call windows 5
described-call macos 6
ratio-margin 1.5
quiet-noise linux 0.001
quiet-noise windows 0.002
quiet-noise macos 0.003
worst-noise linux 0.01
worst-noise windows 0.02
worst-noise macos 0.03
noise-margin linux 10
noise-margin windows 11
noise-margin macos 12
";

fn expect(holds: bool, what: &str) -> Result<(), String> {
    if holds { Ok(()) } else { Err(what.to_owned()) }
}

fn bounds_are_the_highest_ratio_times_the_margin() -> Result<(), String> {
    let budget = read_language_budget(BUDGET).map_err(|refusal| refusal.0)?;
    expect(
        budget.ratio_bound(RatioGate::ControlCall, "windows") == Some(1.5 * 1.5),
        "control-call windows is 1.5 times the margin",
    )?;
    expect(
        budget.ratio_bound(RatioGate::DescribedCall, "macos") == Some(6.0 * 1.5),
        "described-call macos is 6 times the margin",
    )?;
    expect(
        budget
            .ratio_bound(RatioGate::LibraryCall, "freebsd")
            .is_none(),
        "a platform with no line has no bound",
    )?;
    expect(
        budget.noise_bounds("linux")
            == Some(NoiseBounds {
                quiet: 0.001,
                worst: 0.01,
                margin: 10.0,
            }),
        "linux's noise bounds are its three lines",
    )
}

fn a_platform_without_a_line_fails_its_gate() -> Result<(), String> {
    let windowless = BUDGET
        .lines()
        .filter(|line| {
            !line.starts_with("described-call windows") && !line.starts_with("noise-margin windows")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let budget = read_language_budget(&windowless).map_err(|refusal| refusal.0)?;
    expect(
        judge_ratio(Some(&budget), RatioGate::DescribedCall, "windows", 1.0).is_err(),
        "a ratio gate with no line fails",
    )?;
    expect(
        judge_noise(
            Some(&budget),
            "windows",
            NoiseSample {
                noise: 1.0,
                ratio: 1.0,
            },
        )
        .is_err(),
        "a pair with no noise margin fails",
    )?;
    expect(
        judge_ratio(None, RatioGate::ControlCall, "linux", 1.0).is_err(),
        "no budget at all fails",
    )?;
    expect(
        judge_ratio(Some(&budget), RatioGate::DescribedCall, "linux", 6.0).is_ok()
            && judge_ratio(Some(&budget), RatioGate::DescribedCall, "linux", 6.01).is_err(),
        "a ratio passes up to its bound and fails past it",
    )
}

fn a_malformed_budget_is_refused() -> Result<(), String> {
    let refused = |text: &str| read_language_budget(text).is_err();
    let without_margin: String = BUDGET
        .lines()
        .filter(|line| !line.starts_with("ratio-margin"))
        .collect::<Vec<_>>()
        .join("\n");
    expect(refused(&without_margin), "a budget with no ratio-margin")?;
    expect(
        refused(&format!("{BUDGET}dispatch-call linux 3\n")),
        "an unknown gate",
    )?;
    expect(
        refused(&format!("{BUDGET}control-call freebsd 3\n")),
        "an unknown platform",
    )?;
    expect(
        refused(&format!("{BUDGET}noise-margin 2.5\n")),
        "a noise margin without a platform",
    )?;
    expect(
        refused(&BUDGET.replace("ratio-margin 1.5", "ratio-margin 0")),
        "a margin that is not positive",
    )?;
    expect(
        refused(&BUDGET.replace("control-call linux 1.25", "control-call linux x")),
        "a number that is not one",
    )
}

fn the_noise_verdict_widens_to_a_noisy_run_and_refuses_a_noisier_one() -> Result<(), String> {
    let bounds = NoiseBounds {
        quiet: 0.01,
        worst: 0.2,
        margin: 3.0,
    };
    let close = |left: f64, right: f64| (left - right).abs() < 1e-9;
    expect(
        close(same_language_bound(bounds, 1.0), 1.03),
        "a quiet run is judged at the quiet noise",
    )?;
    expect(
        close(same_language_bound(bounds, 1.5), 2.5),
        "a noisy run widens to its own noise",
    )?;
    expect(
        close(same_language_bound(bounds, 1.0 / 1.5), 2.5),
        "either way",
    )?;
    expect(
        !too_noisy_to_judge(bounds, 1.0 + 0.2 * 0.99),
        "a run under the worst noise is judged",
    )?;
    expect(
        too_noisy_to_judge(bounds, 1.0 + 0.2 * 1.01),
        "a run over it is refused",
    )?;
    expect(
        too_noisy_to_judge(bounds, 1.0 / (1.0 + 0.2 * 1.01)),
        "either way",
    )
}

fn each_round_starts_one_path_later() -> Result<(), String> {
    let orders: Vec<Vec<usize>> = (0..4).map(|round| round_order(3, round)).collect();
    expect(
        orders == [vec![0, 1, 2], vec![1, 2, 0], vec![2, 0, 1], vec![0, 1, 2]],
        "rounds rotate which path goes first",
    )
}

fn the_kits_file_reads_when_it_is_there() -> Result<(), String> {
    let path = default_budget_path();
    if !path.is_file() {
        println!("weivel-benchmark: {} is not there yet", path.display());
        return Ok(());
    }
    weivel_rust::benchmarks::load_language_budget(&path)
        .map(|_| ())
        .map_err(|refusal| refusal.0)
}

/// The checks of the bounds.
pub fn checks() -> Vec<Check<'static>> {
    vec![
        (
            "the bounds are each gate's highest ratio times the margin",
            Box::new(bounds_are_the_highest_ratio_times_the_margin),
        ),
        (
            "a platform without a line fails its gate",
            Box::new(a_platform_without_a_line_fails_its_gate),
        ),
        (
            "a malformed budget is refused",
            Box::new(a_malformed_budget_is_refused),
        ),
        (
            "the noise verdict widens to a noisy run and refuses a noisier one",
            Box::new(the_noise_verdict_widens_to_a_noisy_run_and_refuses_a_noisier_one),
        ),
        (
            "each round starts one path later",
            Box::new(each_round_starts_one_path_later),
        ),
        (
            "the kit's file reads when it is there",
            Box::new(the_kits_file_reads_when_it_is_there),
        ),
    ]
}
