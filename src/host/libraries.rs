//! C libraries as host services: loaded from a declaration and a path, each service registered under `WEIVEL_HOST`,
//! unloaded at once, and each binary closed once nothing of it can run.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use super::declaration::{LibraryDeclaration, LibraryServiceDeclaration, mistake_in};
use super::description::{BoundDefinition, Holdings, Mapping, bind_described, contract_of};
use super::state::{Host, HostShared, change_count};
use crate::context::{ForeignObject, Object, ServiceRegistration};
use crate::core::{Answer, BytesFunction, FailureKind, LibraryBinary, LibraryBytes};

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A library's binary while it is open to calls: taken at the unload or the destroy, so a later call finds none and a
/// call under way keeps the binary open only until it returns.
struct Lifetime {
    binary: Mutex<Option<Arc<LibraryBinary>>>,
}

impl Lifetime {
    fn binary(&self) -> Option<Arc<LibraryBinary>> {
        locked(&self.binary).clone()
    }

    fn close_to_calls(&self) {
        let closing = locked(&self.binary).take();
        drop(closing);
    }
}

/// What a call through a [`LibraryService`] answered.
#[derive(Debug)]
pub enum LibraryAnswer {
    /// Status 0, and the bytes the library answered, released when dropped.
    Returned(LibraryBytes),
    /// Any other status, and the body the library answered with it, read as text where it is well-formed UTF-8.
    Failed {
        /// The status.
        status: i32,
        /// Why, as the library said it.
        body: LibraryBytes,
    },
    /// The library was unloaded, so nothing was called.
    Unloaded,
    /// The service declares no function of that name, so nothing was called.
    NoSuchFunction,
}

/// A service of bytes functions a C library declares: the object a consumer resolves, each function called by its
/// declared name as a plain call into the library.
pub struct LibraryService {
    id: String,
    functions: BTreeMap<String, BytesFunction>,
    lifetime: Arc<Lifetime>,
}

impl LibraryService {
    /// The service id it is registered under.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The names of its functions.
    pub fn function_names(&self) -> Vec<String> {
        self.functions.keys().cloned().collect()
    }

    /// Calls the function `function` with `input`, lent to the library for the call only. The bytes it answers are
    /// the library's, read in place and released when the answer is dropped, keeping the binary open until then.
    pub fn call(&self, function: &str, input: &[u8]) -> LibraryAnswer {
        let Some(bytes_function) = self.functions.get(function).copied() else {
            return LibraryAnswer::NoSuchFunction;
        };
        let Some(binary) = self.lifetime.binary() else {
            return LibraryAnswer::Unloaded;
        };
        let answered = binary.call(bytes_function, input);
        match answered.status {
            0 => LibraryAnswer::Returned(answered.body),
            status => LibraryAnswer::Failed {
                status,
                body: answered.body,
            },
        }
    }
}

struct LoadedInner {
    host: Weak<HostShared>,
    declaration: LibraryDeclaration,
    objects: Vec<(String, Object)>,
    lifetime: Arc<Lifetime>,
    /// Every handle and kept answer its bound definitions answered that nobody has released yet.
    holdings: Arc<Holdings>,
    /// The ids it registers for no plugin, so only the caller that loaded it reaches them.
    withheld: Vec<String>,
    /// The capabilities whose contracts it published.
    contracts: Mutex<Vec<String>>,
    /// The registrations it made, each with its id; `None` once an unload or the destroy took it.
    registrations: Mutex<Option<Vec<(String, ServiceRegistration)>>>,
}

/// A C library a host loaded. Cloning it shares it.
#[derive(Clone)]
pub struct LoadedLibrary {
    inner: Arc<LoadedInner>,
}

impl std::fmt::Debug for LoadedLibrary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "LoadedLibrary({})",
            self.inner.declaration.import_name
        )
    }
}

impl LoadedLibrary {
    /// The declaration it was loaded from, its contracts among it.
    pub fn declaration(&self) -> &LibraryDeclaration {
        &self.inner.declaration
    }

    /// Each service's id and the object registered under it: a [`LibraryService`], a [`ForeignObject`] carrying the
    /// address a definition declaring no description answered, which dies with the binary, since holding this keeps
    /// nothing open, or the [`BoundDefinition`](super::BoundDefinition) a described definition was bound as.
    pub fn services(&self) -> Vec<(String, Object)> {
        self.inner.objects.clone()
    }

    /// The definition it bound for the service `id` from the description the service declares, a withheld one
    /// included; `None` for an id it declares no described definition under.
    pub fn described(&self, id: &str) -> Option<BoundDefinition> {
        self.inner
            .objects
            .iter()
            .find(|(declared, _)| declared == id)
            .and_then(|(_, object)| object.downcast_ref::<BoundDefinition>())
            .cloned()
    }

