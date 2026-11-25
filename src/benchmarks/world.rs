//! The host a benchmark runs in, composed as a program composes one: its timer armed on the system scheduler, the
//! standard's description check, the C libraries a benchmark loads, and a provider and a consumer activated in it.

use std::path::PathBuf;
use std::sync::Arc;

use super::BenchmarkFailure;
use crate::context::{Context, Object, Pending};
use crate::core::{CoreBinary, EdgeSource, Handle, Manifest, ServiceOffer, ServiceWant};
use crate::host::{
    ActivationState, Entry, Finished, FunctionKind, Host, HostOptions, LibraryDeclaration,
    LibraryFunction, LibraryServiceDeclaration,
};
use crate::standard::StandardDescriptions;

/// Where the native files a benchmark loads lie.
#[derive(Debug, Clone)]
pub struct BenchmarkFiles {
    /// The core's shared library.
    pub core: PathBuf,
    /// The standard's shared library, whose description check the host runs.
    pub standard: PathBuf,
    /// The core's `echo` test library.
    pub echo: PathBuf,
    /// The standard's `described` test library.
    pub described: PathBuf,
}

pub(crate) const ARITHMETIC_ID: &str = "provider:arithmetic";
pub(crate) const ECHO_ID: &str = "echo";
pub(crate) const ECHO_DEFINITION_ID: &str = "echo-definition";
pub(crate) const MIRROR_ID: &str = "described-mirror";

/// An entry whose activate settles at once, successfully.
struct Settles;

impl Entry for Settles {
    fn activate(&self, _: &Context) -> Pending<Finished> {
        Pending::settled(Ok(()))
    }
}

fn manifest(import_name: &str, provides: &[&str], consumes: &[&str]) -> Manifest {
    Manifest {
        import_name: import_name.to_owned(),
        services_provided: provides
            .iter()
            .map(|id| ServiceOffer {
                id: (*id).to_owned(),
                attributes: Vec::new(),
            })
            .collect(),
        services_consumed: consumes
            .iter()
            .map(|id| ServiceWant {
                id: (*id).to_owned(),
                provider: None,
                optional: Some(true),
                degradable: Some(false),
                constraints: Vec::new(),
            })
            .collect(),
        ..Manifest::default()
    }
}

fn function(name: &str, symbol: &str, kind: FunctionKind) -> LibraryFunction {
    LibraryFunction {
        name: name.to_owned(),
        symbol: symbol.to_owned(),
        kind,
        context: 0,
    }
}

/// The echo library's `echo_echo` as a bytes service.
pub(crate) fn echo_declaration() -> LibraryDeclaration {
    LibraryDeclaration {
        import_name: "echo".to_owned(),
        version: "0.1.0".to_owned(),
        release: Some("echo_release".to_owned()),
        services: vec![LibraryServiceDeclaration {
            id: ECHO_ID.to_owned(),
            functions: vec![function("echo", "echo_echo", FunctionKind::Bytes)],
            description: None,
        }],
        contracts: Vec::new(),
    }
}

/// The echo library's definition, undescribed.
pub(crate) fn echo_definition_declaration() -> LibraryDeclaration {
    LibraryDeclaration {
        import_name: "echo".to_owned(),
        version: "0.2.0".to_owned(),
        release: None,
        services: vec![LibraryServiceDeclaration {
            id: ECHO_DEFINITION_ID.to_owned(),
            functions: vec![function(
                "definition",
                "echo_service_definition",
                FunctionKind::Definition,
            )],
            description: None,
        }],
        contracts: Vec::new(),
    }
}

/// The described library's mirror, bound from its description.
pub(crate) fn mirror_declaration() -> LibraryDeclaration {
    LibraryDeclaration {
        import_name: "described".to_owned(),
        version: "1.0.0".to_owned(),
        release: None,
        services: vec![LibraryServiceDeclaration {
            id: MIRROR_ID.to_owned(),
            functions: vec![function(
                "definition",
                "described_mirror_definition",
                FunctionKind::Definition,
            )],
            description: Some("described_mirror_description".to_owned()),
        }],
        contracts: Vec::new(),
    }
}

/// A host, with a provider and a consumer activated in it, destroyed when dropped.
pub(crate) struct World {
    pub(crate) host: Host,
    pub(crate) provider: Context,
    pub(crate) consumer: Context,
}

impl World {
    /// Makes the host, loads each library at its path under its declaration, then registers and activates the
    /// provider and the consumer.
    pub(crate) fn create(
        files: &BenchmarkFiles,
        libraries: &[(PathBuf, LibraryDeclaration)],
    ) -> Result<World, BenchmarkFailure> {
        let binary = CoreBinary::load(&files.core)
            .map_err(|failure| BenchmarkFailure(format!("the core did not load: {failure}")))?;
        let check = StandardDescriptions::load(&files.standard)
            .map_err(|failure| BenchmarkFailure(format!("the standard did not load: {failure}")))?;
        let host = Host::create(
            &binary,
            HostOptions {
                description_check: Some(Arc::new(check)),
                ..HostOptions::default()
            },
        )
        .map_err(|answer| BenchmarkFailure(format!("the host was refused: {answer:?}")))?;
        let world = World::populated(host.clone(), libraries);
        if world.is_err() {
            host.destroy().wait();
        }
        world
    }

    fn populated(
        host: Host,
        libraries: &[(PathBuf, LibraryDeclaration)],
    ) -> Result<World, BenchmarkFailure> {
        for (path, declaration) in libraries {
            host.load_library(path, declaration).map_err(|answer| {
                BenchmarkFailure(format!("{} did not load: {answer:?}", path.display()))
            })?;
        }
        let consumed = [ARITHMETIC_ID, ECHO_ID, ECHO_DEFINITION_ID, MIRROR_ID];
        let provider = register(&host, manifest("provider", &[ARITHMETIC_ID], &[]))?;
        let consumer = register(&host, manifest("consumer", &[], &consumed))?;
        activate(&host, &[provider, consumer])?;
        let context = |plugin: Handle| {
            host.context(plugin)
                .ok_or_else(|| BenchmarkFailure(format!("plugin {} has no context", plugin.0)))
        };
        Ok(World {
            provider: context(provider)?,
            consumer: context(consumer)?,
            host,
        })
    }

    /// The object the consumer resolves under `id`.
    pub(crate) fn resolved(&self, id: &str) -> Result<Object, BenchmarkFailure> {
        self.consumer.get(id).map_err(|answer| {
            BenchmarkFailure(format!("the consumer could not get {id}: {answer:?}"))
        })
    }
}

impl Drop for World {
    fn drop(&mut self) {
        self.host.destroy().wait();
    }
}

fn register(host: &Host, manifest: Manifest) -> Result<Handle, BenchmarkFailure> {
    let name = manifest.import_name.clone();
    host.register(manifest, Arc::new(Settles))
        .map_err(|answer| BenchmarkFailure(format!("{name} was not registered: {answer:?}")))
}

fn activate(host: &Host, plugins: &[Handle]) -> Result<(), BenchmarkFailure> {
    let activation = host
        .activate(plugins, EdgeSource::CAPABILITIES)
        .map_err(|answer| BenchmarkFailure(format!("the activation was refused: {answer:?}")))?;
    for outcome in &activation.outcomes {
        let outcome = outcome.wait();
        if outcome.state != ActivationState::Active {
            return Err(BenchmarkFailure(format!(
                "plugin {} ended {:?}: {:?}",
                outcome.plugin.0, outcome.state, outcome.refusal
            )));
        }
    }
    Ok(())
}
