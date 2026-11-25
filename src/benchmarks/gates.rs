//! Each gate's verdict on what a benchmark measured: a ratio gate's per-round median ratio against its bound, and a
//! pair of identical cost against its platform's noise. No bound is never a pass.

use super::budget::{
    LanguageBudget, RatioGate, deviation, same_language_bound, too_noisy_to_judge, within_ratio,
};
use super::pairs::{PairSummaries, SameLanguageSummaries};

/// What a pair of identical cost measured: how far its two direct paths differed, and how far the path through
/// weivel lay from the first, each as a ratio of medians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseSample {
    /// The second direct path's median over the first's.
    pub noise: f64,
    /// The path through weivel's median over the first direct path's.
    pub ratio: f64,
}

/// The noise sample of three paths of identical cost.
pub fn noise_sample(summaries: &SameLanguageSummaries) -> NoiseSample {
    NoiseSample {
        noise: summaries.direct_again.median / summaries.direct.median,
        ratio: summaries.through_weivel.median / summaries.direct.median,
    }
}

/// The gate's sample of a ratio pair: the median of its per-round ratios.
pub fn ratio_of(summaries: &PairSummaries) -> f64 {
    summaries.round_ratio.median
}

/// Judges a ratio gate's sample on `platform`: within the kit's bound, or why not.
pub fn judge_ratio(
    budget: Option<&LanguageBudget>,
    gate: RatioGate,
    platform: &str,
    ratio: f64,
) -> Result<(), String> {
    let bound = budget
        .and_then(|budget| budget.ratio_bound(gate, platform))
        .ok_or_else(|| {
            format!(
                "the kit's budget gives {} no line on {platform}, so the gate has no bound and fails",
                gate.kit_name()
            )
        })?;
    if ratio <= bound {
        Ok(())
    } else {
        Err(format!(
            "{} measured {ratio:.4} on {platform}, over its bound {bound:.4}",
            gate.kit_name()
        ))
    }
}

/// Judges a pair of identical cost on `platform`: a run noisier than the kit's worst is refused as too noisy to
/// judge, and otherwise the path through weivel lies within the platform's margin of the run's own noise.
pub fn judge_noise(
    budget: Option<&LanguageBudget>,
    platform: &str,
    sample: NoiseSample,
) -> Result<(), String> {
    let bounds = budget
        .and_then(|budget| budget.noise_bounds(platform))
        .ok_or_else(|| {
            format!("the kit's budget gives no noise lines on {platform}, so the gate has no bound and fails")
        })?;
    if too_noisy_to_judge(bounds, sample.noise) {
        return Err(format!(
            "the run was too noisy to judge: its two identical direct paths differed by {:.6}, over the worst noise \
             {:.6}",
            deviation(sample.noise),
            bounds.worst
        ));
    }
    let bound = same_language_bound(bounds, sample.noise);
    if within_ratio(sample.ratio, bound) {
        Ok(())
    } else {
        Err(format!(
            "the path through weivel lay {:.6} from the direct path, outside the bound {:.6} the run's noise allows",
            deviation(sample.ratio),
            bound - 1.0
        ))
    }
}
