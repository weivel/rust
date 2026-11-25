//! Typed access to the standard's services, each a wrapper over the definition its description bound: config,
//! secrets, paths and log of `services.h`, the scopes of `scope.h` and the host provider of `host.h`. A call the
//! provider declines answers the binding's [`DescribedCallError`], its `detail` and `fix` the provider's own.

use crate::context::Object;
use crate::host::{
    Answers, BoundDefinition, DescribedAnswer, DescribedCallError, Inputs, Members, Tree, Value,
};

const SETTING_TEXT: u32 = 1;
const SETTING_NUMBER: u32 = 2;
const SETTING_BOOLEAN: u32 = 3;

/// A log entry's level, `weivel_standard_log_level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// `WEIVEL_STANDARD_LOG_TRACE`.
    Trace = 1,
    /// `WEIVEL_STANDARD_LOG_DEBUG`.
    Debug = 2,
    /// `WEIVEL_STANDARD_LOG_INFO`.
    Info = 3,
    /// `WEIVEL_STANDARD_LOG_WARN`.
    Warn = 4,
    /// `WEIVEL_STANDARD_LOG_ERROR`.
    Error = 5,
}

/// A setting's value, one of the kinds `services.h` publishes; kind NONE is never a value, so a key with none reads
/// as `None`.
#[derive(Debug, Clone, PartialEq)]
pub enum Setting {
    /// `WEIVEL_STANDARD_SETTING_TEXT`.
    Text(String),
    /// `WEIVEL_STANDARD_SETTING_NUMBER`, finite.
    Number(f64),
    /// `WEIVEL_STANDARD_SETTING_BOOLEAN`.
    Boolean(bool),
}

fn inputs<const N: usize>(pairs: [(&str, Value); N]) -> Inputs {
    pairs
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn text(text: &str) -> Value {
    Value::Text(Some(text.to_owned()))
}

/// The answers of a call, none for one that answered NONE.
fn answered(answer: DescribedAnswer) -> Option<Answers> {
    match answer {
        DescribedAnswer::Answered(answers) => Some(answers),
        DescribedAnswer::None => None,
    }
}

fn member<'a>(members: &'a Members, name: &str) -> Option<&'a Value> {
    members.get(name)
}

fn text_member(members: &Members, name: &str) -> Option<String> {
    match member(members, name) {
        Some(Value::Text(text)) => text.clone(),
        _ => None,
    }
}

fn number_member(members: &Members, name: &str) -> Option<u64> {
    match member(members, name) {
        Some(Value::U32(number)) => Some(u64::from(*number)),
        Some(Value::U64(number)) => Some(*number),
        Some(Value::Size(number)) => Some(*number as u64),
        Some(Value::I64(number)) => u64::try_from(*number).ok(),
        _ => None,
    }
}

fn struct_answer(answers: &Answers, name: &str) -> Option<Members> {
    match answers.values().get(name) {
        Some(Value::Struct(members)) => Some(members.clone()),
        _ => None,
    }
}

fn fault(operation: &str, detail: &str) -> DescribedCallError {
    DescribedCallError {
        operation: operation.to_owned(),
        status: crate::host::CallStatus::ProviderFault,
        detail: Some(detail.to_owned()),
        fix: None,
    }
}

/// The definition `object` is, when a description bound it.
fn bound_of(object: &Object) -> Option<BoundDefinition> {
    object.downcast_ref::<BoundDefinition>().cloned()
}

fn setting_of(value: &Members) -> Option<Setting> {
    match number_member(value, "kind")? as u32 {
        SETTING_TEXT => Some(Setting::Text(
            text_member(value, "text").unwrap_or_default(),
        )),
        SETTING_NUMBER => match member(value, "number") {
            Some(Value::F64(number)) => Some(Setting::Number(*number)),
            _ => Some(Setting::Number(0.0)),
        },
        SETTING_BOOLEAN => Some(Setting::Boolean(number_member(value, "boolean") == Some(1))),
        _ => None,
    }
}

