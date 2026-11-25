//! The `host` vectors, as the kit's `docs/vectors/layer/host.md` says: each run's plugins and `stub-host.json` laid out
//! in a fresh directory, the crate's own host program run over it in process with the kit's `stub-host` as its
//! provider, the stop request sent once the record holds `stopAfter`, and the record and the exit code answered.
//!
//! Every plugin's entry is the suite's test plugins library, one `cdylib` loaded across the plugin seam, whose entry
//! reads what its plugin does from the vector's own words for it, written beside the run: the suite writes the
//! standard's path and the run's directory to [`run_pointer_file`], which the plugin finds by the process it runs in,
//! so one run of the host vectors at a time is laid out in a process, as `stub-host` itself keeps one run at a time.

use std::env::consts::DLL_SUFFIX;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

use super::shapes::{self, DriverFailure, Members};
use crate::context::{HostLog, LogRecord};
use crate::host::Tree;
use crate::program::{RunHostOptions, StopRequest, run_host};
use crate::standard::Json;

/// The one bound the kit sets on every wait of a `host` vector.
pub const HOST_VECTOR_BOUND: Duration = Duration::from_secs(30);

/// The symbol the suite's test plugins library exports its host vector entry under.
pub const HOST_VECTOR_ENTRY_SYMBOL: &str = "host_vector_entry";

/// The directory of the run in which each plugin's words are written, as `<name>.json`.
pub const PLUGIN_WORDS_DIRECTORY: &str = "plugins";

const RECORD_FILE: &str = "record.txt";
const STOP_SENT_FILE: &str = "stop-sent";
const POLL: Duration = Duration::from_millis(5);
const STEP_KINDS: &[&str] = &[
    "log",
    "logSetting",
    "set",
    "absent",
    "waitFor",
    "pause",
    "throw",
    "provide",
];
const ACTION_KINDS: &[&str] = &["echo", "answer", "throw"];
const LONGEST_PAUSE: i64 = 10_000;
const CONSUMED_BY_DEFAULT: &[&str] = &[
    "weivel.config",
    "weivel.secrets",
    "weivel.paths",
    "weivel.log",
];

/// What a `host` vector's run is made of besides the vector: the `stub-host` library, with its declarations beside it,
/// the core and the standard the program loads, and the suite's test plugins library.
#[derive(Debug, Clone)]
pub struct HostProgramFixtures {
    /// A build of the kit's `stub-host`, its `library.json` and the kit's other stub-host declarations beside it.
    pub stub_host: PathBuf,
    /// The core's shared library.
    pub core: PathBuf,
    /// The standard's shared library.
    pub standard: PathBuf,
    /// The test plugins library, exporting [`HOST_VECTOR_ENTRY_SYMBOL`].
    pub plugins: PathBuf,
}

/// The file this process's test plugins read the run from: the standard's path, then the run's directory, a line each.
pub fn run_pointer_file() -> PathBuf {
    std::env::temp_dir().join(format!(
        "weivel-rust-host-vector-run-{}",
        std::process::id()
    ))
}

/// Taken for each run the pointer file names, so no two runs of this process lay one out at once.
static ONE_POINTED_RUN_AT_A_TIME: Mutex<()> = Mutex::new(());

/// The pointer file written for one run, which no other run of this process may take until it is dropped, when the
/// file is removed.
pub struct RunPointer {
    _turn: MutexGuard<'static, ()>,
}

impl Drop for RunPointer {
    fn drop(&mut self) {
        let _ = fs::remove_file(run_pointer_file());
    }
}

/// Points this process's test plugins at the run in `directory` over the standard at `standard`, waiting for any run
/// pointed at before to end.
pub fn point_runs_at(standard: &Path, directory: &Path) -> Result<RunPointer, DriverFailure> {
    let turn = ONE_POINTED_RUN_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    write_file(
        &run_pointer_file(),
        &format!("{}\n{}\n", standard.display(), directory.display()),
    )?;
    Ok(RunPointer { _turn: turn })
}

