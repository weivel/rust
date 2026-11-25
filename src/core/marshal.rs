//! Laying out what a call hands the core as the core's structs, in memory that lives until the call returns.

use std::ffi::{CString, c_char};

use super::abi;
use super::answer::Answer;
use super::values::{ConstraintEntry, CoreOptions, Manifest, Number, ServiceOffer, ServiceWant};

/// Owns every text and array a call's structs point into; dropping it ends them, so it outlives the call.
#[derive(Default)]
pub(crate) struct Arena {
    texts: Vec<CString>,
    pointer_lists: Vec<Vec<*const c_char>>,
    number_lists: Vec<Vec<abi::Number>>,
    offer_lists: Vec<Vec<abi::ServiceOffer>>,
    want_lists: Vec<Vec<abi::ServiceWant>>,
    entry_lists: Vec<Vec<abi::ConstraintEntry>>,
}

fn nul_mistake(what: &str) -> Answer {
    Answer::Mistake(format!(
        "{what} holds a NUL character, which C would read cut short"
    ))
}

/// Refuses a text holding a NUL character, naming what it is, before anything reaches the core.
pub(crate) fn checked_text(what: &str, value: &str) -> Result<CString, Answer> {
    CString::new(value).map_err(|_| nul_mistake(what))
}

impl Arena {
    pub(crate) fn text(&mut self, what: &str, value: &str) -> Result<*const c_char, Answer> {
        let text = checked_text(what, value)?;
        let pointer = text.as_ptr();
        self.texts.push(text);
        Ok(pointer)
    }

    pub(crate) fn optional_text(
        &mut self,
        what: &str,
        value: Option<&str>,
    ) -> Result<*const c_char, Answer> {
        value.map_or(Ok(std::ptr::null()), |value| self.text(what, value))
    }

    pub(crate) fn strings(
        &mut self,
        what: &str,
        values: &[String],
    ) -> Result<abi::Strings, Answer> {
        let pointers = values
            .iter()
            .map(|value| self.text(what, value))
            .collect::<Result<Vec<_>, _>>()?;
        let strings = abi::Strings {
            items: list_pointer(&pointers),
            count: values.len() as u32,
        };
        self.pointer_lists.push(pointers);
        Ok(strings)
    }

    pub(crate) fn numbers(
        &mut self,
        what: &str,
        values: &[Number],
    ) -> Result<abi::Numbers, Answer> {
        let numbers = values
            .iter()
            .map(|number| {
                Ok(abi::Number {
                    size: abi::size_of::<abi::Number>(),
                    name: self.text(what, &number.name)?,
                    value: number.value,
                })
            })
            .collect::<Result<Vec<_>, Answer>>()?;
        let list = abi::Numbers {
            items: list_pointer(&numbers),
            count: values.len() as u32,
        };
        self.number_lists.push(numbers);
        Ok(list)
    }

    fn offers(&mut self, offers: &[ServiceOffer]) -> Result<abi::ServiceOffers, Answer> {
        let built = offers
            .iter()
            .map(|offer| {
                Ok(abi::ServiceOffer {
                    size: abi::size_of::<abi::ServiceOffer>(),
                    id: self.text("a provided service id", &offer.id)?,
                    attributes: self.numbers("an attribute name", &offer.attributes)?,
                })
            })
            .collect::<Result<Vec<_>, Answer>>()?;
        let list = abi::ServiceOffers {
            items: list_pointer(&built),
            count: offers.len() as u32,
        };
        self.offer_lists.push(built);
        Ok(list)
    }

    fn wants(&mut self, wants: &[ServiceWant]) -> Result<abi::ServiceWants, Answer> {
        let built = wants
            .iter()
            .map(|want| {
                Ok(abi::ServiceWant {
                    size: abi::size_of::<abi::ServiceWant>(),
                    id: self.text("a consumed service id", &want.id)?,
                    provider: self
                        .optional_text("a consumed service's provider", want.provider.as_deref())?,
                    optional: choice(want.optional),
                    degradable: choice(want.degradable),
                    constraints: self.numbers("a constraint name", &want.constraints)?,
                })
            })
            .collect::<Result<Vec<_>, Answer>>()?;
        let list = abi::ServiceWants {
            items: list_pointer(&built),
            count: wants.len() as u32,
        };
        self.want_lists.push(built);
        Ok(list)
    }

    fn vocabulary(
        &mut self,
        entries: &[ConstraintEntry],
    ) -> Result<abi::ConstraintVocabulary, Answer> {
        let built = entries
            .iter()
            .map(|entry| {
                Ok(abi::ConstraintEntry {
                    size: abi::size_of::<abi::ConstraintEntry>(),
                    constraint_name: self.text(
                        "a constraint name of the vocabulary",
                        &entry.constraint_name,
                    )?,
                    attribute_name: self
                        .text("an attribute name of the vocabulary", &entry.attribute_name)?,
                    bound: entry.bound.0,
                })
            })
            .collect::<Result<Vec<_>, Answer>>()?;
        let list = abi::ConstraintVocabulary {
            items: list_pointer(&built),
            count: entries.len() as u32,
        };
        self.entry_lists.push(built);
        Ok(list)
    }

    pub(crate) fn manifest(&mut self, manifest: &Manifest) -> Result<abi::Manifest, Answer> {
        Ok(abi::Manifest {
            size: abi::size_of::<abi::Manifest>(),
            import_name: self.text("the import name", &manifest.import_name)?,
            api: manifest.api,
            entry: self.optional_text("the entry", manifest.entry.as_deref())?,
            targets: self.strings("a target", &manifest.targets)?,
            capabilities: abi::CapabilityDeclarations {
                provides: self
                    .strings("a provided capability id", &manifest.capabilities_provided)?,
                consumes: self
                    .strings("a consumed capability id", &manifest.capabilities_consumed)?,
            },
            services: abi::ServiceDeclarations {
                provides: self.offers(&manifest.services_provided)?,
                consumes: self.wants(&manifest.services_consumed)?,
            },
            permissions: self.strings("a permission", &manifest.permissions)?,
            identity: self.optional_text("the identity", manifest.identity.as_deref())?,
        })
    }

    pub(crate) fn core_options(
        &mut self,
        options: &CoreOptions,
    ) -> Result<abi::CoreOptions, Answer> {
        Ok(abi::CoreOptions {
            size: abi::size_of::<abi::CoreOptions>(),
            initial_event_capacity: options.initial_event_capacity,
            minimum_log_level: options.minimum_log_level.0,
            api: options.api,
            applications: self.strings("an application", &options.applications)?,
            well_known_services: self
                .strings("a well-known service id", &options.well_known_services)?,
            constraint_vocabulary: self.vocabulary(&options.constraint_vocabulary)?,
        })
    }
}

/// A list's items, or NULL for an empty one, which the core reads only with a count of 0.
fn list_pointer<T>(items: &[T]) -> *const T {
    if items.is_empty() {
        std::ptr::null()
    } else {
        items.as_ptr()
    }
}

/// A yes-or-no field the caller may leave to the core's default.
pub(crate) fn choice(value: Option<bool>) -> u32 {
    match value {
        None => abi::CHOICE_DEFAULT,
        Some(false) => abi::CHOICE_NO,
        Some(true) => abi::CHOICE_YES,
    }
}