fn setting_value(setting: &Setting) -> Value {
    let members: Members = match setting {
        Setting::Text(text) => [
            ("kind".to_owned(), Value::U32(SETTING_TEXT)),
            ("text".to_owned(), Value::Text(Some(text.clone()))),
            ("text_length".to_owned(), Value::Size(text.len())),
        ]
        .into_iter()
        .collect(),
        Setting::Number(number) => [
            ("kind".to_owned(), Value::U32(SETTING_NUMBER)),
            ("number".to_owned(), Value::F64(*number)),
        ]
        .into_iter()
        .collect(),
        Setting::Boolean(boolean) => [
            ("kind".to_owned(), Value::U32(SETTING_BOOLEAN)),
            ("boolean".to_owned(), Value::U32(u32::from(*boolean))),
        ]
        .into_iter()
        .collect(),
    };
    Value::Struct(members)
}

/// The settings a host reads, `weivel_standard_config`.
#[derive(Debug, Clone)]
pub struct Config {
    bound: BoundDefinition,
}

impl Config {
    /// The config `object` is, when it is a definition a description bound.
    pub fn of(object: &Object) -> Option<Config> {
        bound_of(object).map(|bound| Config { bound })
    }

    /// One setting, `None` when the key is neither stored nor defaulted.
    pub fn get(&self, key: &str) -> Result<Option<Setting>, DescribedCallError> {
        let answer = self.bound.call("get", &inputs([("key", text(key))]))?;
        Ok(answered(answer)
            .and_then(|answers| struct_answer(&answers, "answer"))
            .and_then(|answer| match member(&answer, "value") {
                Some(Value::Struct(value)) => setting_of(value),
                _ => None,
            }))
    }

    /// Stores one setting, replacing any value it had.
    pub fn set(&self, key: &str, setting: &Setting) -> Result<(), DescribedCallError> {
        self.bound
            .call(
                "set",
                &inputs([("key", text(key)), ("value", setting_value(setting))]),
            )
            .map(|_| ())
    }

    /// Every setting, the provider's defaults merged with what the host stores, each key once, in the provider's
    /// order.
    pub fn all(&self) -> Result<Vec<(String, Setting)>, DescribedCallError> {
        let answer = self.bound.call("all", &Inputs::new())?;
        let settings = answered(answer)
            .and_then(|answers| struct_answer(&answers, "settings"))
            .unwrap_or_default();
        let listed = match member(&settings, "settings") {
            Some(Value::Array(listed)) => listed.clone(),
            _ => Vec::new(),
        };
        Ok(listed
            .iter()
            .filter_map(|setting| {
                let Value::Struct(setting) = setting else {
                    return None;
                };
                let key = text_member(setting, "key")?;
                match member(setting, "value") {
                    Some(Value::Struct(value)) => setting_of(value).map(|value| (key, value)),
                    _ => None,
                }
            })
            .collect())
    }

    /// The file `set` writes, the one a user edits by hand, as an absolute path; `None` when the provider keeps the
    /// settings in no file.
    pub fn settings_file(&self) -> Result<Option<String>, DescribedCallError> {
        let answer = self.bound.call("settings_file", &Inputs::new())?;
        Ok(answered(answer)
            .and_then(|answers| struct_answer(&answers, "file"))
            .and_then(|file| text_member(&file, "path")))
    }

    /// One setting whole, as a value tree, `None` when the key is neither stored nor defaulted.
    pub fn get_tree(&self, key: &str) -> Result<Option<Tree>, DescribedCallError> {
        let answer = self.bound.call("get_tree", &inputs([("key", text(key))]))?;
        Ok(
            answered(answer).and_then(|answers| match answers.values().get("tree") {
                Some(Value::Struct(tree)) => match member(tree, "root") {
                    Some(Value::Tree(root)) => Some(root.clone()),
                    _ => None,
                },
                Some(Value::Tree(root)) => Some(root.clone()),
                _ => None,
            }),
        )
    }
}

/// The secrets a host keeps, `weivel_standard_secrets`.
#[derive(Debug, Clone)]
pub struct Secrets {
    bound: BoundDefinition,
}

impl Secrets {
    /// The secrets `object` is, when it is a definition a description bound.
    pub fn of(object: &Object) -> Option<Secrets> {
        bound_of(object).map(|bound| Secrets { bound })
    }