/// A log that keeps nothing: a vector judges the record, and what the program logs on the way is not part of it.
struct QuietLog;

impl HostLog for QuietLog {
    fn write(&self, _: &LogRecord) {}
}

fn tree_member<'a>(tree: &'a Tree, key: &str) -> Option<&'a Tree> {
    match tree {
        Tree::Record(members) => members
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value),
        _ => None,
    }
}

fn tree_text(tree: &Tree, key: &str) -> Option<String> {
    match tree_member(tree, key) {
        Some(Tree::Text(text)) => Some(text.clone()),
        _ => None,
    }
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
            control if (control as u32) < 0x20 => {
                written.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => written.push(other),
        }
    }
    written.push('"');
}

/// `tree` as compact JSON, each record's members in order, a number in the shortest form that reads back as itself.
pub(super) fn json_of(tree: &Tree) -> String {
    let mut written = String::new();
    write_json(tree, &mut written);
    written
}

fn write_json(tree: &Tree, written: &mut String) {
    match tree {
        Tree::Null => written.push_str("null"),
        Tree::Boolean(flag) => written.push_str(if *flag { "true" } else { "false" }),
        Tree::Number(number) => written.push_str(&number.to_string()),
        Tree::Text(text) => write_json_text(text, written),
        Tree::List(items) => {
            written.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    written.push(',');
                }
                write_json(item, written);
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
                write_json(value, written);
            }
            written.push('}');
        }
    }
}

fn write_file(path: &Path, text: &str) -> Result<(), DriverFailure> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)
            .map_err(|error| format!("{} could not be made: {error}", directory.display()))?;
    }
    fs::write(path, text)
        .map_err(|error| format!("{} could not be written: {error}", path.display()))
}

fn is_step_of_its_kind(step: &Members) -> bool {
    let Some(kind) = STEP_KINDS.iter().find(|kind| step.contains_key(**kind)) else {
        return false;
    };
    let expected: &[&str] = if *kind == "set" {
        &["set", "to"]
    } else {
        &[kind]
    };
    if step.len() != expected.len() || !expected.iter().all(|key| step.contains_key(*key)) {
        return false;
    }
    match *kind {
        "waitFor" | "provide" => step[*kind].as_text().is_some_and(|text| !text.is_empty()),
        "pause" => step[*kind].as_number().is_some_and(|millis| {
            millis.fract() == 0.0 && (1.0..=LONGEST_PAUSE as f64).contains(&millis)
        }),
        _ => true,
    }
}

fn is_action_of_its_kind(action: &Members) -> bool {
    let steps_hold_kinds = match action.get("steps") {
        None => true,
        Some(steps) => steps.as_array().is_some_and(|steps| {
            steps
                .iter()
                .all(|step| step.as_object().is_some_and(is_step_of_its_kind))
        }),
    };
    let kinds: Vec<&String> = action.keys().filter(|key| *key != "steps").collect();
    steps_hold_kinds
        && (kinds.is_empty() || (kinds.len() == 1 && ACTION_KINDS.contains(&kinds[0].as_str())))
}

/// Refuses a step or an action of a kind the suite's plugins do not realise, so a vector cannot pass unread.
fn refuse_unknown(name: &str, plugin: &Members) -> Result<(), DriverFailure> {
    let place = format!("the plugin {name}");
    if let Some(steps) = plugin.get("activate") {
        for step in shapes::objects(&place, plugin, "activate")? {
            if !is_step_of_its_kind(step) {
                return Err(format!(
                    "{place}'s step {steps:?} is of no kind the suite realises"
                ));
            }
        }
    }
    let mut named: Vec<(&str, &Members)> = Vec::new();
    if let Some(actions) = plugin.get("actions") {
        named.extend(
            shapes::object(&place, actions)?
                .iter()
                .filter_map(|(name, action)| {
                    action.as_object().map(|action| (name.as_str(), action))
                }),
        );
    }
    for key in ["provides", "implements"] {
        if let Some(ids) = plugin.get(key) {
            for methods in shapes::object(&place, ids)?.values() {
                named.extend(shapes::object(&place, methods)?.iter().filter_map(
                    |(name, method)| method.as_object().map(|method| (name.as_str(), method)),
                ));
            }
        }
    }
    match named
        .into_iter()
        .find(|(_, action)| !is_action_of_its_kind(action))
    {
        Some((action, _)) => Err(format!(
            "{place}'s {action} is of no kind the suite realises"
        )),
        None => Ok(()),
    }
}