    /// Whether it declares the service `id` and has not been unloaded.
    pub(super) fn declares(&self, id: &str) -> bool {
        locked(&self.inner.registrations).is_some()
            && self
                .inner
                .declaration
                .services
                .iter()
                .any(|service| service.id == id)
    }

    fn take_for_unloading(&self) -> Option<Vec<(String, ServiceRegistration)>> {
        locked(&self.inner.registrations).take()
    }

    /// Keeps a registration it made, answering false once an unload took it, so its caller unregisters it at once.
    fn keep(&self, id: &str, registration: ServiceRegistration) -> Result<(), ServiceRegistration> {
        match locked(&self.inner.registrations).as_mut() {
            Some(registrations) => {
                registrations.push((id.to_owned(), registration));
                Ok(())
            }
            None => Err(registration),
        }
    }

    fn is_withheld(&self, id: &str) -> bool {
        self.inner.withheld.iter().any(|withheld| withheld == id)
    }

    fn is_of(&self, shared: &Arc<HostShared>) -> bool {
        std::ptr::eq(self.inner.host.as_ptr(), Arc::as_ptr(shared))
    }
}

fn taken_id_text(id: &str) -> String {
    format!(
        "the service id {id} is registered under this host already, and a registration under WEIVEL_HOST would \
         replace it"
    )
}

impl Host {
    /// Loads a C library and registers each service it declares under `WEIVEL_HOST`: a service of bytes functions as a
    /// [`LibraryService`], and a definition service, registered awaiting releases, as a [`ForeignObject`] carrying
    /// the address its definition's symbol answered, called once here, or, for one declaring a description, as the
    /// [`BoundDefinition`](super::BoundDefinition) bound from it once the host's description check has passed it. An activation plan counts each as provided by the host, and a consumer
    /// resolving one receives that very object. A declaration of contracts alone loads and registers nothing. A load
    /// that fails once a service is registered is undone as an unload.
    ///
    /// Answers the library; [`Answer::Mistake`], told to no sink, for a declaration [`LibraryDeclaration`]'s rules
    /// refuse, an id a registration under this host holds already, a path that names no library, a library lacking a
    /// declared symbol, a definition answering NULL or a struct whose `size` is below `2 * sizeof(void *)`, a
    /// description declared on a host given no description check, one the check refuses, or one this platform's
    /// layout disagrees with or the binding cannot bind; the core's refusal of a
    /// registration; [`Answer::Destroyed`] once the destroy has been called, before any mistake is judged.
    pub fn load_library(
        &self,
        path: &Path,
        declaration: &LibraryDeclaration,
    ) -> Result<LoadedLibrary, Answer> {
        self.load_library_withholding(path, declaration, &[])
    }

    /// Loads a C library as [`Host::load_library`] does, binding every service it declares, but registering none of
    /// the ids `withheld` names, so only its caller reaches those, through [`LoadedLibrary::described`] or
    /// [`LoadedLibrary::services`]. Each contract the declaration publishes, its description checked by the host's
    /// description check, is published to the plugins' contexts until the unload, so a plugin's provide of its
    /// capability is refused as [`Answer::ContractMismatch`] when the implementation does not offer every operation the
    /// description lists but its releases. Answers as [`Host::load_library`] does, and [`Answer::Mistake`] for a
    /// contract another live library publishes for the same capability.
    pub fn load_library_withholding(
        &self,
        path: &Path,
        declaration: &LibraryDeclaration,
        withheld: &[&str],
    ) -> Result<LoadedLibrary, Answer> {
        if self.shared.is_destroy_called() {
            return Err(Answer::Destroyed);
        }
        if let Some(mistake) = mistake_in(declaration) {
            return Err(Answer::Mistake(mistake));
        }
        if let Some(id) = self.registered_already(declaration, withheld) {
            return Err(Answer::Mistake(taken_id_text(&id)));
        }
        let binary = LibraryBinary::open(
            path,
            declaration.release.as_deref(),
            refused_close(&self.shared, &declaration.import_name),
        )?;
        let library = self.built(binary, declaration, withheld)?;
        self.kept(&library)?;
        let registered = self
            .publish_contracts(&library)
            .and_then(|()| self.register_services(&library));
        if registered.is_err() || self.shared.is_destroy_called() {
            self.shared.unload_taken(&library);
            return Err(registered.err().unwrap_or(Answer::Destroyed));
        }
        Ok(library)
    }

