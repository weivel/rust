//! What a `host` vector's plugin does, realised from the vector's words: the steps of its activation, the actions it
//! offers, the methods of the services it provides and the operations of the capabilities it implements, each through
//! the services the program offers it, bound by their descriptions, as the kit's `docs/vectors/layer/host.md` says.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use weivel_rust::conformance::{HOST_VECTOR_BOUND, PLUGIN_WORDS_DIRECTORY, run_pointer_file};
use weivel_rust::context::Context;
use weivel_rust::core::Answer;
use weivel_rust::host::{MethodResult, Methods, Tree};
use weivel_rust::standard::{Config, Json, Log, LogLevel, Setting, StandardJson};

const POLL: Duration = Duration::from_millis(5);

type Words = BTreeMap<String, Json>;

/// One plugin of a run: its context, the run's directory and its words.
pub struct Run {
    context: Context,
    directory: PathBuf,
    words: Words,
}

fn tree_of(json: &Json) -> Tree {
    match json {
        Json::Null => Tree::Null,
        Json::Bool(flag) => Tree::Boolean(*flag),
        Json::Number(number) => Tree::Number(*number),
        Json::Text(text) => Tree::Text(text.clone()),
        Json::Array(items) => Tree::List(items.iter().map(tree_of).collect()),
        Json::Object(members) => Tree::Record(
            members
                .iter()
                .map(|(key, value)| (key.clone(), tree_of(value)))
                .collect(),
        ),
    }
}

fn setting_of(json: &Json) -> Result<Setting, String> {
    match json {
        Json::Text(text) => Ok(Setting::Text(text.clone())),
        Json::Number(number) => Ok(Setting::Number(*number)),
        Json::Bool(flag) => Ok(Setting::Boolean(*flag)),
        other => Err(format!("{other:?} is no setting")),
    }
}

fn setting_text(setting: Option<Setting>) -> String {
    match setting {
        None => "none".to_owned(),
        Some(Setting::Text(text)) => text,
        Some(Setting::Number(number)) => number.to_string(),
        Some(Setting::Boolean(flag)) => flag.to_string(),
    }
}

fn objects(json: Option<&Json>) -> Vec<Words> {
    json.and_then(Json::as_array)
        .map(|items| items.iter().filter_map(Json::as_object).cloned().collect())
        .unwrap_or_default()
}