    /// One secret, `None` when the name holds none.
    pub fn get(&self, name: &str) -> Result<Option<Vec<u8>>, DescribedCallError> {
        let answer = self.bound.call("get", &inputs([("name", text(name))]))?;
        Ok(answered(answer)
            .and_then(|answers| struct_answer(&answers, "secret"))
            .map(|secret| match member(&secret, "bytes") {
                Some(Value::Bytes(bytes)) => bytes.clone(),
                _ => Vec::new(),
            }))
    }

    /// Stores one secret, replacing any value it had.
    pub fn set(&self, name: &str, bytes: &[u8]) -> Result<(), DescribedCallError> {
        self.bound
            .call(
                "set",
                &inputs([
                    ("name", text(name)),
                    ("bytes", Value::Bytes(bytes.to_vec())),
                ]),
            )
            .map(|_| ())
    }

    /// Removes one secret; removing one that is not there succeeds.
    pub fn remove(&self, name: &str) -> Result<(), DescribedCallError> {
        self.bound
            .call("remove", &inputs([("name", text(name))]))
            .map(|_| ())
    }
}

/// The absolute directories a host stores files in, `weivel_standard_paths`.
#[derive(Debug, Clone)]
pub struct Paths {
    bound: BoundDefinition,
}

impl Paths {
    /// The paths `object` is, when it is a definition a description bound.
    pub fn of(object: &Object) -> Option<Paths> {
        bound_of(object).map(|bound| Paths { bound })
    }

    fn directory(&self, operation: &str) -> Result<String, DescribedCallError> {
        let answer = self.bound.call(operation, &Inputs::new())?;
        answered(answer)
            .and_then(|answers| match answers.value() {
                Some(Value::Text(Some(directory))) => Some(directory.clone()),
                _ => None,
            })
            .ok_or_else(|| fault(operation, "the provider answered no directory"))
    }

    /// Where files that can be rebuilt are kept.
    pub fn cache(&self) -> Result<String, DescribedCallError> {
        self.directory("cache")
    }

    /// Where settings files are kept.
    pub fn config(&self) -> Result<String, DescribedCallError> {
        self.directory("config")
    }

    /// The directory the host offers as the plugin's own.
    pub fn plugin(&self) -> Result<String, DescribedCallError> {
        self.directory("plugin")
    }
}

/// The log a host writes, `weivel_standard_log`.
#[derive(Debug, Clone)]
pub struct Log {
    bound: BoundDefinition,
}

impl Log {
    /// The log `object` is, when it is a definition a description bound.
    pub fn of(object: &Object) -> Option<Log> {
        bound_of(object).map(|bound| Log { bound })
    }

    /// Writes one entry at `level`, with `fields` as a record when there are any.
    pub fn write(
        &self,
        level: LogLevel,
        message: &str,
        fields: Option<&Tree>,
    ) -> Result<(), DescribedCallError> {
        let mut given = inputs([
            ("level", Value::U32(level as u32)),
            ("message", text(message)),
        ]);
        if let Some(fields) = fields {
            given.insert("fields".to_owned(), Value::Tree(fields.clone()));
        }
        self.bound.call("write", &given).map(|_| ())
    }
}

/// One plugin's own services, as the scopes' `open` answered them, each valid until [`Scope::release`].
#[derive(Debug)]
pub struct Scope {
    answers: Answers,
    /// Its config, when the provider offers the plugin one.
    pub config: Option<BoundDefinition>,
    /// Its secrets, when the provider offers the plugin any.
    pub secrets: Option<BoundDefinition>,
    /// Its paths, when the provider offers the plugin any.
    pub paths: Option<BoundDefinition>,
    /// Its log, when the provider offers the plugin one.
    pub log: Option<BoundDefinition>,
}

impl Scope {
    /// Releases the scope: every definition it answered ends, and the provider frees it.
    pub fn release(&self) {
        self.answers.release();
    }
}

fn definition_member(members: &Members, name: &str) -> Option<BoundDefinition> {
    match member(members, name) {
        Some(Value::Definition(bound)) => Some(bound.clone()),
        _ => None,
    }
}

/// The scopes a host opens for its plugins, `weivel_standard_scopes`.
#[derive(Debug, Clone)]
pub struct Scopes {
    bound: BoundDefinition,
}

impl Scopes {
    /// The scopes `bound` serves.
    pub fn of(bound: BoundDefinition) -> Scopes {
        Scopes { bound }
    }

