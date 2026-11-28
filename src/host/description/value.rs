//! The values a bound definition takes and answers, and what a call answers.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use super::definition::BoundDefinition;
use super::holdings::{DescribedHandle, KeptAnswer};

pub use super::members::{Members, MembersIter};

/// An operation's inputs by parameter name.
pub type Inputs = BTreeMap<String, Value>;

/// One value of a described kind. A struct is [`Members`] keyed by member name, since no library ships Rust source
/// for its structs; a pointer is what it points at; a counted array, texts or bytes carry their own count, which the
/// binding writes.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `uint32_t`, an enum's value included.
    U32(u32),
    /// `uint64_t`.
    U64(u64),
    /// `int64_t`.
    I64(i64),
    /// `double`.
    F64(f64),
    /// `size_t`.
    Size(usize),
    /// `const char *`, or none for NULL.
    Text(Option<String>),
    /// `const char *const *` with its count, each text or none for NULL.
    Texts(Vec<Option<String>>),
    /// `const uint8_t *` with its count.
    Bytes(Vec<u8>),
    /// `const T *` with its count, each element of the array's element kind.
    Array(Vec<Value>),
    /// A struct, held by value or pointed at.
    Struct(Members),
    /// The standard's value tree, held by value or pointed at, or a VALUE answer.
    Tree(Tree),
    /// A value node of kind NONE, held by value or pointed at, which `value.h` calls no value: what a pointer to a
    /// value is handed when it must point at one holding nothing, as a host's `finished` is for an action that answered
    /// nothing.
    NoValue,
    /// An opaque object the provider made, released once when its last clone is dropped.
    Handle(DescribedHandle),
    /// A definition an answer carried, bound from the description nested in its carrier's.
    Definition(BoundDefinition),
}

/// The standard's value tree, `weivel/value.h`'s node as a Rust value. A record keeps its entries in the provider's
/// order.
#[derive(Debug, Clone, PartialEq)]
pub enum Tree {
    /// `null`.
    Null,
    /// A boolean.
    Boolean(bool),
    /// A finite number.
    Number(f64),
    /// A text.
    Text(String),
    /// A list.
    List(Vec<Tree>),
    /// A record, each key once.
    Record(Vec<(String, Tree)>),
}

impl Tree {
    /// The tree as compact JSON, each record's members in the order it holds them and a number in the shortest form
    /// that reads back as itself. A number that is not finite has no JSON form and is refused, as the first one met.
    pub fn to_json(&self) -> Result<String, NonFiniteNumber> {
        let mut written = String::new();
        write_json(self, &mut written)?;
        Ok(written)
    }
}

/// A number a [`Tree`] held that JSON cannot write: NaN or an infinity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NonFiniteNumber(pub f64);

impl fmt::Display for NonFiniteNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "the number {} has no JSON form", self.0)
    }
}

impl std::error::Error for NonFiniteNumber {}

fn write_json(tree: &Tree, written: &mut String) -> Result<(), NonFiniteNumber> {
    match tree {
        Tree::Null => written.push_str("null"),
        Tree::Boolean(flag) => written.push_str(if *flag { "true" } else { "false" }),
        Tree::Number(number) if number.is_finite() => written.push_str(&number.to_string()),
        Tree::Number(number) => return Err(NonFiniteNumber(*number)),
        Tree::Text(text) => write_json_text(text, written),
        Tree::List(items) => {
            written.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    written.push(',');
                }
                write_json(item, written)?;
            }
            written.push(']');
        }
        Tree::Record(members) => {
            written.push('{');
            for (index, (key, value)) in members.iter().enumerate() {
                if index > 0 {
                    written.push(',');
                }
                write_json_text(key, written);
                written.push(':');
                write_json(value, written)?;
            }
            written.push('}');
        }
    }
    Ok(())
}

fn write_json_text(text: &str, written: &mut String) {
    written.push('"');
    for character in text.chars() {
        match character {
            '"' => written.push_str("\\\""),
            '\\' => written.push_str("\\\\"),
            '\n' => written.push_str("\\n"),
            '\r' => written.push_str("\\r"),
            '\t' => written.push_str("\\t"),
            control if u32::from(control) < 0x20 => {
                written.push_str(&format!("\\u{:04x}", u32::from(control)));
            }
            other => written.push(other),
        }
    }
    written.push('"');
}

