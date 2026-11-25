//! Each benchmark: the call through weivel and the call it is measured against, sampled side by side.

use std::hint::black_box;
use std::sync::Arc;

use super::BenchmarkFailure;
use super::bare::{BareEcho, BareGetService};
use super::hand::{HandMirror, HandSample};
use super::measure::{
    Sampling, Slowed, Summary, sample_interleaved, summarise, summarise_round_ratios,
};
use super::world::{
    ARITHMETIC_ID, BenchmarkFiles, ECHO_DEFINITION_ID, ECHO_ID, MIRROR_ID, World, echo_declaration,
    echo_definition_declaration, mirror_declaration,
};
use crate::context::{ForeignObject, Object};
use crate::host::{
    BoundDefinition, DescribedAnswer, Inputs, LibraryAnswer, LibraryService,
    LibraryServiceFunction, Value,
};

/// How a benchmark samples, and which path, if any, it slows.
#[derive(Debug, Clone, Copy, Default)]
pub struct BenchmarkOptions {
    /// The sampling, or the benchmark's own default when `None`.
    pub sampling: Option<Sampling>,
    /// A path to slow, for a red proof.
    pub slowed: Option<Slowed>,
}

/// The samplings each benchmark uses unless told otherwise.
pub mod default_sampling {
    use super::Sampling;

    /// The same-language pair: three paths of one compiled call of about a nanosecond, in batches of a million.
    pub const SAME_LANGUAGE: Sampling = Sampling {
        warmup_calls: 1_000_000,
        calls_per_batch: 1_000_000,
        batches: 101,
    };
    /// The control call.
    pub const CONTROL_CALL: Sampling = Sampling {
        warmup_calls: 200_000,
        calls_per_batch: 1_000,
        batches: 201,
    };
    /// The library call.
    pub const LIBRARY_CALL: Sampling = Sampling {
        warmup_calls: 200_000,
        calls_per_batch: 1_000,
        batches: 201,
    };
    /// The library call's definition row, judged by noise as the same-language pair is.
    pub const DEFINITION_CALL: Sampling = Sampling {
        warmup_calls: 200_000,
        calls_per_batch: 20_000,
        batches: 101,
    };
    /// The described call.
    pub const DESCRIBED_CALL: Sampling = Sampling {
        warmup_calls: 20_000,
        calls_per_batch: 200,
        batches: 201,
    };
}

/// Three paths of identical cost: the call on the provider's own object twice, and on what the consumer resolved.
#[derive(Debug, Clone, Copy)]
pub struct SameLanguageSummaries {
    /// The call made on the provider's object directly.
    pub direct: Summary,
    /// The same direct call again, sampled beside the others: how far two identical paths differ is the noise.
    pub direct_again: Summary,
    /// The call made on the object the consumer resolved.
    pub through_weivel: Summary,
}

/// A call through weivel and the same call made past the layer.
#[derive(Debug, Clone, Copy)]
pub struct PairSummaries {
    /// The call made past the layer.
    pub bare: Summary,
    /// The call weivel makes.
    pub weivel: Summary,
    /// The weivel call's batch time over the bare call's in the same round, per round.
    pub round_ratio: Summary,
}

/// The described pair, and the echo library's bytes call through the host sampled in the same rounds beside them.
#[derive(Debug, Clone, Copy)]
pub struct DescribedCallSummaries {
    /// The pair.
    pub pair: PairSummaries,
    /// The echo call's time, for scale.
    pub echo: Summary,
}

/// The plain value a provider registers; the call measured is `add`.
#[derive(Debug)]
pub struct Arithmetic;

impl Arithmetic {
    /// Adds, wrapping.
    #[inline(never)]
    pub fn add(&self, left: u64, right: u64) -> u64 {
        left.wrapping_add(right)
    }
}

fn sampling(options: &BenchmarkOptions, default: Sampling) -> Sampling {
    options.sampling.unwrap_or(default)
}

fn failure(text: impl Into<String>) -> BenchmarkFailure {
    BenchmarkFailure(text.into())
}

fn pair(samples: &[Vec<f64>]) -> PairSummaries {
    PairSummaries {
        bare: summarise(&samples[0]),
        weivel: summarise(&samples[1]),
        round_ratio: summarise_round_ratios(&samples[1], &samples[0]),
    }
}