    /// Opens the scope of the plugin whose manifest is at `manifest_path`; `None` for a provider that answers NONE.
    pub fn open(&self, manifest_path: &str) -> Result<Option<Scope>, DescribedCallError> {
        let answer = self
            .bound
            .call("open", &inputs([("manifest_path", text(manifest_path))]))?;
        Ok(answered(answer).map(|answers| {
            let members = struct_answer(&answers, "scope").unwrap_or_default();
            Scope {
                config: definition_member(&members, "config"),
                secrets: definition_member(&members, "secrets"),
                paths: definition_member(&members, "paths"),
                log: definition_member(&members, "log"),
                answers,
            }
        }))
    }
}

/// What a run does, `weivel_standard_host_mode`.
pub mod host_modes {
    /// `WEIVEL_STANDARD_HOST_SERVE`.
    pub const SERVE: u32 = 1;
    /// `WEIVEL_STANDARD_HOST_ACTION`.
    pub const ACTION: u32 = 2;
}

/// What became of one plugin, `weivel_standard_host_plugin_state`.
pub mod plugin_states {
    /// `WEIVEL_STANDARD_HOST_ACTIVATED`.
    pub const ACTIVATED: u32 = 1;
    /// `WEIVEL_STANDARD_HOST_REFUSED`.
    pub const REFUSED: u32 = 2;
    /// `WEIVEL_STANDARD_HOST_FAILED`.
    pub const FAILED: u32 = 3;
}

/// One library the plan loads, `weivel_standard_host_library`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedLibrary {
    /// The path of its binary.
    pub binary: String,
    /// The path of its declaration.
    pub declaration: String,
}

/// One plugin the plan activates, `weivel_standard_host_plugin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedPlugin {
    /// Its directory.
    pub directory: String,
    /// The path of its manifest.
    pub manifest: String,
    /// The path of its entry.
    pub entry: String,
}

/// A run as `start` answered it, `weivel_standard_host_plan`: a NULL text, and a member past the size the provider
/// answered, is `None`, and so is a value of kind NONE.
#[derive(Debug, Clone, PartialEq)]
pub struct HostPlan {
    /// A [`host_modes`] mode, or another number, which is malformed.
    pub mode: u32,
    /// The libraries, in order.
    pub libraries: Vec<PlannedLibrary>,
    /// The plugins, in order.
    pub plugins: Vec<PlannedPlugin>,
    /// The manifest path of the plugin whose action an ACTION run runs.
    pub action_plugin: Option<String>,
    /// The action of the entry an ACTION run runs.
    pub action_name: Option<String>,
    /// What the action is handed.
    pub action_arguments: Option<Tree>,
    /// The service whose method an ACTION run calls.
    pub action_service: Option<String>,
    /// The method of `action_service` an ACTION run calls.
    pub action_method: Option<String>,
}

/// What became of one plugin, as `started` is told it, `weivel_standard_host_outcome`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostOutcome {
    /// The plugin's manifest path, as the plan named it.
    pub manifest: String,
    /// A [`plugin_states`] state.
    pub state: u32,
    /// Why it was refused or failed, not empty when set.
    pub detail: Option<String>,
}

/// When to tick next, as `tick` answered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextTick {
    /// At these milliseconds since the Unix epoch.
    At(i64),
    /// Never again.
    Never,
    /// A `has_next` of neither 0 nor 1, which ends the run.
    Malformed(u32),
}

/// How an ACTION run's action ended, as `finished` is told it.
#[derive(Debug, Clone, PartialEq)]
pub enum ActionEnding {
    /// It answered this value tree, `None` for nothing, a value of kind NONE.
    Answered(Option<Tree>),
    /// It failed, saying why.
    Failed(String),
}

fn planned_library(value: &Value) -> Option<PlannedLibrary> {
    let Value::Struct(members) = value else {
        return None;
    };
    Some(PlannedLibrary {
        binary: text_member(members, "binary").unwrap_or_default(),
        declaration: text_member(members, "declaration").unwrap_or_default(),
    })
}

fn planned_plugin(value: &Value) -> Option<PlannedPlugin> {
    let Value::Struct(members) = value else {
        return None;
    };
    Some(PlannedPlugin {
        directory: text_member(members, "directory").unwrap_or_default(),
        manifest: text_member(members, "manifest").unwrap_or_default(),
        entry: text_member(members, "entry").unwrap_or_default(),
    })
}

