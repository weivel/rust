//! Answers a vector's `then` field by field from what a run shows, in the field's own shape, so the answer equals the
//! `then` exactly when the run witnessed the vector.

use std::collections::BTreeMap;

use super::shapes::{self, DriverFailure, Members};
use crate::core::Refusal;
use crate::standard::Json;

const EQUALS: &str = "equals";
const BEFORE: &str = "before";

/// What a run shows for one `then` field, handed the field's expectation for a matcher that names it.
pub(crate) type Observer<'a> = Box<dyn Fn(&Json) -> Result<Json, DriverFailure> + 'a>;

/// Every field of `then` as the run shows it; a field no observer answers fails, so nothing passes unread.
pub(crate) fn observe(
    vector: &str,
    then: &Members,
    observers: &BTreeMap<&'static str, Observer<'_>>,
) -> Result<Json, DriverFailure> {
    let mut observed = BTreeMap::new();
    for (field, expectation) in then {
        let observer = observers.get(field.as_str()).ok_or_else(|| {
            format!("the vector {vector} states then.{field}, which this driver does not witness")
        })?;
        let answer = observer(expectation)?;
        let shaped = if is_equals_matcher(expectation) {
            Json::Object(BTreeMap::from([(EQUALS.to_owned(), answer)]))
        } else {
            answer
        };
        observed.insert(field.clone(), shaped);
    }
    Ok(Json::Object(observed))
}

fn is_equals_matcher(expectation: &Json) -> bool {
    expectation
        .as_object()
        .is_some_and(|members| members.len() == 1 && members.contains_key(EQUALS))
}

/// The pairs a `before` matcher asks about, each one the trace keeps in that order answered as asked and any other
/// marked with the trace.
pub(crate) fn before(expectation: &Json, trace: &[String]) -> Result<Json, DriverFailure> {
    let place = "a trace expectation";
    let matcher = shapes::object(place, expectation)?;
    shapes::refuse_unknown_keys(place, matcher, &[BEFORE])?;
    let answered = shapes::array(place, matcher, BEFORE)?
        .iter()
        .map(|pair| pair_as_the_trace_has_it(pair, trace))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json::Object(BTreeMap::from([(
        BEFORE.to_owned(),
        Json::Array(answered),
    )])))
}

fn pair_as_the_trace_has_it(pair: &Json, trace: &[String]) -> Result<Json, DriverFailure> {
    let Some([Json::Text(earlier), Json::Text(later)]) = pair.as_array() else {
        return Err(format!("a before pair {pair:?} is no pair of texts"));
    };
    let earlier_at = trace.iter().position(|record| record == earlier);
    let later_at = trace.iter().position(|record| record == later);
    let mut answered = vec![Json::Text(earlier.clone()), Json::Text(later.clone())];
    if !matches!((earlier_at, later_at), (Some(first), Some(second)) if first < second) {
        answered.push(Json::Text(format!("not in that order in {trace:?}")));
    }
    Ok(Json::Array(answered))
}

/// The kit's name for a refusal code, the core's constant in lower case, such as `timed_out`.
pub(crate) fn code_name(refusal: &Refusal) -> String {
    refusal
        .code
        .name()
        .map_or_else(|| refusal.code.0.to_string(), str::to_lowercase)
}

/// `{code, subject, hasFix}`, the subject null when the refusal names none.
pub(crate) fn kit_refusal(refusal: &Refusal) -> Json {
    Json::Object(BTreeMap::from([
        ("code".to_owned(), Json::Text(code_name(refusal))),
        (
            "subject".to_owned(),
            refusal.subject.clone().map_or(Json::Null, Json::Text),
        ),
        ("hasFix".to_owned(), Json::Bool(refusal.has_fix)),
    ]))
}
