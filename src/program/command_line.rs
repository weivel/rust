//! The host program's command line, `weivel-host [--core <path>] [--standard <path>] <host library>
//! [arguments...]`.

use std::env::consts::{DLL_PREFIX, DLL_SUFFIX};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::options::{FAILED_RUN_EXIT, RunHostOptions};
use super::run::run_host;
use super::signals::forward_terminate_signals;
use crate::context::{HostLog, LogRecord};

const USAGE: &str = "usage: weivel-host [--core <core library>] [--standard <standard library>] <host library> \
                     [arguments...]";

/// Writes every record to standard error.
struct StandardErrorLog;

impl HostLog for StandardErrorLog {
    fn write(&self, record: &LogRecord) {
        match &record.plugin {
            Some(plugin) => eprintln!("weivel-host: {plugin}: {}", record.text),
            None => eprintln!("weivel-host: {}", record.text),
        }
    }
}

/// A shared library named as CMake names `name`, beside the running program.
fn beside_the_program(name: &str) -> PathBuf {
    let file = format!("{DLL_PREFIX}{name}{DLL_SUFFIX}");
    std::env::current_exe()
        .ok()
        .and_then(|program| program.parent().map(|directory| directory.join(&file)))
        .unwrap_or_else(|| Path::new(&file).to_owned())
}

/// The command line read into a run's options, or why it cannot be.
fn options_of(command_line: Vec<OsString>) -> Result<RunHostOptions, String> {
    let mut core = None;
    let mut standard = None;
    let mut words = command_line.into_iter();
    let host_library = loop {
        let word = words.next().ok_or_else(|| USAGE.to_owned())?;
        match word.to_str() {
            Some("--core") => {
                core = Some(PathBuf::from(words.next().ok_or_else(|| USAGE.to_owned())?))
            }
            Some("--standard") => {
                standard = Some(PathBuf::from(words.next().ok_or_else(|| USAGE.to_owned())?));
            }
            _ => break PathBuf::from(word),
        }
    };
    let arguments = words
        .map(|word| {
            word.into_string()
                .map_err(|word| format!("the argument {word:?} is not UTF-8, which start takes"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut options = RunHostOptions::new(
        host_library,
        core.unwrap_or_else(|| beside_the_program("weivel_core")),
        standard.unwrap_or_else(|| beside_the_program("weivel_standard")),
    );
    options.arguments = arguments;
    options.log = Some(Arc::new(StandardErrorLog));
    Ok(options)
}

/// The code a process exits with for a run's exit code: the code modulo 256, as a process's exit status holds one
/// byte on every platform the program runs on.
pub fn process_exit_code(exit: u32) -> u8 {
    (exit % 256) as u8
}

/// Runs the host program from its command line, without the program's own name, and answers the code to exit with,
/// each terminate signal of the platform a stop request. The core and the standard are the shared libraries beside
/// the program unless `--core` and `--standard` name others.
pub fn main(command_line: Vec<OsString>) -> u32 {
    let options = match options_of(command_line) {
        Ok(options) => options,
        Err(why) => {
            eprintln!("{why}");
            return FAILED_RUN_EXIT;
        }
    };
    let forwarding = forward_terminate_signals(options.stop.clone());
    if forwarding.is_none() {
        eprintln!(
            "weivel-host: the terminate signals could not be hooked, so only the run itself ends it"
        );
    }
    run_host(options)
}