fn keys_of(plugin: &Members, key: &str) -> Vec<Tree> {
    plugin
        .get(key)
        .and_then(Json::as_object)
        .map(|ids| ids.keys().map(|id| Tree::Text(id.clone())).collect())
        .unwrap_or_default()
}

/// Keys of a plugin's `provides` or `implements` in the order the vector writes them.
fn keys_in_order(plugin: &Tree, key: &str) -> Vec<Tree> {
    match tree_member(plugin, key) {
        Some(Tree::Record(ids)) => ids.iter().map(|(id, _)| Tree::Text(id.clone())).collect(),
        _ => Vec::new(),
    }
}

/// The manifest of the vector's plugin `name`: consuming the four standard ids unless it carries `consumes`,
/// providing the services of its `provides` and the capabilities of its `implements`.
fn manifest_of(name: &str, plugin: &Members, in_order: &Tree, entry: &str) -> Tree {
    let consumes = match plugin.get("consumes").and_then(Json::as_array) {
        Some(ids) => ids
            .iter()
            .filter_map(|id| id.as_text().map(|id| Tree::Text(id.to_owned())))
            .collect(),
        None => CONSUMED_BY_DEFAULT
            .iter()
            .map(|id| Tree::Text((*id).to_owned()))
            .collect(),
    };
    let mut services = vec![("consumes".to_owned(), Tree::List(consumes))];
    let provided = keys_in_order(in_order, "provides");
    if !provided.is_empty() {
        services.push(("provides".to_owned(), Tree::List(provided)));
    }
    let mut manifest = vec![
        ("api".to_owned(), Tree::Number(1.0)),
        ("importName".to_owned(), Tree::Text(name.to_owned())),
        ("entry".to_owned(), Tree::Text(entry.to_owned())),
        ("services".to_owned(), Tree::Record(services)),
    ];
    let implemented = keys_of(plugin, "implements");
    if !implemented.is_empty() {
        manifest.push((
            "capabilities".to_owned(),
            Tree::Record(vec![(
                "provides".to_owned(),
                Tree::List(keys_in_order(in_order, "implements")),
            )]),
        ));
    }
    Tree::Record(manifest)
}

/// What one run is laid out from.
struct Given<'a> {
    plugins: Vec<&'a Members>,
    plugins_in_order: Vec<&'a Tree>,
    stub_host: &'a Tree,
}

impl Given<'_> {
    fn plugin(&self, name: &str) -> Result<(&Members, &Tree), DriverFailure> {
        let index = self
            .plugins
            .iter()
            .position(|plugin| plugin.get("name").and_then(Json::as_text) == Some(name))
            .ok_or_else(|| format!("the vector gives no steps for the plugin {name}"))?;
        let in_order = self
            .plugins_in_order
            .get(index)
            .ok_or_else(|| format!("the vector's plugin {name} reads in two shapes"))?;
        Ok((self.plugins[index], in_order))
    }
}

fn library_of(library: &Tree, fixtures: &HostProgramFixtures) -> Tree {
    let Some(declaration) = tree_text(library, "stubHostDeclaration") else {
        return library.clone();
    };
    let beside = fixtures.stub_host.parent().unwrap_or(Path::new(""));
    Tree::Record(vec![
        (
            "binary".to_owned(),
            Tree::Text(fixtures.stub_host.to_string_lossy().into_owned()),
        ),
        (
            "declaration".to_owned(),
            Tree::Text(beside.join(declaration).to_string_lossy().into_owned()),
        ),
    ])
}