fn array_member<T>(members: &Members, name: &str, each: fn(&Value) -> Option<T>) -> Vec<T> {
    match member(members, name) {
        Some(Value::Array(items)) => items.iter().filter_map(each).collect(),
        _ => Vec::new(),
    }
}

fn plan_of(members: &Members) -> HostPlan {
    HostPlan {
        mode: number_member(members, "mode").unwrap_or(0) as u32,
        libraries: array_member(members, "libraries", planned_library),
        plugins: array_member(members, "plugins", planned_plugin),
        action_plugin: text_member(members, "action_plugin"),
        action_name: text_member(members, "action_name"),
        action_arguments: match member(members, "action_arguments") {
            Some(Value::Tree(tree)) => Some(tree.clone()),
            _ => None,
        },
        action_service: text_member(members, "action_service"),
        action_method: text_member(members, "action_method"),
    }
}

fn outcome_value(outcome: &HostOutcome) -> Value {
    Value::Struct(
        [
            ("manifest".to_owned(), text(&outcome.manifest)),
            ("state".to_owned(), Value::U32(outcome.state)),
            ("detail".to_owned(), Value::Text(outcome.detail.clone())),
        ]
        .into_iter()
        .collect(),
    )
}

/// The provider a host program runs, `weivel_standard_host`.
#[derive(Debug, Clone)]
pub struct HostProvider {
    bound: BoundDefinition,
}

impl HostProvider {
    /// The provider `bound` serves.
    pub fn of(bound: BoundDefinition) -> HostProvider {
        HostProvider { bound }
    }

    /// Asks the provider what to run, handing it the program's own arguments.
    pub fn start(&self, arguments: &[String]) -> Result<HostPlan, DescribedCallError> {
        let given = inputs([(
            "arguments",
            Value::Texts(arguments.iter().cloned().map(Some).collect()),
        )]);
        let answer = self.bound.call("start", &given)?;
        answered(answer)
            .and_then(|answers| struct_answer(&answers, "plan"))
            .map(|plan| plan_of(&plan))
            .ok_or_else(|| fault("start", "the provider answered no plan"))
    }

    /// Tells the provider what became of each plugin of the plan, in its order.
    pub fn started(&self, outcomes: &[HostOutcome]) -> Result<(), DescribedCallError> {
        let given = inputs([(
            "outcomes",
            Value::Array(outcomes.iter().map(outcome_value).collect()),
        )]);
        self.bound.call("started", &given).map(|_| ())
    }

    /// Lets the provider act on the time, milliseconds since the Unix epoch, and asks when to tick next.
    pub fn tick(&self, now: i64) -> Result<NextTick, DescribedCallError> {
        let answer = self
            .bound
            .call("tick", &inputs([("now", Value::I64(now))]))?;
        let next = answered(answer)
            .and_then(|answers| struct_answer(&answers, "next"))
            .ok_or_else(|| fault("tick", "the provider answered no next tick"))?;
        let at = match member(&next, "next") {
            Some(Value::I64(at)) => *at,
            _ => 0,
        };
        Ok(match number_member(&next, "has_next").unwrap_or(0) {
            0 => NextTick::Never,
            1 => NextTick::At(at),
            other => NextTick::Malformed(other as u32),
        })
    }

    /// Tells the provider how the action ended.
    pub fn finished(&self, ending: &ActionEnding) -> Result<(), DescribedCallError> {
        let given = match ending {
            ActionEnding::Answered(result) => {
                inputs([("result", result.clone().map_or(Value::NoValue, Value::Tree))])
            }
            ActionEnding::Failed(failure) => inputs([("failure", text(failure))]),
        };
        self.bound.call("finished", &given).map(|_| ())
    }

    /// Ends the run, answering the code the program exits with.
    pub fn stop(&self) -> Result<u32, DescribedCallError> {
        let answer = self.bound.call("stop", &Inputs::new())?;
        answered(answer)
            .and_then(|answers| match answers.value() {
                Some(Value::U32(code)) => Some(*code),
                _ => None,
            })
            .ok_or_else(|| fault("stop", "the provider answered no exit code"))
    }
}
