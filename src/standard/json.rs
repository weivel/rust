//! The standard's JSON reader, its functions resolved by name from the standard's binary and declared by hand.

use std::collections::BTreeMap;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use crate::core::LoadFailure;
use crate::core::binary::SharedLibrary;

const DETAIL_CAPACITY: usize = 96;

const KIND_BOOL: c_int = 1;
const KIND_NUMBER: c_int = 2;
const KIND_STRING: c_int = 3;
const KIND_ARRAY: c_int = 4;
const KIND_OBJECT: c_int = 5;

/// `weivel_json_error`, declared by hand from `json.h`.
#[repr(C)]
struct JsonError {
    size: u32,
    line: c_int,
    column: c_int,
    message: *const c_char,
    detail: [c_char; DETAIL_CAPACITY],
    out_of_memory: c_int,
}

type Value = *const c_void;

macro_rules! json_functions {
    ($($name:ident($($argument:ty),*) -> $answer:ty;)*) => {
        struct JsonFunctions {
            $($name: unsafe extern "C" fn($($argument),*) -> $answer,)*
        }

        impl JsonFunctions {
            fn resolve(library: &SharedLibrary) -> Result<JsonFunctions, &'static str> {
                Ok(JsonFunctions {
                    $($name: {
                        let symbol = concat!(stringify!($name), "\0");
                        let name = CStr::from_bytes_with_nul(symbol.as_bytes()).expect("a symbol ends in its NUL");
                        let address = library.symbol(name).ok_or(stringify!($name))?;
                        // SAFETY: `json.h` declares this symbol as a function of exactly this type, and a function's
                        // address fits an object pointer on every supported platform.
                        unsafe {
                            std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($argument),*) -> $answer>(
                                address,
                            )
                        }
                    },)*
                })
            }
        }
    };
}

json_functions! {
    weivel_json_parse(*const c_char, usize, *mut JsonError) -> *mut c_void;
    weivel_json_free(*mut c_void) -> ();
    weivel_json_kind_of(Value) -> c_int;
    weivel_json_object_size(Value) -> usize;
    weivel_json_object_key_at(Value, usize) -> *const c_char;
    weivel_json_object_value_at(Value, usize) -> Value;
    weivel_json_array_length(Value) -> usize;
    weivel_json_array_at(Value, usize) -> Value;
    weivel_json_as_string(Value) -> *const c_char;
    weivel_json_string_length(Value) -> usize;
    weivel_json_as_bool(Value, *mut c_int) -> c_int;
    weivel_json_as_double(Value, *mut f64) -> c_int;
}

/// A JSON value as the standard's reader read it. An object's members are keyed by name, so two objects are equal
/// whatever order their members were written in, which the standard's reader allows since it refuses a duplicate key.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number, read as a double.
    Number(f64),
    /// A string, decoded.
    Text(String),
    /// An array, in order.
    Array(Vec<Json>),
    /// An object, by member name.
    Object(BTreeMap<String, Json>),
}

impl Json {
    /// The member `key` of an object; `None` for another value or an absent member.
    pub fn member(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(members) => members.get(key),
            _ => None,
        }
    }

    /// The text of a string.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Json::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The elements of an array.
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(elements) => Some(elements),
            _ => None,
        }
    }

    /// The members of an object.
    pub fn as_object(&self) -> Option<&BTreeMap<String, Json>> {
        match self {
            Json::Object(members) => Some(members),
            _ => None,
        }
    }

    /// The value of a number.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Json::Number(number) => Some(*number),
            _ => None,
        }
    }

    /// The value of a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(flag) => Some(*flag),
            _ => None,
        }
    }
}

/// Why the standard's reader refused a text: where, and what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonRefusal {
    /// The line, from 1, counting bytes.
    pub line: i32,
    /// The column, from 1, counting bytes.
    pub column: i32,
    /// What the reader said.
    pub message: String,
    /// Whether memory ran out rather than the text being malformed.
    pub out_of_memory: bool,
}

impl fmt::Display for JsonRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "line {} column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for JsonRefusal {}

struct LoadedJson {
    functions: JsonFunctions,
    _library: SharedLibrary,
}

/// The standard's JSON reader, bound from the standard's binary its caller names; the binary is closed when the last
/// clone is gone.
#[derive(Clone)]
pub struct StandardJson {
    loaded: Arc<LoadedJson>,
}

