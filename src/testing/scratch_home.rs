//! A fresh folder laid out with libraries, files and variables, and its removal.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::TestingFailure;
use super::variables::{VariablesSet, set_variables};

/// The name of a C library's declaration, which lies in the library's folder beside its one binary.
pub const LIBRARY_DECLARATION: &str = "library.json";

/// A file a layout has written into its home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScratchFile {
    /// Written with these bytes, its folders made first.
    Written { path: PathBuf, content: Vec<u8> },
    /// Copied byte for byte from `from`, its folders made first.
    Copied { path: PathBuf, from: PathBuf },
}

impl ScratchFile {
    /// A file written with `text`, as it is. A value meant as JSON is written as its text, since the crate has no JSON
    /// writer and a layout decides how its own files read.
    pub fn text(path: impl Into<PathBuf>, text: &str) -> ScratchFile {
        ScratchFile::Written {
            path: path.into(),
            content: text.as_bytes().to_vec(),
        }
    }
}

/// Where a library of a layout was placed: its folder, and its one binary in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedLibrary {
    pub directory: PathBuf,
    pub binary: PathBuf,
}

/// What a scratch home holds, which is the test's own: nothing here names any one project's layout.
pub trait ScratchLayout {
    /// Starts the name of the temporary folder.
    fn prefix(&self) -> &str;

    /// Each library's name and the source folder holding its one binary and its `library.json`.
    fn libraries(&self) -> Vec<(String, PathBuf)> {
        Vec::new()
    }

    /// The variables to set while the home lives, given the home's base.
    fn environment(&self, _base: &Path) -> Vec<(String, OsString)> {
        Vec::new()
    }

    /// The files to write, given the home's base and where each library was placed.
    fn files(
        &self,
        _base: &Path,
        _libraries: &BTreeMap<String, PlacedLibrary>,
    ) -> Result<Vec<ScratchFile>, TestingFailure> {
        Ok(Vec::new())
    }
}

/// A home [`scratch_home`] laid out, until [`ScratchHome::remove`].
#[derive(Debug)]
pub struct ScratchHome {
    /// The home's folder, a real path.
    pub base: PathBuf,
    /// Each library by its name in the layout.
    pub libraries: BTreeMap<String, PlacedLibrary>,
    variables: VariablesSet,
}

impl ScratchHome {
    /// Puts every variable the layout set back as it was, removing one that was unset, and deletes the home.
    ///
    /// # Safety
    ///
    /// No other thread may read or write the environment while it runs, as for [`super::set_variables`].
    pub unsafe fn remove(self) {
        // SAFETY: the caller's promise is this function's own.
        unsafe { self.variables.restore() };
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn failure_at(action: &str, path: &Path, error: std::io::Error) -> TestingFailure {
    TestingFailure::new(format!("{action} {} failed: {error}", path.display()))
}

fn ensured_directory(path: &Path) -> Result<(), TestingFailure> {
    std::fs::create_dir_all(path).map_err(|error| failure_at("making", path, error))
}

fn place_file(file: &ScratchFile) -> Result<(), TestingFailure> {
    let path = match file {
        ScratchFile::Written { path, .. } | ScratchFile::Copied { path, .. } => path,
    };
    if let Some(parent) = path.parent() {
        ensured_directory(parent)?;
    }
    match file {
        ScratchFile::Written { path, content } => {
            std::fs::write(path, content).map_err(|error| failure_at("writing", path, error))
        }
        ScratchFile::Copied { path, from } => std::fs::copy(from, path)
            .map(drop)
            .map_err(|error| failure_at("copying to", path, error)),
    }
}

fn file_names(directory: &Path) -> Result<Vec<OsString>, TestingFailure> {
    let entries =
        std::fs::read_dir(directory).map_err(|error| failure_at("reading", directory, error))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| failure_at("reading", directory, error))?;
        if entry.path().is_file() {
            names.push(entry.file_name());
        }
    }
    names.sort();
    Ok(names)
}

