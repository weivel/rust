//! One run of the host program, from `start` to `stop`, as `weivel/host.h` of the standard describes it.

use std::path::Path;
use std::sync::{Arc, Mutex};

use super::options::{FAILED_RUN_EXIT, ProgramLog, RunHostOptions, text_for_c};
use super::plugins::PluginRun;
use super::stop::{StopRequest, now_millis};
use crate::context::{Object, Pending, Settle, pending};
use crate::core::CoreBinary;
use crate::host::{DescribedCallError, Host, HostOptions, LoadedLibrary};
use crate::standard::{ActionEnding, HostPlan, HostProvider, NextTick, Standard, host_modes};

/// Why a run ends early: a refusal the provider answered, or anything else that went wrong.
enum Ending {
    Refused(DescribedCallError),
    Failed(String),
}

impl From<DescribedCallError> for Ending {
    fn from(refused: DescribedCallError) -> Ending {
        Ending::Refused(refused)
    }
}

fn log_ending(log: &ProgramLog, during: &str, ending: Ending) {
    match ending {
        Ending::Refused(refused) => log.warn(format!("The run ends: {refused}")),
        Ending::Failed(failure) => log.error(format!("The run ends: {during} failed: {failure}")),
    }
}

/// Whether the plan sets only one of `action_service` and `action_method`, or sets either to an empty text.
fn names_its_method_malformedly(plan: &HostPlan) -> bool {
    let service = plan.action_service.as_deref();
    let method = plan.action_method.as_deref();
    service == Some("") || method == Some("") || service.is_some() != method.is_some()
}

/// Loads a C library from its binary and the declaration the standard reads, as host services, withholding
/// `weivel.host` and `weivel.scope` from every plugin: the program binds them for itself, and a scope is made for one
/// plugin and handed to no other.
fn load_declared(
    host: &Host,
    standard: &Standard,
    binary: &Path,
    declaration: &Path,
) -> Result<LoadedLibrary, String> {
    let json = std::fs::read(declaration).map_err(|error| {
        format!(
            "The declaration {} could not be read: {error}",
            declaration.display()
        )
    })?;
    let declared = standard.read_library(&json).map_err(|refused| {
        format!(
            "The standard refused the declaration {}: {refused}",
            declaration.display()
        )
    })?;
    let ids = standard.service_ids();
    host.load_library_withholding(binary, &declared, &[ids.host.as_str(), ids.scope.as_str()])
        .map_err(|answer| format!("{} could not be loaded: {answer}", binary.display()))
}

/// What the one who started a run waits for: `started` told, or the run ended without getting that far, and the run's
/// plugins once it got that far.
struct Starting {
    reached: Mutex<Option<Settle<()>>>,
    plugins: Mutex<Option<Arc<PluginRun>>>,
}

impl Starting {
    fn reach(&self, plugins: Option<Arc<PluginRun>>) {
        if plugins.is_some() {
            *self
                .plugins
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = plugins;
        }
        let settle = self
            .reached
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(settle) = settle {
            settle.settle(());
        }
    }

    fn plugins(&self) -> Option<Arc<PluginRun>> {
        self.plugins
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// What one run started with an OK `start` holds, and how it carries the plan out.
struct Run<'a> {
    host: &'a Host,
    standard: &'a Standard,
    provider: HostProvider,
    plan: HostPlan,
    options: &'a RunHostOptions,
    log: ProgramLog,
    libraries: Vec<LoadedLibrary>,
    plugins: Option<Arc<PluginRun>>,
}

impl Run<'_> {
    /// Carries the plan out, releases every plugin, then calls `stop` last and answers its code.
    fn carry_out(mut self, starting: &Starting) -> u32 {
        if let Err(ending) = self.carry_out_the_plan(starting) {
            log_ending(&self.log, "carrying out the plan", ending);
        }
        if let Some(plugins) = &self.plugins {
            plugins.release_every();
        }
        match self.provider.stop() {
            Ok(code) => code,
            Err(refused) => {
                log_ending(&self.log, "stop", Ending::Refused(refused));
                FAILED_RUN_EXIT
            }
        }
    }

