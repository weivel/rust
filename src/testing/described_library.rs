//! A described C library bound from its folder on a host of its own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::TestingFailure;
use super::scratch_home::{LIBRARY_DECLARATION, library_binary_name};
use crate::core::CoreBinary;
use crate::host::{BoundDefinition, Host, HostOptions, LibraryDeclaration, LoadedLibrary};
use crate::standard::Standard;

/// The binaries a described library is bound with, each loaded from the path its caller gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribedLibraryOptions {
    /// The core's shared library.
    pub core_library: PathBuf,
    /// The standard's shared library, which reads the declaration and checks each description.
    pub standard_library: PathBuf,
}

/// A C library [`described_library`] loaded, until it is closed or dropped.
pub struct DescribedLibrary {
    host: Host,
    described: BTreeMap<String, BoundDefinition>,
}

impl std::fmt::Debug for DescribedLibrary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DescribedLibrary")
            .field("described", &self.described.keys())
            .finish()
    }
}

impl DescribedLibrary {
    /// Each service the library declares with a description, bound as a plugin would get it, by id.
    pub fn described(&self) -> &BTreeMap<String, BoundDefinition> {
        &self.described
    }

    /// The service `id` bound from its description; `None` for an id the library declares no description under.
    pub fn get(&self, id: &str) -> Option<&BoundDefinition> {
        self.described.get(id)
    }

    /// Unloads the library and destroys the host that loaded it, as dropping it does, so every later call through
    /// one of its bound services answers [`crate::host::CallStatus::Unloaded`].
    pub fn close(self) {}
}

impl Drop for DescribedLibrary {
    fn drop(&mut self) {
        self.host.destroy().wait();
    }
}

fn declaration_in(
    directory: &Path,
    standard: &Standard,
) -> Result<LibraryDeclaration, TestingFailure> {
    let path = directory.join(LIBRARY_DECLARATION);
    let json = std::fs::read(&path).map_err(|error| {
        TestingFailure::new(format!("{} could not be read: {error}", path.display()))
    })?;
    standard.read_library(&json).map_err(|refused| {
        TestingFailure::new(format!(
            "The standard refused {}: {refused}",
            path.display()
        ))
    })
}

fn described_of(library: &LoadedLibrary) -> BTreeMap<String, BoundDefinition> {
    library
        .declaration()
        .services
        .iter()
        .filter_map(|service| {
            library
                .described(&service.id)
                .map(|bound| (service.id.clone(), bound))
        })
        .collect()
}

fn host_checked_by(standard: &Standard, core_library: &Path) -> Result<Host, TestingFailure> {
    let core = CoreBinary::load(core_library).map_err(|failure| {
        TestingFailure::new(format!(
            "The core {} could not be loaded: {failure}",
            core_library.display()
        ))
    })?;
    Host::create(
        &core,
        HostOptions {
            description_check: Some(Arc::new(standard.descriptions().clone())),
            ..HostOptions::default()
        },
    )
    .map_err(|answer| TestingFailure::new(format!("The host could not be made: {answer}")))
}

/// Loads the C library in `directory`, its one binary beside its `library.json`, on a host of its own given the
/// standard's description check, so a test calls any described library's operations with no binding written by hand.
///
/// Answers a failure naming the declaration the standard refuses or that cannot be read, a folder holding no binary,
/// a core or standard that cannot be loaded, and every refusal of [`Host::load_library`], the host destroyed again.
pub fn described_library(
    directory: &Path,
    options: &DescribedLibraryOptions,
) -> Result<DescribedLibrary, TestingFailure> {
    let standard = Standard::load(&options.standard_library).map_err(|failure| {
        TestingFailure::new(format!(
            "The standard {} could not be loaded: {failure}",
            options.standard_library.display()
        ))
    })?;
    let declaration = declaration_in(directory, &standard)?;
    let binary = directory.join(library_binary_name(directory)?);
    let host = host_checked_by(&standard, &options.core_library)?;
    match host.load_library(&binary, &declaration) {
        Ok(library) => Ok(DescribedLibrary {
            described: described_of(&library),
            host,
        }),
        Err(answer) => {
            host.destroy().wait();
            Err(TestingFailure::new(format!(
                "{} could not be loaded: {answer}",
                binary.display()
            )))
        }
    }
}
