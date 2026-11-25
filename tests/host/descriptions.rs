//! Definitions bound from a description beyond what the layer vectors witness: enums read by name through a plugin's
//! context, a handle released by the drop of its last clone, a carried definition kept alive by itself, and calls the
//! binding refuses as the caller's mistake.

use std::sync::{Arc, MutexGuard};

use weivel_rust::context::{Object, address_of};
use weivel_rust::core::{Answer, EdgeSource};
use weivel_rust::host::{
    BoundDefinition, CallStatus, DescribedAnswer, Host, Inputs, LoadedLibrary, Members, Value,
};

use crate::libraries::{ONE_AT_A_TIME, described_declaration, described_host};
use crate::support::shared_library;
use crate::{Held, manifest};

struct Described {
    _turn: MutexGuard<'static, ()>,
    host: Host,
    library: LoadedLibrary,
}

impl Described {
    fn loaded() -> Described {
        let turn = ONE_AT_A_TIME
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let host = described_host();
        let library = host
            .load_library(
                &shared_library("weivel_standard_described"),
                &described_declaration("described_walker_description"),
            )
            .expect("the described library loads");
        Described {
            _turn: turn,
            host,
            library,
        }
    }

    fn service(&self, id: &str) -> Object {
        self.library
            .services()
            .into_iter()
            .find(|(service, _)| service == id)
            .map(|(_, object)| object)
            .unwrap_or_else(|| panic!("the library registered no {id}"))
    }

    fn bound(&self, id: &str) -> BoundDefinition {
        self.service(id)
            .downcast_ref::<BoundDefinition>()
            .cloned()
            .unwrap_or_else(|| panic!("{id} was not bound"))
    }

    fn cursors_released(&self) -> u64 {
        let answered = self
            .bound("described-mirror")
            .call("count_releases", &Inputs::new());
        let Ok(DescribedAnswer::Answered(answers)) = answered else {
            panic!("count_releases answered {answered:?}");
        };
        match answers.value() {
            Some(Value::Struct(counts)) => match counts.get("cursors") {
                Some(Value::U64(cursors)) => *cursors,
                other => panic!("count_releases counted cursors as {other:?}"),
            },
            other => panic!("count_releases answered {other:?}"),
        }
    }
}

impl Drop for Described {
    fn drop(&mut self) {
        self.host.destroy().wait();
    }
}