    /// Activation is one phase, as `host.h` says: a stop requested while it runs takes effect once `started` has been
    /// told every plugin's outcome, so it is read only after that call.
    fn carry_out_the_plan(&mut self, starting: &Starting) -> Result<(), Ending> {
        let mode = self.plan.mode;
        if mode != host_modes::SERVE && mode != host_modes::ACTION {
            return Err(Ending::Failed(format!(
                "start answered the mode {mode}, which this program does not run"
            )));
        }
        if mode == host_modes::ACTION && names_its_method_malformedly(&self.plan) {
            return Err(Ending::Failed(
                "start answered an action service without its method, a method without its service, or either empty"
                    .to_owned(),
            ));
        }
        self.load_every_library()?;
        let plugins = Arc::new(PluginRun::new(
            self.host.clone(),
            self.standard.clone(),
            self.options.entry_symbol.clone(),
            self.log.clone(),
            &self.libraries,
        ));
        self.plugins = Some(plugins.clone());
        let outcomes: Vec<_> = self
            .plan
            .plugins
            .iter()
            .map(|planned| plugins.activate(planned))
            .collect();
        self.provider.started(&outcomes)?;
        starting.reach(Some(plugins.clone()));
        if self.options.stop.is_requested() {
            return Ok(());
        }
        if mode == host_modes::SERVE {
            self.serve()
        } else {
            let ending = self.run_the_action(&plugins);
            self.provider.finished(&ending).map_err(Ending::from)
        }
    }

    /// Loads the plan's libraries in order; the first that will not load ends the run.
    fn load_every_library(&mut self) -> Result<(), Ending> {
        for library in &self.plan.libraries {
            let loaded = load_declared(
                self.host,
                self.standard,
                Path::new(&library.binary),
                Path::new(&library.declaration),
            )
            .map_err(Ending::Failed)?;
            self.libraries.push(loaded);
        }
        Ok(())
    }

    fn tick(&self) -> Result<NextTick, Ending> {
        match self.provider.tick(now_millis())? {
            NextTick::Malformed(has_next) => Err(Ending::Failed(format!(
                "tick answered has_next {has_next}, which is neither 0 nor 1"
            ))),
            next => Ok(next),
        }
    }

    /// Ticks at once, then at each time a tick answered, until a stop is requested.
    fn serve(&self) -> Result<(), Ending> {
        let stop = &self.options.stop;
        let mut next = self.tick()?;
        while !stop.is_requested() {
            match next {
                NextTick::At(at) => stop.wait_until(at),
                NextTick::Never | NextTick::Malformed(_) => {
                    stop.wait();
                    return Ok(());
                }
            }
            if stop.is_requested() {
                return Ok(());
            }
            next = self.tick()?;
        }
        Ok(())
    }

    fn run_the_action(&self, plugins: &PluginRun) -> ActionEnding {
        let plan = &self.plan;
        let action = match (&plan.action_service, &plan.action_method) {
            (Some(service), Some(method)) => {
                plugins.method_of(plan.action_plugin.as_deref(), service, method)
            }
            _ => plugins.action_of(plan.action_plugin.as_deref(), plan.action_name.as_deref()),
        };
        let ran = action.and_then(|action| action(plan.action_arguments.as_ref()));
        match ran {
            Ok(result) => ActionEnding::Answered(result),
            Err(failure) => ActionEnding::Failed(text_for_c(&failure)),
        }
    }
}

/// One run of the program over a host it made, answering the code it exits with.
fn run_on(host: &Host, standard: &Standard, options: &RunHostOptions, starting: &Starting) -> u32 {
    let log = ProgramLog::of(options);
    let host_library = match load_declared(
        host,
        standard,
        &options.host_library,
        &options.host_declaration(),
    ) {
        Ok(library) => library,
        Err(failure) => {
            log.error(format!("The host program could not run: {failure}"));
            return FAILED_RUN_EXIT;
        }
    };
    let Some(bound) = host_library.described(&standard.service_ids().host) else {
        log.error(format!(
            "The host program could not run: the host library {} serves no described {}",
            host_library.declaration().import_name,
            standard.service_ids().host
        ));
        return FAILED_RUN_EXIT;
    };
    let provider = HostProvider::of(bound);
    let plan = match provider.start(&options.arguments) {
        Ok(plan) => plan,
        Err(refused) => {
            log_ending(&log, "start", Ending::Refused(refused));
            return FAILED_RUN_EXIT;
        }
    };
    Run {
        host,
        standard,
        provider,
        plan,
        options,
        log,
        libraries: vec![host_library],
        plugins: None,
    }
    .carry_out(starting)
}