fn same_language(samples: &[Vec<f64>]) -> SameLanguageSummaries {
    SameLanguageSummaries {
        direct: summarise(&samples[0]),
        direct_again: summarise(&samples[1]),
        through_weivel: summarise(&samples[2]),
    }
}

/// Times a call on a service a consumer resolved through weivel against the same call made on the provider's object
/// directly.
///
/// The three paths run the same compiled call, so the timing can show only noise. What proves weivel puts nothing
/// between a consumer and a provider of the same language is the identity check before it: the object the consumer's
/// get resolved is the provider's own, the very allocation, or the benchmark fails.
pub fn benchmark_same_language_call(
    files: &BenchmarkFiles,
    options: &BenchmarkOptions,
) -> Result<SameLanguageSummaries, BenchmarkFailure> {
    let world = World::create(files, &[])?;
    let provided: Object = Arc::new(Arithmetic);
    world
        .provider
        .register(ARITHMETIC_ID, provided.clone())
        .map_err(|answer| failure(format!("the provider could not register: {answer:?}")))?;
    let resolved = world.resolved(ARITHMETIC_ID)?;
    if !Arc::ptr_eq(&resolved, &provided) {
        return Err(failure(
            "the consumer's get did not answer the provider's own object",
        ));
    }
    let target = provided
        .downcast_ref::<Arithmetic>()
        .ok_or_else(|| failure("the provider's object is no Arithmetic"))?;
    let through = resolved
        .downcast_ref::<Arithmetic>()
        .ok_or_else(|| failure("the object the consumer resolved is no Arithmetic"))?;
    let mut direct = |call: u64| {
        black_box(black_box(target).add(call, 1));
    };
    let mut direct_again = |call: u64| {
        black_box(black_box(target).add(call, 1));
    };
    let mut through_weivel = |call: u64| {
        black_box(black_box(through).add(call, 1));
    };
    let samples = sample_interleaved(
        &mut [
            ("direct", &mut direct),
            ("direct_again", &mut direct_again),
            ("through_weivel", &mut through_weivel),
        ],
        sampling(options, default_sampling::SAME_LANGUAGE),
        options.slowed,
    );
    Ok(same_language(&samples))
}

/// Times `weivel_get_service` through the wrapper, with the host's timer armed as a host arms it, against the same
/// function of the core's table called on the same core, plugin and id with nothing around it.
pub fn benchmark_control_call(
    files: &BenchmarkFiles,
    options: &BenchmarkOptions,
) -> Result<PairSummaries, BenchmarkFailure> {
    let world = World::create(files, &[])?;
    world
        .provider
        .register(ARITHMETIC_ID, Arc::new(Arithmetic))
        .map_err(|answer| failure(format!("the provider could not register: {answer:?}")))?;
    let core = world.host.core();
    let consumer = world.consumer.plugin();
    let mut bare_get = BareGetService::new(core, consumer, ARITHMETIC_ID)
        .ok_or_else(|| failure("the core was destroyed before the bare call was prepared"))?;
    let through = core
        .get_service(consumer, ARITHMETIC_ID)
        .map_err(|answer| failure(format!("weivel_get_service answered {answer:?}")))?;
    if bare_get.call() != Ok(through.0) {
        return Err(failure(
            "the bare weivel_get_service did not answer the wrapper's handle",
        ));
    }
    let mut bare = |_: u64| {
        black_box(
            bare_get
                .call()
                .expect("the bare weivel_get_service answered OK"),
        );
    };
    let mut weivel = |_: u64| {
        black_box(
            core.get_service(consumer, ARITHMETIC_ID)
                .expect("weivel_get_service answered OK"),
        );
    };
    let samples = sample_interleaved(
        &mut [("bare", &mut bare), ("weivel", &mut weivel)],
        sampling(options, default_sampling::CONTROL_CALL),
        options.slowed,
    );
    Ok(pair(&samples))
}

const ECHO_INPUT: [u8; 16] = [7; 16];

fn echo_service(world: &World) -> Result<Object, BenchmarkFailure> {
    let echo = world.resolved(ECHO_ID)?;
    if !echo.is::<LibraryService>() {
        return Err(failure("the echo service is no LibraryService"));
    }
    Ok(echo)
}