fn named(json: Option<&Json>) -> Vec<(String, Words)> {
    json.and_then(Json::as_object)
        .map(|members| {
            members
                .iter()
                .filter_map(|(name, value)| {
                    value.as_object().map(|words| (name.clone(), words.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Run {
    /// The run of `context`'s plugin, its words read from the run the pointer file names.
    pub fn of(context: &Context) -> Result<Run, String> {
        let pointer = std::fs::read_to_string(run_pointer_file())
            .map_err(|error| format!("the run pointer file could not be read: {error}"))?;
        let mut lines = pointer.lines();
        let (Some(standard), Some(directory)) = (lines.next(), lines.next()) else {
            return Err("the run pointer file names no standard and no run".to_owned());
        };
        let json = StandardJson::load(standard)
            .map_err(|failure| format!("the standard did not load: {failure}"))?;
        let name = &context.manifest().import_name;
        let path = PathBuf::from(directory)
            .join(PLUGIN_WORDS_DIRECTORY)
            .join(format!("{name}.json"));
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
        let words = json
            .parse(&bytes)
            .map_err(|refusal| format!("{}: {refusal}", path.display()))?
            .as_object()
            .cloned()
            .ok_or_else(|| format!("{} holds no object", path.display()))?;
        Ok(Run {
            context: context.clone(),
            directory: PathBuf::from(directory),
            words,
        })
    }

    /// Runs the activation's steps in order.
    pub fn activate(&self) -> Result<(), String> {
        objects(self.words.get("activate"))
            .iter()
            .try_for_each(|step| self.step(step))
    }

    /// Each service the plugin provides, with its methods.
    pub fn provided(self: &Arc<Run>) -> Vec<(String, Methods)> {
        named(self.words.get("provides"))
            .into_iter()
            .map(|(id, methods)| (id, self.methods_of(&methods)))
            .collect()
    }

    /// The actions the plugin's entry offers.
    pub fn actions(self: &Arc<Run>) -> Methods {
        let words: Words = self
            .words
            .get("actions")
            .and_then(Json::as_object)
            .cloned()
            .unwrap_or_default();
        self.methods_of(&words)
    }

    fn methods_of(self: &Arc<Run>, methods: &Words) -> Methods {
        methods
            .iter()
            .fold(Methods::new(), |offered, (name, action)| {
                let run = self.clone();
                let action = action.as_object().cloned().unwrap_or_default();
                offered.with(name.clone(), move |arguments| run.act(&action, arguments))
            })
    }

    fn act(&self, action: &Words, arguments: Option<&Tree>) -> MethodResult {
        for step in objects(action.get("steps")) {
            self.step(&step)?;
        }
        if action.contains_key("echo") {
            return Ok(arguments.cloned());
        }
        if let Some(answer) = action.get("answer") {
            return Ok(Some(tree_of(answer)));
        }
        if let Some(thrown) = action.get("throw") {
            return Err(thrown.as_text().unwrap_or_default().to_owned());
        }
        Ok(None)
    }

    fn log(&self) -> Result<Log, String> {
        let object = self
            .context
            .get("weivel.log")
            .map_err(|answer| format!("weivel.log answered nothing: {answer}"))?;
        Log::of(&object).ok_or_else(|| "weivel.log is no described definition".to_owned())
    }

    fn info(&self, message: &str) -> Result<(), String> {
        self.log()?
            .write(LogLevel::Info, message, None)
            .map_err(|refused| refused.to_string())
    }

    fn config(&self) -> Result<Config, String> {
        let object = self
            .context
            .get("weivel.config")
            .map_err(|answer| format!("weivel.config answered nothing: {answer}"))?;
        Config::of(&object).ok_or_else(|| "weivel.config is no described definition".to_owned())
    }

    fn step(&self, step: &Words) -> Result<(), String> {
        let text = |key: &str| {
            step.get(key)
                .and_then(Json::as_text)
                .unwrap_or_default()
                .to_owned()
        };
        if step.contains_key("log") {
            self.info(&text("log"))
        } else if step.contains_key("logSetting") {
            let key = text("logSetting");
            let setting = self
                .config()?
                .get(&key)
                .map_err(|refused| refused.to_string())?;
            self.info(&format!("{key}={}", setting_text(setting)))
        } else if step.contains_key("set") {
            let setting = setting_of(step.get("to").unwrap_or(&Json::Null))?;
            self.config()?
                .set(&text("set"), &setting)
                .map_err(|refused| refused.to_string())
        } else if step.contains_key("absent") {
            match self.context.get(&text("absent")) {
                Err(Answer::Absent) => Ok(()),
                Ok(_) => Err(format!("{} answered this plugin something", text("absent"))),
                Err(other) => Err(format!("{} was refused: {other}", text("absent"))),
            }
        } else if step.contains_key("waitFor") {
            self.wait_for(&text("waitFor"))
        } else if let Some(millis) = step.get("pause").and_then(Json::as_number) {
            std::thread::sleep(Duration::from_millis(millis as u64));
            Ok(())
        } else if step.contains_key("provide") {
            self.provide(&text("provide"))
        } else {
            Err(text("throw"))
        }
    }

    fn wait_for(&self, file_name: &str) -> Result<(), String> {
        let path = self.directory.join(file_name);
        let started = Instant::now();
        while !path.exists() {
            if started.elapsed() > HOST_VECTOR_BOUND {
                return Err(format!(
                    "{file_name} did not appear within {HOST_VECTOR_BOUND:?}"
                ));
            }
            std::thread::sleep(POLL);
        }
        Ok(())
    }

    /// Provides the implementation of `id` its `implements` names, writing how the provide came out; it never fails.
    fn provide(&self, id: &str) -> Result<(), String> {
        let operations = named(self.words.get("implements"))
            .into_iter()
            .find(|(implemented, _)| implemented == id)
            .map(|(_, operations)| operations)
            .unwrap_or_default();
        let implementation = operations.keys().fold(Methods::new(), |offered, name| {
            offered.with(name.clone(), |_| Ok(None))
        });
        match self.context.provide(id, Arc::new(implementation)) {
            Ok(()) => self.info(&format!("provided {id}")),
            Err(Answer::ContractMismatch { operation, .. }) => self.info(&format!(
                "refused {} {operation}",
                Answer::CONTRACT_MISMATCH
            )),
            Err(other) => self.info(&format!("refused {other}")),
        }
    }
}
