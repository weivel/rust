//! The kit's bounds for this language, read from its `benchmarks/rust.txt` as its `docs/benchmarks.md` lays it out.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// A ratio gate the kit bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RatioGate {
    /// The control call through the wrapper over the same core function called bare.
    ControlCall,
    /// A C library function through the host's service over the same function called bare.
    LibraryCall,
    /// A described operation through the bound definition over the same operation through a hand binding.
    DescribedCall,
}

impl RatioGate {
    /// Every ratio gate this language carries.
    pub const ALL: [RatioGate; 3] = [
        RatioGate::ControlCall,
        RatioGate::LibraryCall,
        RatioGate::DescribedCall,
    ];

    /// The gate's name in the kit's file.
    pub fn kit_name(self) -> &'static str {
        match self {
            RatioGate::ControlCall => "control-call",
            RatioGate::LibraryCall => "library-call",
            RatioGate::DescribedCall => "described-call",
        }
    }

    fn of_kit_name(name: &str) -> Option<RatioGate> {
        RatioGate::ALL
            .into_iter()
            .find(|gate| gate.kit_name() == name)
    }
}

/// The kit's platforms, which are the names `std::env::consts::OS` gives them.
pub const KIT_PLATFORMS: [&str; 3] = ["linux", "windows", "macos"];

/// The platform the benchmarks run on, as the kit names it.
pub fn this_platform() -> &'static str {
    std::env::consts::OS
}

/// Each ratio gate's highest measured ratio per platform, the margin over it, and per platform the noise a pair of
/// identical cost is judged against and the margin over that noise.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LanguageBudget {
    /// Each gate's highest measured ratio, by platform.
    pub highest_measured_ratio: BTreeMap<(RatioGate, String), f64>,
    /// How far over its highest measured ratio a run may go.
    pub ratio_margin: f64,
    /// How far from 1 two identical paths are on a quiet run, by platform.
    pub quiet_noise: BTreeMap<String, f64>,
    /// The noisiest such deviation measured, times a margin, by platform.
    pub worst_noise: BTreeMap<String, f64>,
    /// How many times its noise a same-language pair may differ by, by platform.
    pub noise_margin: BTreeMap<String, f64>,
}

/// One platform's noise bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseBounds {
    /// How far apart two identical paths are on a quiet run.
    pub quiet: f64,
    /// The most a run's two identical paths may differ by before it is too noisy to judge.
    pub worst: f64,
    /// How many times its noise a same-language pair may lie from 1.
    pub margin: f64,
}

/// Why a budget file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetRefusal(pub String);

impl fmt::Display for BudgetRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BudgetRefusal {}

/// The kit's budget file of the commit the repository pins, where CI checks the kit out.
pub fn default_budget_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("siblings")
        .join("conformance")
        .join("benchmarks")
        .join("rust.txt")
}

fn positive_number(text: &str, line: &str) -> Result<f64, BudgetRefusal> {
    match text.parse::<f64>() {
        Ok(number) if number.is_finite() && number > 0.0 => Ok(number),
        _ => Err(BudgetRefusal(format!(
            "the budget line \"{line}\" carries {text}, which is not a positive number"
        ))),
    }
}

fn noise_row<'a>(
    budget: &'a mut LanguageBudget,
    name: &str,
) -> Option<&'a mut BTreeMap<String, f64>> {
    match name {
        "quiet-noise" => Some(&mut budget.quiet_noise),
        "worst-noise" => Some(&mut budget.worst_noise),
        "noise-margin" => Some(&mut budget.noise_margin),
        _ => None,
    }
}

fn read_line(budget: &mut LanguageBudget, line: &str) -> Result<(), BudgetRefusal> {
    let fields: Vec<&str> = line.split(' ').collect();
    let unknown = || {
        BudgetRefusal(format!(
            "the budget line \"{line}\" is neither a gate or noise bound on a kit platform nor one of the kit's margins"
        ))
    };
    match fields.as_slice() {
        ["ratio-margin", number] => budget.ratio_margin = positive_number(number, line)?,
        [name, platform, number] if KIT_PLATFORMS.contains(platform) => {
            let number = positive_number(number, line)?;
            if let Some(gate) = RatioGate::of_kit_name(name) {
                budget
                    .highest_measured_ratio
                    .insert((gate, (*platform).to_owned()), number);
            } else {
                let row = noise_row(budget, name).ok_or_else(unknown)?;
                row.insert((*platform).to_owned(), number);
            }
        }
        _ => return Err(unknown()),
    }
    Ok(())
}

/// Reads a budget file's text, refusing a line the format does not define, a number that is not positive, or a
/// missing ratio margin.
pub fn read_language_budget(text: &str) -> Result<LanguageBudget, BudgetRefusal> {
    let mut budget = LanguageBudget::default();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        read_line(&mut budget, line)?;
    }
    if budget.ratio_margin == 0.0 {
        return Err(BudgetRefusal("the budget names no ratio-margin".to_owned()));
    }
    Ok(budget)
}

/// Reads the budget file at `path`.
pub fn load_language_budget(path: &Path) -> Result<LanguageBudget, BudgetRefusal> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| BudgetRefusal(format!("{} cannot be read: {error}", path.display())))?;
    read_language_budget(&text)
}

impl LanguageBudget {
    /// The bound of `gate` on `platform`, or `None` for a platform the kit gives no line, which the gate fails.
    pub fn ratio_bound(&self, gate: RatioGate, platform: &str) -> Option<f64> {
        self.highest_measured_ratio
            .get(&(gate, platform.to_owned()))
            .map(|highest| highest * self.ratio_margin)
    }

    /// The noise bounds on `platform`, or `None` for a platform the kit gives no noise or noise-margin line.
    pub fn noise_bounds(&self, platform: &str) -> Option<NoiseBounds> {
        Some(NoiseBounds {
            quiet: *self.quiet_noise.get(platform)?,
            worst: *self.worst_noise.get(platform)?,
            margin: *self.noise_margin.get(platform)?,
        })
    }
}

/// How far `ratio` is from 1, the same either way: 1.1 and 1/1.1 are both 0.1 away.
pub fn deviation(ratio: f64) -> f64 {
    ratio.max(1.0 / ratio) - 1.0
}

/// Whether a run whose two identical direct paths differed by `noise_ratio` was noisier than any the kit measured.
pub fn too_noisy_to_judge(bounds: NoiseBounds, noise_ratio: f64) -> bool {
    deviation(noise_ratio) > bounds.worst
}

/// How far from 1 a same-language pair may be in a run whose two identical paths differed by `noise_ratio`: the
/// platform's margin times that run's own noise, or times the platform's quiet noise when the run was quieter.
pub fn same_language_bound(bounds: NoiseBounds, noise_ratio: f64) -> f64 {
    1.0 + bounds.margin * bounds.quiet.max(deviation(noise_ratio))
}

/// Whether `ratio` lies within `bound` of 1 either way.
pub fn within_ratio(ratio: f64, bound: f64) -> bool {
    ratio <= bound && ratio >= 1.0 / bound
}