/// Lays the run out in `directory`: each plugin's manifest, unless its `unreadableManifest` is true, and its words,
/// then `stub-host.json` naming the test plugins library as every plugin's entry and each library, one naming a
/// stub-host declaration as `stub-host` under it.
fn lay_out(
    given: &Given<'_>,
    directory: &Path,
    fixtures: &HostProgramFixtures,
) -> Result<(), DriverFailure> {
    let entry_file = format!("weivel_test_plugins{DLL_SUFFIX}");
    let entry_path = Tree::Text(fixtures.plugins.to_string_lossy().into_owned());
    let Some(Tree::Record(stub_host)) = Some(given.stub_host) else {
        return Err("the vector's stubHost is not an object".to_owned());
    };
    let mut written = Vec::new();
    for (key, value) in stub_host {
        let rewritten = match (key.as_str(), value) {
            ("plugins", Tree::List(plugins)) => {
                let mut entries = Vec::new();
                for stub in plugins {
                    let name = tree_text(stub, "name").ok_or("a stub-host plugin has no name")?;
                    let (plugin, in_order) = given.plugin(&name)?;
                    refuse_unknown(&name, plugin)?;
                    if plugin.get("unreadableManifest").and_then(Json::as_bool) != Some(true) {
                        let manifest = tree_text(stub, "manifest")
                            .ok_or("a stub-host plugin has no manifest")?;
                        write_file(
                            &directory.join(manifest),
                            &json_of(&manifest_of(&name, plugin, in_order, &entry_file)),
                        )?;
                    }
                    write_file(
                        &directory
                            .join(PLUGIN_WORDS_DIRECTORY)
                            .join(format!("{name}.json")),
                        &json_of(in_order),
                    )?;
                    let Tree::Record(mut members) = stub.clone() else {
                        return Err(format!("the stub-host plugin {name} is not an object"));
                    };
                    members.push(("entry".to_owned(), entry_path.clone()));
                    entries.push(Tree::Record(members));
                }
                Tree::List(entries)
            }
            ("libraries", Tree::List(libraries)) => Tree::List(
                libraries
                    .iter()
                    .map(|library| library_of(library, fixtures))
                    .collect(),
            ),
            _ => value.clone(),
        };
        written.push((key.clone(), rewritten));
    }
    write_file(
        &directory.join("stub-host.json"),
        &json_of(&Tree::Record(written)),
    )
}

/// The complete lines of the record: a line counts once the newline after it is written.
fn record_lines(directory: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(directory.join(RECORD_FILE)) else {
        return Vec::new();
    };
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r').to_owned())
        .collect();
    lines.pop();
    lines
}

fn fresh_directory() -> Result<PathBuf, DriverFailure> {
    static RUNS: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!(
        "weivel-rust-host-vector-{}-{}",
        std::process::id(),
        RUNS.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("{} could not be made: {error}", directory.display()))?;
    Ok(directory)
}

/// Waits for the run's exit code within the bound, measured from `since`.
fn ended_within(
    exits: &mpsc::Receiver<u32>,
    since: Instant,
    what: &str,
) -> Result<u32, DriverFailure> {
    let left = HOST_VECTOR_BOUND.saturating_sub(since.elapsed());
    exits
        .recv_timeout(left)
        .map_err(|_| format!("the run did not end within {HOST_VECTOR_BOUND:?} of {what}"))
}

