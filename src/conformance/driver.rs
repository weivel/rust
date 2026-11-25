//! Reading the kit's layer vectors, and running one through a host of its own.

use std::fs;
use std::path::Path;

use super::observations::observe;
use super::shapes::{self, DriverFailure, Members};
use super::world::DriverWorld;
use super::{consume, lifecycle, want};
use crate::context::LogRecord;
use crate::core::{CoreBinary, CoreOptions, Failure};
use crate::standard::{Json, StandardJson};

/// One layer vector of the kit, as the standard's reader read it.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerVector {
    /// Its id: its path under the kit's `vectors/layer/`, without `.json`.
    pub id: String,
    /// The op that drives it.
    pub op: String,
    /// What the run is built from.
    pub given: Json,
    /// What the run must show.
    pub then: Json,
}

/// What the driver saw of one vector's run.
#[derive(Debug, Clone)]
pub struct LayerWitness {
    /// The vector's id.
    pub id: String,
    /// Every field of the vector's `then`, in its shape: an `equals` field as `{equals: observed}`, a `before` field
    /// as each pair asked, marked when the run broke it, and any other field as the observed value.
    pub observed: Json,
    /// Every record the host's log received during the run, in order.
    pub logs: Vec<LogRecord>,
    /// Every failure the error sink received during the run, in order.
    pub failures: Vec<Failure>,
}

/// The driver for the layer suite: the core its caller loaded and the standard's JSON reader. One driver runs one
/// vector at a time, each on a host of its own.
#[derive(Debug, Clone)]
pub struct LayerDriver {
    core: CoreBinary,
    json: StandardJson,
}

impl LayerDriver {
    /// A driver over the core's binary and the standard's JSON reader, each loaded by its caller.
    pub fn new(core: CoreBinary, json: StandardJson) -> LayerDriver {
        LayerDriver { core, json }
    }

    /// Reads every layer vector under `directory`, at any depth, sorted by id. A vector the reader refuses, or whose
    /// id is not its path without `.json`, fails the read.
    pub fn read(&self, directory: &Path) -> Result<Vec<LayerVector>, DriverFailure> {
        let mut vectors = Vec::new();
        self.read_into(directory, "", &mut vectors)?;
        vectors.sort_by(|first, second| first.id.cmp(&second.id));
        Ok(vectors)
    }

    fn read_into(
        &self,
        directory: &Path,
        prefix: &str,
        vectors: &mut Vec<LayerVector>,
    ) -> Result<(), DriverFailure> {
        let listing = fs::read_dir(directory)
            .map_err(|error| format!("{} could not be listed: {error}", directory.display()))?;
        for found in listing {
            let path = found
                .map_err(|error| format!("{} could not be listed: {error}", directory.display()))?
                .path();
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if path.is_dir() {
                self.read_into(&path, &format!("{prefix}{name}/"), vectors)?;
            } else if let Some(stem) = name.strip_suffix(".json") {
                vectors.push(self.vector(&path, &format!("{prefix}{stem}"))?);
            }
        }
        Ok(())
    }

    fn vector(&self, path: &Path, id: &str) -> Result<LayerVector, DriverFailure> {
        let bytes = fs::read(path)
            .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
        let document = self
            .json
            .parse(&bytes)
            .map_err(|refusal| format!("{}: {refusal}", path.display()))?;
        let place = format!("the vector {id}");
        let members = shapes::object(&place, &document)?;
        let named = shapes::text(&place, members, "id")?;
        if named != id {
            return Err(format!("{place} names itself {named}"));
        }
        Ok(LayerVector {
            id: named,
            op: shapes::text(&place, members, "op")?,
            given: shapes::member(&place, members, "given")?.clone(),
            then: shapes::member(&place, members, "then")?.clone(),
        })
    }

    /// Builds the vector's `given` through a host on the real core, drives it, and answers every field of its `then`
    /// from the run, before the host is destroyed.
    pub fn witness(&self, vector: &LayerVector) -> Result<LayerWitness, DriverFailure> {
        let place = format!("the vector {}", vector.id);
        let given = shapes::object(&format!("{place}'s given"), &vector.given)?;
        let then = shapes::object(&format!("{place}'s then"), &vector.then)?;
        let options = if vector.op == "want" {
            want::core_options(given)?
        } else {
            CoreOptions::default()
        };
        let mut world = DriverWorld::new(&self.core, options)?;
        let observed = drive(vector, given, then, &mut world)?;
        let witness = LayerWitness {
            id: vector.id.clone(),
            observed,
            logs: world.logs(),
            failures: world.failures(),
        };
        world.host.destroy();
        Ok(witness)
    }
}

fn drive(
    vector: &LayerVector,
    given: &Members,
    then: &Members,
    world: &mut DriverWorld,
) -> Result<Json, DriverFailure> {
    match vector.op.as_str() {
        "consume" => {
            let run = consume::drive(given, world)?;
            observe(&vector.id, then, &run.observers(world))
        }
        "want" => {
            let run = want::drive(given, world)?;
            observe(&vector.id, then, &run.observers(world))
        }
        "lifecycle" => {
            let run = lifecycle::drive(given, world)?;
            observe(&vector.id, then, &run.observers(world))
        }
        other => Err(format!(
            "the vector {} has the op {other}, which this driver does not drive",
            vector.id
        )),
    }
}