fn echo_through(echo: &LibraryServiceFunction<'_>, input: &[u8]) -> usize {
    match echo.call(input) {
        LibraryAnswer::Returned(bytes) => bytes.len(),
        other => panic!("the echo service answered {other:?}"),
    }
}

/// Times `echo_echo` through the host's bytes service, 16 bytes in and the same 16 back, against the same function
/// reached through the platform's loader and called directly, its answer released through `echo_release` as the
/// host releases it.
pub fn benchmark_library_call(
    files: &BenchmarkFiles,
    options: &BenchmarkOptions,
) -> Result<PairSummaries, BenchmarkFailure> {
    let world = World::create(files, &[(files.echo.clone(), echo_declaration())])?;
    let echo = echo_service(&world)?;
    let service = echo
        .downcast_ref::<LibraryService>()
        .and_then(|service| service.function("echo"))
        .ok_or_else(|| failure("the echo service offers no function echo"))?;
    let service = &service;
    let bare_echo = BareEcho::open(&files.echo)
        .ok_or_else(|| failure("the echo library did not open through the platform's loader"))?;
    if bare_echo.echo(&ECHO_INPUT) != (0, ECHO_INPUT.len())
        || echo_through(service, &ECHO_INPUT) != ECHO_INPUT.len()
    {
        return Err(failure("an echo did not answer the bytes it was handed"));
    }
    let mut bare = |_: u64| {
        black_box(bare_echo.echo(black_box(&ECHO_INPUT)));
    };
    let mut weivel = |_: u64| {
        black_box(echo_through(service, black_box(&ECHO_INPUT)));
    };
    let samples = sample_interleaved(
        &mut [("bare", &mut bare), ("weivel", &mut weivel)],
        sampling(options, default_sampling::LIBRARY_CALL),
        options.slowed,
    );
    Ok(pair(&samples))
}

/// Times the echo definition's `echo` called through the definition a consumer resolved against the same operation
/// called through the pointer `echo_service_definition` answered, 16 bytes in and the same 16 back.
///
/// The two are the same pointer, so the ratio is 1 by construction and bounds only the measurement's noise; it is
/// judged as the same-language pair is. Each path is handed the address itself and reads the operations out of the
/// struct once, before the timed loop. What proves the host adds no wrapper, count or copy is the identity check: the
/// address the consumer resolved is the address the symbol answered, or the benchmark fails.
pub fn benchmark_definition_call(
    files: &BenchmarkFiles,
    options: &BenchmarkOptions,
) -> Result<SameLanguageSummaries, BenchmarkFailure> {
    let world = World::create(
        files,
        &[(files.echo.clone(), echo_definition_declaration())],
    )?;
    let resolved = world.resolved(ECHO_DEFINITION_ID)?;
    let resolved_address = resolved
        .downcast_ref::<ForeignObject>()
        .map(ForeignObject::address)
        .ok_or_else(|| failure("the echo definition is no ForeignObject"))?;
    let bare_echo = BareEcho::open(&files.echo)
        .ok_or_else(|| failure("the echo library did not open through the platform's loader"))?;
    let answered = bare_echo
        .answered_address("echo_service_definition")
        .ok_or_else(|| failure("the echo library lacks echo_service_definition"))?;
    if resolved_address != answered {
        return Err(failure(
            "the definition the consumer resolved is not the pointer echo_service_definition answered",
        ));
    }
    let definition = |address: usize| {
        bare_echo
            .definition_at(address)
            .ok_or_else(|| failure("the echo definition is too small for its operations"))
    };
    let (first, second, through) = (
        definition(answered)?,
        definition(answered)?,
        definition(resolved_address)?,
    );
    let mut direct = |_: u64| {
        black_box(first.echo(black_box(&ECHO_INPUT)));
    };
    let mut direct_again = |_: u64| {
        black_box(second.echo(black_box(&ECHO_INPUT)));
    };
    let mut through_weivel = |_: u64| {
        black_box(through.echo(black_box(&ECHO_INPUT)));
    };
    let samples = sample_interleaved(
        &mut [
            ("direct", &mut direct),
            ("direct_again", &mut direct_again),
            ("through_weivel", &mut through_weivel),
        ],
        sampling(options, default_sampling::DEFINITION_CALL),
        options.slowed,
    );
    Ok(same_language(&samples))
}