    /// Unloads a library: releases every handle its bound definitions answered and every answer carrying a definition
    /// that nobody released, while its binary is still mapped, closes it to calls at once, so every later call through one of its bytes services answers
    /// [`LibraryAnswer::Unloaded`], and unregisters every service it registered, which revokes each definition to its
    /// holders. Its binary closes once no call runs in it, no bytes it answered are held, and the core has told the
    /// host that nothing holds any of its definitions. Answers true; false, doing nothing, for a library unloaded
    /// already; [`Answer::Mistake`] for another host's; [`Answer::Destroyed`] once the destroy has been called.
    pub fn unload_library(&self, library: &LoadedLibrary) -> Result<bool, Answer> {
        if self.shared.is_destroy_called() {
            return Err(Answer::Destroyed);
        }
        if !library.is_of(&self.shared) {
            return Err(Answer::Mistake(
                "the library was loaded by another host".to_owned(),
            ));
        }
        Ok(self.shared.unload_taken(library))
    }

    fn registered_already(
        &self,
        declaration: &LibraryDeclaration,
        withheld: &[&str],
    ) -> Option<String> {
        declaration
            .services
            .iter()
            .filter(|service| !withheld.contains(&service.id.as_str()))
            .find(|service| {
                self.shared.contexts.holds_a_registration_of(&service.id)
                    || self.shared.records().is_taken(&service.id)
            })
            .map(|service| service.id.clone())
    }

    /// Binds every declared function and calls every definition once, before anything is registered; the binary
    /// closes again with the last reference when one is refused.
    fn built(
        &self,
        binary: Arc<LibraryBinary>,
        declaration: &LibraryDeclaration,
        withheld: &[&str],
    ) -> Result<LoadedLibrary, Answer> {
        let lifetime = Arc::new(Lifetime {
            binary: Mutex::new(Some(binary.clone())),
        });
        let holdings = Holdings::of_a_library();
        let objects = declaration
            .services
            .iter()
            .map(|service| {
                let object = if service.is_a_definition() {
                    self.definition(&binary, service, &holdings)?
                } else {
                    bytes_service(&binary, service, &lifetime)?
                };
                Ok((service.id.clone(), object))
            })
            .collect::<Result<Vec<_>, Answer>>()
            .inspect_err(|_| lifetime.close_to_calls())?;
        Ok(LoadedLibrary {
            inner: Arc::new(LoadedInner {
                host: Arc::downgrade(&self.shared),
                declaration: declaration.clone(),
                objects,
                lifetime,
                holdings,
                withheld: withheld.iter().map(|id| (*id).to_owned()).collect(),
                contracts: Mutex::new(Vec::new()),
                registrations: Mutex::new(Some(Vec::new())),
            }),
        })
    }

    /// Checks and reads each contract the library publishes, then publishes every one, refusing a capability whose
    /// contract another live library publishes.
    fn publish_contracts(&self, library: &LoadedLibrary) -> Result<(), Answer> {
        let binary = library.inner.lifetime.binary().ok_or(Answer::Destroyed)?;
        for contract in &library.declaration().contracts {
            let Some(check) = &self.shared.description_check else {
                return Err(Answer::Mistake(format!(
                    "the contract of {} is published by its description, and the host was given no description \
                     check to run on it: create it with HostOptions::description_check",
                    contract.capability
                )));
            };
            let described = binary.described(&contract.description, 0)?;
            check.check_contract(&described)?;
            let (operations, enums) = contract_of(&described).map_err(Answer::Mistake)?;
            self.shared
                .contexts
                .publish_contract(&contract.capability, operations, enums)?;
            locked(&library.inner.contracts).push(contract.capability.clone());
        }
        Ok(())
    }

    fn definition(
        &self,
        binary: &Arc<LibraryBinary>,
        service: &LibraryServiceDeclaration,
        holdings: &Arc<Holdings>,
    ) -> Result<Object, Answer> {
        let address = binary.definition(&service.functions[0].symbol)?;
        if let Some(symbol) = &service.description {
            let Some(check) = &self.shared.description_check else {
                return Err(Answer::Mistake(format!(
                    "the service {} declares a description, and the host was given no description check to run on \
                     it: create it with HostOptions::description_check",
                    service.id
                )));
            };
            let described = binary.described(symbol, address)?;
            check.check(&described)?;
            self.shared.seam.remember_described(&described, binary);
            let mapping = Mapping::Library(Arc::downgrade(binary));
            let bound =
                bind_described(&described, mapping, holdings.clone()).map_err(Answer::Mistake)?;
            return Ok(Arc::new(bound));
        }
        Ok(Arc::new(ForeignObject::at(address)))
    }

    /// Keeps the library among the host's before anything of it is registered, so every later ending of it finds it,
    /// unless the destroy was called or an id of it is taken: the check and the keeping are one step under the lock.
    fn kept(&self, library: &LoadedLibrary) -> Result<(), Answer> {
        let mut records = self.shared.records();
        let refused = if records.destroy.is_some() {
            Some(Answer::Destroyed)
        } else {
            library
                .declaration()
                .services
                .iter()
                .find(|service| records.is_taken(&service.id))
                .map(|service| Answer::Mistake(taken_id_text(&service.id)))
        };
        if let Some(refused) = refused {
            drop(records);
            library.inner.lifetime.close_to_calls();
            return Err(refused);
        }
        records.libraries.push(library.clone());
        Ok(())
    }