/// The name of the one binary in a library's folder: its first file by name that is not the declaration.
pub fn library_binary_name(directory: &Path) -> Result<OsString, TestingFailure> {
    file_names(directory)?
        .into_iter()
        .find(|name| name != OsStr::new(LIBRARY_DECLARATION))
        .ok_or_else(|| {
            TestingFailure::new(format!(
                "{} holds no binary beside {LIBRARY_DECLARATION}",
                directory.display()
            ))
        })
}

fn place_library(base: &Path, name: &str, source: &Path) -> Result<PlacedLibrary, TestingFailure> {
    let binary_name = library_binary_name(source)?;
    let directory = base.join("libraries").join(name);
    ensured_directory(&directory)?;
    for file_name in file_names(source)? {
        let target = directory.join(&file_name);
        std::fs::copy(source.join(&file_name), &target)
            .map_err(|error| failure_at("copying to", &target, error))?;
    }
    Ok(PlacedLibrary {
        binary: directory.join(binary_name),
        directory,
    })
}

fn lay_out(
    base: &Path,
    layout: &impl ScratchLayout,
) -> Result<BTreeMap<String, PlacedLibrary>, TestingFailure> {
    let mut libraries = BTreeMap::new();
    for (name, source) in layout.libraries() {
        let placed = place_library(base, &name, &source)?;
        libraries.insert(name, placed);
    }
    for file in layout.files(base, &libraries)? {
        place_file(&file)?;
    }
    Ok(libraries)
}

/// A path with every link resolved, written as a plain path. macOS's temporary directory sits under `/var`, a link to `/private/var`, and what a process resolves
/// there is the real path, so a path a test builds is compared only once it is real too. Windows answers a real path
/// in its verbatim form, `\\?\C:\...`, which is written back as the plain drive path it names.
fn real_path(path: &Path) -> Result<PathBuf, TestingFailure> {
    let real = std::fs::canonicalize(path).map_err(|error| failure_at("resolving", path, error))?;
    let text = real.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(plain) if !plain.starts_with(r"UNC\") => Ok(PathBuf::from(plain)),
        _ => Ok(real),
    }
}

/// A fresh folder under the temporary directory whose name starts with `prefix`, as a real path.
fn real_temporary_folder(prefix: &str) -> Result<PathBuf, TestingFailure> {
    static MADE: AtomicU64 = AtomicU64::new(0);
    loop {
        let made = MADE.fetch_add(1, Ordering::Relaxed);
        let name = format!("{prefix}{}-{made}", std::process::id());
        let folder = std::env::temp_dir().join(name);
        match std::fs::create_dir(&folder) {
            Ok(()) => return real_path(&folder),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(failure_at("making", &folder, error)),
        }
    }
}

/// Lays out a fresh home as `layout` describes it: the environment pointed into it, the libraries copied to
/// `<base>/libraries/<name>`, and the files written, until [`ScratchHome::remove`]. A layout that fails partway puts
/// the environment back and deletes the home before its failure is answered.
///
/// # Safety
///
/// No other thread may read or write the environment while it runs, as for [`super::set_variables`].
pub unsafe fn scratch_home(layout: &impl ScratchLayout) -> Result<ScratchHome, TestingFailure> {
    let base = real_temporary_folder(layout.prefix())?;
    let environment = layout.environment(&base);
    // SAFETY: the caller's promise is this function's own.
    let variables = unsafe { set_variables(&environment) };
    let home = ScratchHome {
        base,
        libraries: BTreeMap::new(),
        variables,
    };
    match lay_out(&home.base, layout) {
        Ok(libraries) => Ok(ScratchHome { libraries, ..home }),
        Err(failure) => {
            // SAFETY: the caller's promise is this function's own.
            unsafe { home.remove() };
            Err(failure)
        }
    }
}