/// One run of the program to its end, answering the code it exits with; the host is destroyed, and with it every
/// library unloaded, once the run has ended.
fn run_to_the_end(options: &RunHostOptions, starting: &Starting) -> u32 {
    let log = ProgramLog::of(options);
    let made = Standard::load(&options.standard_library)
        .map_err(|failure| {
            format!(
                "the standard {} could not be loaded: {failure}",
                options.standard_library.display()
            )
        })
        .and_then(|standard| {
            let core = CoreBinary::load(&options.core_library).map_err(|failure| {
                format!(
                    "the core {} could not be loaded: {failure}",
                    options.core_library.display()
                )
            })?;
            let host = Host::create(
                &core,
                HostOptions {
                    log: options.log.clone(),
                    sink: options.sink.clone(),
                    description_check: Some(Arc::new(standard.descriptions().clone())),
                    ..HostOptions::default()
                },
            )
            .map_err(|answer| format!("the host could not be made: {answer}"))?;
            Ok((standard, host))
        });
    let (standard, host) = match made {
        Ok(made) => made,
        Err(failure) => {
            log.error(format!("The host program could not run: {failure}"));
            return FAILED_RUN_EXIT;
        }
    };
    let exit = run_on(&host, &standard, options, starting);
    host.destroy().wait();
    exit
}

/// A host program [`start_host`] started in this process.
pub struct RunningHost {
    stop: StopRequest,
    ended: Pending<u32>,
    starting: Arc<Starting>,
}

impl std::fmt::Debug for RunningHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "RunningHost(ended: {})", self.ended.is_settled())
    }
}

impl RunningHost {
    /// The object the plugin whose manifest's `importName` is `plugin` itself registered under the service id `id`
    /// and still has registered, or `None`: a registration under the id by another plugin or the host is never
    /// answered, and nothing is once the run has ended.
    pub fn service(&self, plugin: &str, id: &str) -> Option<Object> {
        self.starting.plugins()?.service_of(plugin, id)
    }

    /// Requests the run's stop and answers [`RunningHost::ended`].
    pub fn stop(&self) -> Pending<u32> {
        self.stop.request();
        self.ended.clone()
    }

    /// Settles with the code the run exits with once it has ended, by a stop request or by itself.
    pub fn ended(&self) -> Pending<u32> {
        self.ended.clone()
    }
}

/// Starts the library at `options.host_library` as a host program's provider, as `weivel/host.h` of the standard
/// describes, on a thread of its own in this process, and answers the running host once `started` has been told every
/// plugin's outcome, or once the run has ended without getting that far. Every decision is the provider's; the program
/// reads the clock and carries its answers out, and once `start` answered OK it calls `stop` last whatever happened.
/// `options.stop` and the answer's [`RunningHost::stop`] are the same stop request. The libraries are unloaded, and the
/// core destroyed, once the run has ended.
pub fn start_host(options: RunHostOptions) -> RunningHost {
    let (reached, settle_reached) = pending::<()>();
    let (ended, settle_ended) = pending::<u32>();
    let starting = Arc::new(Starting {
        reached: Mutex::new(Some(settle_reached)),
        plugins: Mutex::new(None),
    });
    let stop = options.stop.clone();
    let running = starting.clone();
    let spawned = std::thread::Builder::new()
        .name("weivel-host-run".to_owned())
        .spawn(move || {
            let exit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_to_the_end(&options, &running)
            }))
            .unwrap_or(FAILED_RUN_EXIT);
            running.reach(None);
            settle_ended.settle(exit);
        });
    if spawned.is_err() {
        return RunningHost {
            stop,
            ended: Pending::settled(FAILED_RUN_EXIT),
            starting,
        };
    }
    reached.wait();
    RunningHost {
        stop,
        ended,
        starting,
    }
}

/// Runs the program as [`start_host`] does and answers the code it exits with once the run has ended: `stop`'s, or
/// [`FAILED_RUN_EXIT`] when `start` did not answer OK, in which case nothing else is called.
pub fn run_host(options: RunHostOptions) -> u32 {
    start_host(options).ended().wait()
}