    fn register_services(&self, library: &LoadedLibrary) -> Result<(), Answer> {
        for (id, object) in &library.inner.objects {
            if library.is_withheld(id) {
                continue;
            }
            let registration = if object.is::<LibraryService>() {
                self.shared.contexts.register_host_service(
                    id,
                    object.clone(),
                    || Ok(()),
                    || change_count(&mut self.shared.records().host_services, id, true),
                )?
            } else {
                self.register_definition(library, id, object)?
            };
            if let Err(late) = library.keep(id, registration) {
                self.shared.unregister(&late, id);
            }
        }
        Ok(())
    }

    /// Registers a definition awaiting releases, holding the library's binary open from before the core is asked
    /// until the core reports that nothing holds the registration any more, or the destroy ends it.
    fn register_definition(
        &self,
        library: &LoadedLibrary,
        id: &str,
        object: &Object,
    ) -> Result<ServiceRegistration, Answer> {
        let binary = library.inner.lifetime.binary().ok_or(Answer::Destroyed)?;
        let hold = Arc::new(Mutex::new(Some(binary)));
        let released = hold.clone();
        let registered = self
            .shared
            .contexts
            .register_host_service_awaiting_releases(
                id,
                object.clone(),
                || change_count(&mut self.shared.records().host_services, id, true),
                move || drop(locked(&released).take()),
            );
        if registered.is_err() {
            drop(locked(&hold).take());
        }
        registered
    }
}

/// What tells the sink that a library's binary did not close, unless the host's destroy has finished.
fn refused_close(shared: &Arc<HostShared>, import_name: &str) -> crate::core::RefusedClose {
    let host = Arc::downgrade(shared);
    let text = format!("the C library {import_name} could not be closed");
    Box::new(move || {
        if let Some(shared) = host.upgrade()
            && !shared.records().destroy_finished
        {
            shared.report(FailureKind::LibraryUnload, text.clone());
        }
    })
}

fn bytes_service(
    binary: &LibraryBinary,
    service: &LibraryServiceDeclaration,
    lifetime: &Arc<Lifetime>,
) -> Result<Object, Answer> {
    let functions = service
        .functions
        .iter()
        .map(|function| {
            Ok((
                function.name.clone(),
                binary.bytes_function(&function.symbol)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, Answer>>()?;
    Ok(Arc::new(LibraryService {
        id: service.id.clone(),
        functions,
        lifetime: lifetime.clone(),
    }))
}

impl HostShared {
    /// Unloads a library unless an unload or the destroy took it first, answering whether this call took it.
    fn unload_taken(&self, library: &LoadedLibrary) -> bool {
        let Some(registrations) = library.take_for_unloading() else {
            return false;
        };
        if let Some(binary) = library.inner.lifetime.binary() {
            self.seam.library_unloading(&binary);
        }
        library.inner.holdings.release_all();
        library.inner.lifetime.close_to_calls();
        self.withdraw_contracts(library);
        for (id, registration) in &registrations {
            self.unregister(registration, id);
        }
        self.drop_library(library);
        true
    }

    fn withdraw_contracts(&self, library: &LoadedLibrary) {
        for capability in std::mem::take(&mut *locked(&library.inner.contracts)) {
            self.contexts.withdraw_contract(&capability);
        }
    }

    fn unregister(&self, registration: &ServiceRegistration, id: &str) {
        let unregistered = registration
            .unregister_then(|| change_count(&mut self.records().host_services, id, false));
        if let Err(answer) = unregistered
            && answer != Answer::Destroyed
        {
            self.report(
                FailureKind::LayerDefect,
                format!(
                    "the host did not unregister a service of a C library it unloaded: {answer}"
                ),
            );
        }
    }

    fn drop_library(&self, library: &LoadedLibrary) {
        self.records()
            .libraries
            .retain(|kept| !Arc::ptr_eq(&kept.inner, &library.inner));
    }

    /// The destroy's step 3: closes every live library to calls, each binary closing once nothing holds it. Its
    /// services are not unregistered one by one, since the core's destroy next ends every registration.
    pub(super) fn close_every_library(&self) {
        let libraries = std::mem::take(&mut self.records().libraries);
        for library in libraries {
            if library.take_for_unloading().is_some() {
                if let Some(binary) = library.inner.lifetime.binary() {
                    self.seam.library_unloading(&binary);
                }
                library.inner.holdings.release_all();
                library.inner.lifetime.close_to_calls();
                self.withdraw_contracts(&library);
            }
        }
    }
}
