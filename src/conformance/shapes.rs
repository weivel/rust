//! Reading the parts of a vector, each part checked for its shape so a vector the driver cannot read fails rather
//! than passes unread. Every failure names where in the vector it was found.

use std::collections::BTreeMap;

use crate::standard::Json;

/// Why the driver could not drive a vector, or what the crate answered that no well-made step can have.
pub type DriverFailure = String;

pub(crate) type Members = BTreeMap<String, Json>;

pub(crate) fn object<'a>(place: &str, value: &'a Json) -> Result<&'a Members, DriverFailure> {
    value
        .as_object()
        .ok_or_else(|| format!("{place} is not an object"))
}

pub(crate) fn member<'a>(
    place: &str,
    members: &'a Members,
    key: &str,
) -> Result<&'a Json, DriverFailure> {
    members
        .get(key)
        .ok_or_else(|| format!("{place} has no {key}"))
}

pub(crate) fn object_at<'a>(
    place: &str,
    members: &'a Members,
    key: &str,
) -> Result<&'a Members, DriverFailure> {
    object(&format!("{place}'s {key}"), member(place, members, key)?)
}

pub(crate) fn text(place: &str, members: &Members, key: &str) -> Result<String, DriverFailure> {
    member(place, members, key)?
        .as_text()
        .map(str::to_owned)
        .ok_or_else(|| format!("{place}'s {key} is not a string"))
}

pub(crate) fn array<'a>(
    place: &str,
    members: &'a Members,
    key: &str,
) -> Result<&'a [Json], DriverFailure> {
    member(place, members, key)?
        .as_array()
        .ok_or_else(|| format!("{place}'s {key} is not an array"))
}

pub(crate) fn texts(
    place: &str,
    members: &Members,
    key: &str,
) -> Result<Vec<String>, DriverFailure> {
    array(place, members, key)?
        .iter()
        .map(|value| {
            value
                .as_text()
                .map(str::to_owned)
                .ok_or_else(|| format!("{place}'s {key} holds a value that is not a string"))
        })
        .collect()
}

pub(crate) fn objects<'a>(
    place: &str,
    members: &'a Members,
    key: &str,
) -> Result<Vec<&'a Members>, DriverFailure> {
    array(place, members, key)?
        .iter()
        .map(|value| object(&format!("{place}'s {key} element"), value))
        .collect()
}

/// A boolean member, `absent` when the member is absent.
pub(crate) fn flag(
    place: &str,
    members: &Members,
    key: &str,
    absent: bool,
) -> Result<bool, DriverFailure> {
    match members.get(key) {
        None => Ok(absent),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| format!("{place}'s {key} is not a boolean")),
    }
}

pub(crate) fn require_true(place: &str, members: &Members, key: &str) -> Result<(), DriverFailure> {
    if flag(place, members, key, false)? {
        Ok(())
    } else {
        Err(format!("{place}'s {key} is not true"))
    }
}

pub(crate) fn number(place: &str, members: &Members, key: &str) -> Result<i64, DriverFailure> {
    whole(place, key, member(place, members, key)?)
}

pub(crate) fn number_or_null(
    place: &str,
    members: &Members,
    key: &str,
) -> Result<Option<i64>, DriverFailure> {
    match members.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => whole(place, key, value).map(Some),
    }
}

fn whole(place: &str, key: &str, value: &Json) -> Result<i64, DriverFailure> {
    match value.as_number() {
        Some(number) if number.fract() == 0.0 => Ok(number as i64),
        _ => Err(format!("{place}'s {key} is not a whole number")),
    }
}

pub(crate) fn refuse_unknown_keys(
    place: &str,
    members: &Members,
    known: &[&str],
) -> Result<(), DriverFailure> {
    match members.keys().find(|key| !known.contains(&key.as_str())) {
        Some(unknown) => Err(format!(
            "{place} has {unknown}, which this driver does not read"
        )),
        None => Ok(()),
    }
}

/// The one key of `names` the object carries.
pub(crate) fn the_one_named(
    place: &str,
    members: &Members,
    names: &[&'static str],
) -> Result<&'static str, DriverFailure> {
    let present: Vec<&'static str> = names
        .iter()
        .copied()
        .filter(|name| members.contains_key(*name))
        .collect();
    match present.as_slice() {
        [one] => Ok(one),
        _ => Err(format!("{place} names none or several of {names:?}")),
    }
}

pub(crate) fn texts_json(values: impl IntoIterator<Item = String>) -> Json {
    Json::Array(values.into_iter().map(Json::Text).collect())
}