/// Sends the run its stop request once the record holds `stop_after`, then writes `stop-sent`, as the kit says, and
/// answers the exit code.
fn drive_to_the_end(
    directory: &Path,
    stop: &StopRequest,
    exits: &mpsc::Receiver<u32>,
    stop_after: Option<&str>,
) -> Result<u32, DriverFailure> {
    let started = Instant::now();
    let Some(line) = stop_after else {
        return ended_within(exits, started, "starting");
    };
    while !record_lines(directory)
        .iter()
        .any(|recorded| recorded == line)
    {
        if let Ok(exit) = exits.try_recv() {
            return Err(format!(
                "the run ended, exiting {exit}, before the record held {line:?}"
            ));
        }
        if started.elapsed() > HOST_VECTOR_BOUND {
            stop.request();
            let _ = ended_within(exits, Instant::now(), "its stop request");
            return Err(format!(
                "the record did not hold {line:?} within {HOST_VECTOR_BOUND:?} of the run starting"
            ));
        }
        std::thread::sleep(POLL);
    }
    if let Ok(exit) = exits.try_recv() {
        return Ok(exit);
    }
    stop.request();
    write_file(&directory.join(STOP_SENT_FILE), "")?;
    ended_within(exits, Instant::now(), "its stop request")
}

fn run_in(
    directory: &Path,
    arguments: Vec<String>,
    stop_after: Option<&str>,
    fixtures: &HostProgramFixtures,
) -> Result<u32, DriverFailure> {
    let _pointer = point_runs_at(&fixtures.standard, directory)?;
    let mut options = RunHostOptions::new(&fixtures.stub_host, &fixtures.core, &fixtures.standard);
    options.arguments = std::iter::once(directory.to_string_lossy().into_owned())
        .chain(arguments)
        .collect();
    options.log = Some(std::sync::Arc::new(QuietLog));
    options.entry_symbol = Some(HOST_VECTOR_ENTRY_SYMBOL.to_owned());
    let stop = options.stop.clone();
    let (exited, exits) = mpsc::channel();
    std::thread::Builder::new()
        .name("weivel-host-vector".to_owned())
        .spawn(move || {
            let _ = exited.send(run_host(options));
        })
        .map_err(|error| format!("the run's thread could not be started: {error}"))?;
    drive_to_the_end(directory, &stop, &exits, stop_after)
}

/// Runs one `host` vector and answers each field of its `then` from the run: `record` as `{equals: lines}`, `exit`
/// as the exit code, and `exitsNonZero` as whether it was not 0.
pub(super) fn witness(
    given: &Members,
    given_in_order: &Tree,
    then: &Members,
    fixtures: &HostProgramFixtures,
) -> Result<Json, DriverFailure> {
    let place = "the vector's given";
    let plugins_in_order = match tree_member(given_in_order, "plugins") {
        Some(Tree::List(plugins)) => plugins.iter().collect(),
        _ => Vec::new(),
    };
    let stub_host =
        tree_member(given_in_order, "stubHost").ok_or("the vector's given has no stubHost")?;
    let laid = Given {
        plugins: shapes::objects(place, given, "plugins")?,
        plugins_in_order,
        stub_host,
    };
    let arguments = match given.get("arguments") {
        Some(_) => shapes::texts(place, given, "arguments")?,
        None => Vec::new(),
    };
    let stop_after = match given.get("stopAfter") {
        Some(_) => Some(shapes::text(place, given, "stopAfter")?),
        None => None,
    };
    let directory = fresh_directory()?;
    let ran = lay_out(&laid, &directory, fixtures)
        .and_then(|()| run_in(&directory, arguments, stop_after.as_deref(), fixtures));
    let record = record_lines(&directory);
    let _ = fs::remove_dir_all(&directory);
    let exit = ran?;
    then.keys()
        .map(|key| {
            let observed = match key.as_str() {
                "record" => {
                    Json::Object([("equals".to_owned(), shapes::texts_json(record.clone()))].into())
                }
                "exit" => Json::Number(f64::from(exit)),
                "exitsNonZero" => Json::Bool(exit != 0),
                other => {
                    return Err(format!(
                        "the vector's then holds {other}, which a host run does not witness"
                    ));
                }
            };
            Ok((key.clone(), observed))
        })
        .collect::<Result<_, DriverFailure>>()
        .map(Json::Object)
}