fn inputs(pairs: &[(&str, Value)]) -> Inputs {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

#[test]
fn a_plugin_reads_a_described_definitions_enums_by_name_through_its_context() {
    let described = Described::loaded();
    let plugin = described
        .host
        .register(
            manifest("reader", &[], &["described-mirror"]),
            Arc::new(Held::default()),
        )
        .expect("the reader registers");
    described
        .host
        .activate(&[plugin], EdgeSource::CAPABILITIES)
        .expect("the reader opens");
    let context = described
        .host
        .context(plugin)
        .expect("the reader has a context");
    let mirror = context
        .get("described-mirror")
        .expect("the reader gets the mirror");
    let enums = context
        .enums_of(&mirror)
        .expect("the mirror's description lists enums");
    assert_eq!(
        enums.number("described_shade", "DESCRIBED_SHADE_DARK"),
        Some(2)
    );
    assert_eq!(
        enums.number("described_tier", "DESCRIBED_TIER_HIGH"),
        Some(7)
    );
    assert_eq!(
        enums.number("weivel_status", "WEIVEL_STATUS_REFUSED"),
        Some(3)
    );
    let read = context
        .require_enums(
            Some(&enums),
            &[("described_shade", &["DESCRIBED_SHADE_LIGHT"])],
            "described-mirror",
        )
        .expect("the shade carries LIGHT");
    assert_eq!(read.names().collect::<Vec<_>>(), vec!["described_shade"]);
    assert_eq!(read.number("described_shade", "DESCRIBED_SHADE_DARK"), None);
    let missing = context.require_enums(
        Some(&enums),
        &[("described_shade", &["DESCRIBED_SHADE_GREY"])],
        "described-mirror",
    );
    assert!(
        matches!(&missing, Err(Answer::Mistake(text)) if text.contains("DESCRIBED_SHADE_GREY")),
        "{missing:?}"
    );
    let undescribed: Object = Arc::new(());
    assert_eq!(context.enums_of(&undescribed), None);
}

#[test]
fn a_bound_definition_crosses_as_the_address_its_symbol_answered() {
    let described = Described::loaded();
    let mirror = described.service("described-mirror");
    let bound = described.bound("described-mirror");
    assert_eq!(address_of(&mirror), bound.address());
    assert_eq!(bound.name(), "described_mirror");
}

#[test]
fn a_handle_is_released_once_by_the_drop_of_its_last_clone() {
    let described = Described::loaded();
    let walker = described.bound("described-walker");
    let opened = walker.call(
        "open",
        &inputs(&[
            ("label", Value::Text(Some("walk".to_owned()))),
            ("start", Value::U64(4)),
        ]),
    );
    let Ok(DescribedAnswer::Answered(answers)) = opened else {
        panic!("open answered {opened:?}");
    };
    let Some(Value::Handle(handle)) = answers.value().cloned() else {
        panic!("open answered {answers:?}");
    };
    let before = described.cursors_released();
    let clone = handle.clone();
    drop(answers);
    drop(handle);
    assert_eq!(described.cursors_released(), before);
    drop(clone);
    assert_eq!(described.cursors_released(), before + 1);
}

#[test]
fn a_released_handle_handed_back_is_the_callers_mistake() {
    let described = Described::loaded();
    let walker = described.bound("described-walker");
    let opened = walker.call(
        "open",
        &inputs(&[("label", Value::Text(Some("walk".to_owned())))]),
    );
    let Ok(DescribedAnswer::Answered(answers)) = opened else {
        panic!("open answered {opened:?}");
    };
    let Some(Value::Handle(handle)) = answers.value().cloned() else {
        panic!("open answered {answers:?}");
    };
    handle.release();
    let used = walker.call("use", &inputs(&[("cursor", Value::Handle(handle))]));
    assert!(
        used.as_ref().is_err_and(|error| error.is_callers_mistake()),
        "{used:?}"
    );
}

#[test]
fn a_value_its_kind_does_not_hold_is_the_callers_mistake() {
    let described = Described::loaded();
    let mirror = described.bound("described-mirror");
    let negated = mirror.call("negate", &inputs(&[("number", Value::U64(5))]));
    assert!(
        negated
            .as_ref()
            .is_err_and(|error| error.status == CallStatus::CallersMistake),
        "{negated:?}"
    );
    let text = mirror.call(
        "find",
        &inputs(&[("key", Value::Text(Some("al\0pha".to_owned())))]),
    );
    assert!(
        text.as_ref().is_err_and(|error| error.is_callers_mistake()),
        "{text:?}"
    );
    let unknown = mirror.call("fold", &Inputs::new());
    assert!(
        unknown
            .as_ref()
            .is_err_and(|error| error.is_callers_mistake()),
        "{unknown:?}"
    );
}

#[test]
fn a_carried_definition_outlives_the_answers_value_it_was_read_from() {
    let described = Described::loaded();
    let walker = described.bound("described-walker");
    let lent = walker.call("lend", &Inputs::new());
    let Ok(DescribedAnswer::Answered(answers)) = lent else {
        panic!("lend answered {lent:?}");
    };
    assert!(answers.is_kept());
    let Some(Value::Struct(loan)) = answers.value().cloned() else {
        panic!("lend answered {answers:?}");
    };
    let Some(Value::Definition(counter)) = loan.get("counter").cloned() else {
        panic!("the loan holds {loan:?}");
    };
    drop(loan);
    drop(answers);
    let added = counter.call("add", &inputs(&[("amount", Value::U64(2))]));
    let Ok(DescribedAnswer::Answered(total)) = added else {
        panic!("add answered {added:?}");
    };
    assert_eq!(total.value(), Some(&Value::U64(1002)));
    assert!(counter.blocking_operations().is_empty());
}

/// A node whose `next` reaches `links` nodes below it, each one pointer further down.
fn chain(links: usize) -> Value {
    let mut node = Value::Struct(Members::from([(
        "label",
        Value::Text(Some("last".to_owned())),
    )]));
    for _ in 0..links {
        node = Value::Struct(Members::from([
            ("label", Value::Text(Some("link".to_owned()))),
            ("next", node),
        ]));
    }
    node
}

/// How many nodes lie below `node` through `next`, and the `depth` the provider gave the deepest.
fn links_of(node: &Value) -> (usize, u32) {
    let mut links = 0;
    let mut current = node;
    while let Value::Struct(members) = current {
        match members.get("next") {
            Some(next) => {
                links += 1;
                current = next;
            }
            None => {
                let Some(Value::U32(depth)) = members.get("depth") else {
                    panic!("the last node carries no depth: {members:?}");
                };
                return (links, *depth);
            }
        }
    }
    panic!("an outline node is not a struct: {current:?}");
}

/// Runs `test` on a thread with the 2 MB stack a test thread gets, so a walk that recurses too deep overflows here.
fn on_a_two_megabyte_stack(test: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(test)
        .expect("the thread starts")
        .join()
        .expect("the walk finished on a 2 MB stack");
}

#[test]
fn structs_nested_past_the_limit_are_the_callers_mistake_without_exhausting_the_stack() {
    let described = Described::loaded();
    let mirror = described.bound("described-mirror");
    on_a_two_megabyte_stack(move || {
        let outlined = mirror.call("outline", &inputs(&[("node", chain(1001))]));
        assert!(
            outlined
                .as_ref()
                .is_err_and(|error| error.is_callers_mistake()),
            "{outlined:?}"
        );
    });
}

#[test]
fn structs_nested_to_the_limit_are_written_and_read_back_on_a_two_megabyte_stack() {
    let described = Described::loaded();
    let mirror = described.bound("described-mirror");
    on_a_two_megabyte_stack(move || {
        let outlined = mirror.call("outline", &inputs(&[("node", chain(999))]));
        let Ok(DescribedAnswer::Answered(answers)) = outlined else {
            panic!("outline answered {outlined:?}");
        };
        let Some(Value::Struct(copy)) = answers.value() else {
            panic!("outline answered {answers:?}");
        };
        let root = copy.get("root").expect("the copy holds its root");
        assert_eq!(links_of(root), (999, 999));
    });
}