const DESCRIBED_LABEL: &str = "request";
const DESCRIBED_TAGS: [&str; 3] = ["accept", "host", "content-length"];

fn reflect_inputs() -> Inputs {
    let sample = [
        (
            "label".to_owned(),
            Value::Text(Some(DESCRIBED_LABEL.to_owned())),
        ),
        (
            "tags".to_owned(),
            Value::Texts(
                DESCRIBED_TAGS
                    .iter()
                    .map(|tag| Some((*tag).to_owned()))
                    .collect(),
            ),
        ),
    ];
    Inputs::from([(
        "sample".to_owned(),
        Value::Struct(sample.into_iter().collect()),
    )])
}

fn bound_reflects(answer: &DescribedAnswer) -> bool {
    let DescribedAnswer::Answered(answers) = answer else {
        return false;
    };
    let Some(Value::Struct(reflection)) = answers.value() else {
        return false;
    };
    let Some(Value::Struct(sample)) = reflection.get("sample") else {
        return false;
    };
    let tags: Vec<Option<String>> = DESCRIBED_TAGS
        .iter()
        .map(|tag| Some((*tag).to_owned()))
        .collect();
    sample.get("label") == Some(&Value::Text(Some(DESCRIBED_LABEL.to_owned())))
        && sample.get("tags") == Some(&Value::Texts(tags))
}

fn hand_reflects(answer: &Result<HandSample, i32>) -> bool {
    answer.as_ref().is_ok_and(|sample| {
        sample.label.as_deref() == Some(DESCRIBED_LABEL)
            && sample
                .tags
                .iter()
                .map(Option::as_deref)
                .eq(DESCRIBED_TAGS.iter().map(|tag| Some(*tag)))
    })
}

/// Times the described mirror's `reflect`, a sample holding a text and a counted array of three texts in and its
/// reflection out, called on the bound definition a consumer resolved, against the same operation through a binding of
/// the fixture's structs written by hand, every member each answers copied out into Rust values and the reflection
/// and the refusal released after every call. The echo library's bytes call through the host is sampled in the same
/// rounds, for its time beside theirs.
pub fn benchmark_described_call(
    files: &BenchmarkFiles,
    options: &BenchmarkOptions,
) -> Result<DescribedCallSummaries, BenchmarkFailure> {
    let world = World::create(
        files,
        &[
            (files.echo.clone(), echo_declaration()),
            (files.described.clone(), mirror_declaration()),
        ],
    )?;
    let echo = echo_service(&world)?;
    let service = echo
        .downcast_ref::<LibraryService>()
        .and_then(|service| service.function("echo"))
        .ok_or_else(|| failure("the echo service offers no function echo"))?;
    let service = &service;
    let mirror_object = world.resolved(MIRROR_ID)?;
    let mirror = mirror_object
        .downcast_ref::<BoundDefinition>()
        .ok_or_else(|| failure("the described mirror is no BoundDefinition"))?;
    let mut hand = HandMirror::open(&files.described, DESCRIBED_LABEL, &DESCRIBED_TAGS)
        .ok_or_else(|| {
            failure("the described library did not open for a hand binding of reflect")
        })?;
    let inputs = reflect_inputs();
    let bound = mirror
        .call("reflect", &inputs)
        .map_err(|error| failure(format!("the bound reflect failed: {error:?}")))?;
    if !bound_reflects(&bound) || !hand_reflects(&hand.reflect()) {
        return Err(failure("a reflect did not answer the sample it was handed"));
    }
    let mut bare = |_: u64| {
        black_box(hand.reflect().expect("the hand reflect answered OK"));
    };
    let mut weivel = |_: u64| {
        black_box(
            mirror
                .call("reflect", black_box(&inputs))
                .expect("the bound reflect answered OK"),
        );
    };
    let mut echo_call = |_: u64| {
        black_box(echo_through(service, black_box(&ECHO_INPUT)));
    };
    let samples = sample_interleaved(
        &mut [
            ("bare", &mut bare),
            ("weivel", &mut weivel),
            ("echo", &mut echo_call),
        ],
        sampling(options, default_sampling::DESCRIBED_CALL),
        options.slowed,
    );
    Ok(DescribedCallSummaries {
        pair: pair(&samples[..2]),
        echo: summarise(&samples[2]),
    })
}