/// What a call answered on `WEIVEL_STATUS_OK` or `WEIVEL_STATUS_NONE`.
#[derive(Debug)]
pub enum DescribedAnswer {
    /// OK: the operation's answers.
    Answered(Answers),
    /// NONE: the operation has nothing to answer, which is neither an answer nor a refusal.
    None,
}

/// What an operation answered on OK, each answer by its parameter's name, a NULL pointer, a NULL handle and a value
/// of kind NONE left out. Every answer was copied and released before the call answered, but one carrying a
/// definition, which lives until [`Answers::release`], its library's unload, or the drop of its last part.
pub struct Answers {
    pub(crate) values: Members,
    pub(crate) only: Option<Arc<str>>,
    pub(crate) kept: Option<Arc<KeptAnswer>>,
}

impl fmt::Debug for Answers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Answers({:?})", self.values)
    }
}

impl Answers {
    /// Each answer by its parameter's name.
    pub fn values(&self) -> &Members {
        &self.values
    }

    /// The operation's one answer, as most operations have; none when it has none or several, or its one answer was
    /// left out.
    pub fn value(&self) -> Option<&Value> {
        self.values.get(self.only.as_deref()?)
    }

    /// Whether the answer carries a definition, and so lives until it is released.
    pub fn is_kept(&self) -> bool {
        self.kept.is_some()
    }

    /// Ends an answer that carries a definition: every call through a definition it carried is the caller's mistake
    /// from now on, and once the calls still running in them have returned, whatever was got through them is
    /// released, then the answer through the release its description names. A later call does nothing, and so does a
    /// call on an answer that carries no definition, which was released before its call answered.
    pub fn release(&self) {
        if let Some(kept) = &self.kept {
            kept.release();
        }
    }
}

/// Why a call answered neither OK nor NONE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStatus {
    /// The operation returned this `weivel_status`: MALFORMED, REFUSED, OUT_OF_MEMORY, or one the core does not
    /// publish.
    Returned(i32),
    /// The caller's mistake, refused before anything was called: an unknown operation or input, a value its kind
    /// does not hold, an operation its provider does not offer, or a definition whose answer has ended.
    CallersMistake,
    /// The library's binary has closed, so nothing was called.
    Unloaded,
    /// The provider answered what the binding cannot read; its answer was released all the same.
    ProviderFault,
}

/// A call that answered neither OK nor NONE: the operation, why, and for REFUSED the provider's own `detail` and
/// `fix`. For the caller's mistake and a provider's fault, `detail` says what was wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribedCallError {
    /// The operation called.
    pub operation: String,
    /// Why it answered neither OK nor NONE.
    pub status: CallStatus,
    /// Why, in the provider's words for REFUSED, in the binding's otherwise; none when nobody said.
    pub detail: Option<String>,
    /// What fixes it, in the provider's words; none when it gave none.
    pub fix: Option<String>,
}

impl DescribedCallError {
    pub(crate) fn mistake(operation: &str, detail: String) -> DescribedCallError {
        DescribedCallError {
            operation: operation.to_owned(),
            status: CallStatus::CallersMistake,
            detail: Some(detail),
            fix: None,
        }
    }

    pub(crate) fn fault(operation: &str, detail: String) -> DescribedCallError {
        DescribedCallError {
            operation: operation.to_owned(),
            status: CallStatus::ProviderFault,
            detail: Some(detail),
            fix: None,
        }
    }

    /// Whether the call was refused as the caller's mistake, calling nothing.
    pub fn is_callers_mistake(&self) -> bool {
        self.status == CallStatus::CallersMistake
    }
}

impl fmt::Display for DescribedCallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} answered {:?}", self.operation, self.status)?;
        if let Some(detail) = &self.detail {
            write!(formatter, ": {detail}")?;
        }
        if let Some(fix) = &self.fix {
            write!(formatter, " ({fix})")?;
        }
        Ok(())
    }
}

impl std::error::Error for DescribedCallError {}