impl fmt::Debug for StandardJson {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StandardJson")
    }
}

impl StandardJson {
    /// Opens the standard's shared binary at `path` and resolves the reader's functions.
    pub fn load(path: impl AsRef<Path>) -> Result<StandardJson, LoadFailure> {
        let library = SharedLibrary::open(path.as_ref()).ok_or(LoadFailure::NotALibrary)?;
        let functions = JsonFunctions::resolve(&library).map_err(LoadFailure::MissingFunction)?;
        Ok(StandardJson {
            loaded: Arc::new(LoadedJson {
                functions,
                _library: library,
            }),
        })
    }

    /// Reads one JSON document.
    pub fn parse(&self, text: &[u8]) -> Result<Json, JsonRefusal> {
        let functions = &self.loaded.functions;
        let mut error = JsonError {
            size: std::mem::size_of::<JsonError>() as u32,
            line: 0,
            column: 0,
            message: std::ptr::null(),
            detail: [0; DETAIL_CAPACITY],
            out_of_memory: 0,
        };
        // SAFETY: the text lives for the call, its length is given, and the error is the crate's own of its size.
        let root =
            unsafe { (functions.weivel_json_parse)(text.as_ptr().cast(), text.len(), &mut error) };
        if root.is_null() {
            return Err(refusal_of(&error));
        }
        // SAFETY: `root` is the document just parsed, read whole before it is freed below.
        let read = unsafe { self.value(root) };
        // SAFETY: `root` came from the parse and is freed exactly once.
        unsafe { (functions.weivel_json_free)(root) };
        Ok(read)
    }

    /// Reads one value of a parsed document.
    ///
    /// # Safety
    /// `value` is a value of a document not yet freed.
    unsafe fn value(&self, value: Value) -> Json {
        let functions = &self.loaded.functions;
        // SAFETY: as the caller promised, `value` lives, and each accessor reads only its own kind.
        unsafe {
            match (functions.weivel_json_kind_of)(value) {
                KIND_BOOL => {
                    let mut flag: c_int = 0;
                    (functions.weivel_json_as_bool)(value, &mut flag);
                    Json::Bool(flag != 0)
                }
                KIND_NUMBER => {
                    let mut number = 0f64;
                    (functions.weivel_json_as_double)(value, &mut number);
                    Json::Number(number)
                }
                KIND_STRING => Json::Text(self.text(value)),
                KIND_ARRAY => {
                    let length = (functions.weivel_json_array_length)(value);
                    Json::Array(
                        (0..length)
                            .map(|index| self.value((functions.weivel_json_array_at)(value, index)))
                            .collect(),
                    )
                }
                KIND_OBJECT => {
                    let size = (functions.weivel_json_object_size)(value);
                    Json::Object(
                        (0..size)
                            .map(|index| {
                                let key = (functions.weivel_json_object_key_at)(value, index);
                                let key = CStr::from_ptr(key).to_string_lossy().into_owned();
                                let member = (functions.weivel_json_object_value_at)(value, index);
                                (key, self.value(member))
                            })
                            .collect(),
                    )
                }
                _ => Json::Null,
            }
        }
    }

    /// # Safety
    /// `value` is a string value of a document not yet freed.
    unsafe fn text(&self, value: Value) -> String {
        let functions = &self.loaded.functions;
        // SAFETY: as the caller promised; the decoded bytes run for the length the reader gives.
        unsafe {
            let start = (functions.weivel_json_as_string)(value);
            let length = (functions.weivel_json_string_length)(value);
            let bytes = std::slice::from_raw_parts(start.cast::<u8>(), length);
            String::from_utf8_lossy(bytes).into_owned()
        }
    }
}

fn refusal_of(error: &JsonError) -> JsonRefusal {
    let message = if error.message.is_null() {
        "the reader gave no reason".to_owned()
    } else {
        // SAFETY: the message points at a literal or into the error's own detail, both NUL-terminated.
        unsafe { CStr::from_ptr(error.message) }
            .to_string_lossy()
            .into_owned()
    };
    JsonRefusal {
        line: error.line,
        column: error.column,
        message,
        out_of_memory: error.out_of_memory != 0,
    }
}
