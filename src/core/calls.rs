//! Every call of the core as one method of [`Core`], each handed to the wrapper. A method takes the crate's own
//! values, lays them out as the core's structs and reads what the core answers back into the crate's own values.

use std::ffi::c_char;

use super::abi;
use super::answer::Answer;
use super::codes::{Axis, EdgeSource, HandleCondition, PluginStatus, ReachKind, RefusalCode};
use super::marshal::{Arena, checked_text, choice};
use super::read::{self, Reader};
use super::values::{
    ActivationPlan, CapabilityImplementation, Capacity, DeclarationRefusal, Edge, Handle,
    LedgerRow, LedgerSnapshot, Manifest, PluginRefusal, QuarantineReason, Support, WantOptions,
};
use super::wrapper::{Core, Failure, FailureKind, Hold, Parts};

type HandleCall =
    unsafe extern "C" fn(*mut std::ffi::c_void, u64, *const c_char, *mut u64, *mut u32) -> i32;

impl Core {
    /// The core's earliest deadline, in the caller's milliseconds; [`Answer::Absent`] when nothing waits on time.
    pub fn next_deadline(&self) -> Result<i64, Answer> {
        self.hold(|hold| hold.next_deadline())
    }

    /// Hands the core the caller's time now, so it acts on every deadline that has fallen due.
    pub fn advance(&self, now: i64) -> Result<(), Answer> {
        self.hold_handing_the_time(|hold| hold.advance(now))
    }

    /// Whether the core supports `manifest`, and if not, on which axis and why.
    pub fn supports(&self, manifest: &Manifest) -> Result<Support, Answer> {
        self.hold(|hold| hold.supports(manifest))
    }

    /// Registers a plugin's manifest, answering the plugin's handle.
    pub fn register_manifest(&self, manifest: &Manifest) -> Result<Handle, Answer> {
        self.hold(|hold| hold.register_manifest(manifest))
    }

    /// Plans the activation of registered plugins: each provider before its consumers, cycles and refusals as data.
    /// `host_provided` names the services the host itself provides, `None` for none.
    pub fn plan_activation(
        &self,
        plugins: &[Handle],
        edge_source: EdgeSource,
        host_provided: Option<&[String]>,
    ) -> Result<ActivationPlan, Answer> {
        self.hold(|hold| hold.plan_activation(plugins, edge_source, host_provided))
    }

    /// Opens a registered plugin, giving the core the caller's own token for it.
    pub fn open_plugin(&self, plugin: Handle, payload: u64) -> Result<(), Answer> {
        self.hold(|hold| hold.open_plugin(plugin, payload))
    }

    /// The token the caller gave when it opened the plugin.
    pub fn plugin_payload(&self, plugin: Handle) -> Result<u64, Answer> {
        self.hold(|hold| hold.plugin_payload(plugin))
    }

    /// Releases an open plugin.
    pub fn release_plugin(&self, plugin: Handle) -> Result<(), Answer> {
        self.hold(|hold| hold.release_plugin(plugin))
    }

    /// Quarantines a plugin, with why when the host knows it.
    pub fn quarantine_plugin(
        &self,
        plugin: Handle,
        reason: Option<&QuarantineReason>,
    ) -> Result<(), Answer> {
        self.hold(|hold| hold.quarantine_plugin(plugin, reason))
    }

    /// Forgets a released or quarantined plugin.
    pub fn forget_plugin(&self, plugin: Handle) -> Result<(), Answer> {
        self.hold(|hold| hold.forget_plugin(plugin))
    }

    /// Registers a service `owner` provides under `id`, answering the registration.
    pub fn register_service(&self, owner: Handle, id: &str) -> Result<Handle, Answer> {
        self.hold(|hold| hold.register_service(owner, id))
    }

    /// Registers a service under `id` that waits for every holding of an earlier registration to be released.
    pub fn register_service_awaiting_releases(
        &self,
        owner: Handle,
        id: &str,
    ) -> Result<Handle, Answer> {
        self.hold(|hold| hold.register_service_awaiting_releases(owner, id))
    }

    /// Releases what `holder` holds of `registration`.
    pub fn release_holding(&self, holder: Handle, registration: Handle) -> Result<(), Answer> {
        self.hold(|hold| hold.release_holding(holder, registration))
    }

    /// Unregisters a registration, a want or a watch.
    pub fn unregister(&self, handle: Handle) -> Result<(), Answer> {
        self.hold(|hold| hold.unregister(handle))
    }

    /// The condition of whatever `handle` names.
    pub fn handle_state(&self, handle: Handle) -> Result<HandleCondition, Answer> {
        self.hold(|hold| hold.handle_state(handle))
    }

    /// The service registered under `id` that `plugin` may reach; [`Answer::Absent`] when none is.
    pub fn get_service(&self, plugin: Handle, id: &str) -> Result<Handle, Answer> {
        self.hold(|hold| hold.get_service(plugin, id))
    }

    /// Wants the service `id` for `plugin`, answering the want, which settles exactly once by an event.
    pub fn want_service(
        &self,
        plugin: Handle,
        id: &str,
        options: &WantOptions,
    ) -> Result<Handle, Answer> {
        self.hold(|hold| hold.want_service(plugin, id, options))
    }

    /// Answers a capacity question the core put to `registration` for `want`.
    pub fn report_capacity(
        &self,
        want: Handle,
        registration: Handle,
        capacity: &Capacity,
        now: i64,
    ) -> Result<(), Answer> {
        self.hold(|hold| hold.report_capacity(want, registration, capacity, now))
    }

    /// Watches the service `id` for `plugin`, answering the watch.
    pub fn watch_service(&self, plugin: Handle, id: &str) -> Result<Handle, Answer> {
        self.hold(|hold| hold.watch_service(plugin, id))
    }

    /// Every service id `plugin` may reach.
    pub fn service_ids(&self, plugin: Handle) -> Result<Vec<String>, Answer> {
        self.hold(|hold| hold.service_ids(plugin))
    }

    /// Every service `plugin` declares that nothing provides yet.
    pub fn unbound_services(&self, plugin: Handle) -> Result<Vec<String>, Answer> {
        self.hold(|hold| hold.unbound_services(plugin))
    }

    /// Provides the capability `id` from `plugin`, answering the implementation.
    pub fn provide_capability(&self, plugin: Handle, id: &str) -> Result<Handle, Answer> {
        self.hold(|hold| hold.provide_capability(plugin, id))
    }

    /// Verifies that an open plugin did what its manifest declares.
    pub fn verify_activation(&self, plugin: Handle) -> Result<(), Answer> {
        self.hold(|hold| hold.verify_activation(plugin))
    }

    /// Every verified implementation of the capability `id` that `plugin` may reach.
    pub fn all_capabilities(
        &self,
        plugin: Handle,
        id: &str,
    ) -> Result<Vec<CapabilityImplementation>, Answer> {
        self.hold(|hold| hold.all_capabilities(plugin, id))
    }

    /// Watches the capability `id` for `plugin`, answering the watch.
    pub fn watch_capability(&self, plugin: Handle, id: &str) -> Result<Handle, Answer> {
        self.hold(|hold| hold.watch_capability(plugin, id))
    }

    /// The whole of the core's ledger.
    pub fn ledger(&self) -> Result<LedgerSnapshot, Answer> {
        self.hold(|hold| hold.ledger())
    }

    /// One plugin's row of the core's ledger.
    pub fn ledger_entry(&self, plugin: Handle) -> Result<LedgerRow, Answer> {
        self.hold(|hold| hold.ledger_entry(plugin))
    }
}

impl Hold<'_> {
    pub(crate) fn next_deadline(&mut self) -> Result<i64, Answer> {
        let mut deadline = 0i64;
        self.invoke(Parts::BuiltByTheLayer, |functions, core, pending| {
            // SAFETY: `core` is live and every out pointer is the crate's own.
            unsafe { (functions.weivel_next_deadline)(core, &mut deadline, pending) }
        })?;
        Ok(deadline)
    }

    pub(crate) fn advance(&mut self, now: i64) -> Result<(), Answer> {
        self.invoke(Parts::CarriesTheCallers, |functions, core, pending| {
            // SAFETY: `core` is live.
            unsafe { (functions.weivel_advance)(core, now, pending) }
        })
    }

    pub(crate) fn supports(&mut self, manifest: &Manifest) -> Result<Support, Answer> {
        let mut arena = Arena::default();
        let manifest = arena.manifest(manifest)?;
        let mut answer = abi::answer_struct::<abi::Support>();
        self.invoke(Parts::CarriesTheCallers, |functions, core, pending| {
            // SAFETY: the manifest's texts live in `arena` until the call returns.
            unsafe { (functions.weivel_supports)(core, &manifest, &mut answer, pending) }
        })?;
        let mut reader = Reader::default();
        let support = Support {
            supported: answer.supported != 0,
            axis: Axis(answer.axis),
            // SAFETY: the answer's texts live in its storage until its release below.
            refusal: unsafe { reader.refusal(answer.refusal, answer.subject, answer.has_fix) },
        };
        // SAFETY: the answer was filled by the core and is released exactly once.
        self.released("weivel_support_release", unsafe {
            (self.functions().weivel_support_release)(&mut answer)
        });
        Ok(support)
    }

    pub(crate) fn register_manifest(&mut self, manifest: &Manifest) -> Result<Handle, Answer> {
        let mut arena = Arena::default();
        let manifest = arena.manifest(manifest)?;
        let mut plugin = 0u64;
        self.invoke(Parts::CarriesTheCallers, |functions, core, pending| {
            // SAFETY: the manifest's texts live in `arena` until the call returns.
            unsafe { (functions.weivel_register_manifest)(core, &manifest, &mut plugin, pending) }
        })?;
        Ok(Handle(plugin))
    }

    pub(crate) fn plan_activation(
        &mut self,
        plugins: &[Handle],
        edge_source: EdgeSource,
        host_provided: Option<&[String]>,
    ) -> Result<ActivationPlan, Answer> {
        let mut arena = Arena::default();
        let provided = host_provided
            .map(|ids| arena.strings("a service id the host provides", ids))
            .transpose()?;
        let handles: Vec<u64> = plugins.iter().map(|plugin| plugin.0).collect();
        let mut answer = abi::answer_struct::<abi::ActivationPlan>();
        self.invoke(Parts::CarriesTheCallers, |functions, core, pending| {
            let provided = provided
                .as_ref()
                .map_or(std::ptr::null(), |provided| provided as *const abi::Strings);
            // SAFETY: the handles and the provided ids live until the call returns.
            unsafe {
                (functions.weivel_plan_activation)(
                    core,
                    handles.as_ptr(),
                    handles.len() as u32,
                    edge_source.0,
                    provided,
                    &mut answer,
                    pending,
                )
            }
        })?;
        // SAFETY: the plan's arrays live in its storage until its release below.
        let plan = unsafe { read_plan(&answer) };
        // SAFETY: the plan was filled by the core and is released exactly once.
        self.released("weivel_activation_plan_release", unsafe {
            (self.functions().weivel_activation_plan_release)(&mut answer)
        });
        Ok(plan)
    }

    pub(crate) fn open_plugin(&mut self, plugin: Handle, payload: u64) -> Result<(), Answer> {
        let options = abi::PluginOptions {
            size: abi::size_of::<abi::PluginOptions>(),
            payload,
        };
        self.invoke(Parts::CarriesTheCallers, |functions, core, pending| {
            // SAFETY: `options` lives until the call returns.
            unsafe { (functions.weivel_open_plugin)(core, plugin.0, &options, pending) }
        })
    }

    pub(crate) fn plugin_payload(&mut self, plugin: Handle) -> Result<u64, Answer> {
        let mut payload = 0u64;
        self.invoke(Parts::BuiltByTheLayer, |functions, core, pending| {
            // SAFETY: `core` is live and the out pointer is the crate's own.
            unsafe { (functions.weivel_plugin_payload)(core, plugin.0, &mut payload, pending) }
        })?;
        Ok(payload)
    }

    pub(crate) fn release_plugin(&mut self, plugin: Handle) -> Result<(), Answer> {
        self.invoke(Parts::BuiltByTheLayer, |functions, core, pending| {
            // SAFETY: `core` is live.
            unsafe { (functions.weivel_release_plugin)(core, plugin.0, pending) }
        })
    }

    pub(crate) fn quarantine_plugin(
        &mut self,
        plugin: Handle,
        reason: Option<&QuarantineReason>,
    ) -> Result<(), Answer> {
        let mut arena = Arena::default();
        let built = reason
            .map(|reason| {
                Ok::<_, Answer>(abi::QuarantineReason {
                    size: abi::size_of::<abi::QuarantineReason>(),
                    code: reason.code.0,
                    detail: arena.text("the quarantine's detail", &reason.detail)?,
                    fix: arena.text("the quarantine's fix", &reason.fix)?,
                })
            })
            .transpose()?;
        self.invoke(Parts::CarriesTheCallers, |functions, core, pending| {
            let reason = built.as_ref().map_or(std::ptr::null(), |reason| {
                reason as *const abi::QuarantineReason
            });
            // SAFETY: the reason's texts live in `arena` until the call returns.
            unsafe { (functions.weivel_quarantine_plugin)(core, plugin.0, reason, pending) }
        })
    }

    pub(crate) fn forget_plugin(&mut self, plugin: Handle) -> Result<(), Answer> {
        self.invoke(Parts::BuiltByTheLayer, |functions, core, pending| {
            // SAFETY: `core` is live.
            unsafe { (functions.weivel_forget_plugin)(core, plugin.0, pending) }
        })
    }

    pub(crate) fn register_service(&mut self, owner: Handle, id: &str) -> Result<Handle, Answer> {
        self.handle_call(owner, id, |functions| functions.weivel_register_service)
    }

    pub(crate) fn register_service_awaiting_releases(
        &mut self,
        owner: Handle,
        id: &str,
    ) -> Result<Handle, Answer> {
        self.handle_call(owner, id, |functions| {
            functions.weivel_register_service_awaiting_releases
        })
    }

    pub(crate) fn release_holding(
        &mut self,
        holder: Handle,
        registration: Handle,
    ) -> Result<(), Answer> {
        self.invoke(Parts::BuiltByTheLayer, |functions, core, pending| {
            // SAFETY: `core` is live.
            unsafe { (functions.weivel_release_holding)(core, holder.0, registration.0, pending) }
        })
    }

    pub(crate) fn unregister(&mut self, handle: Handle) -> Result<(), Answer> {
        self.invoke(Parts::BuiltByTheLayer, |functions, core, pending| {
            // SAFETY: `core` is live.
            unsafe { (functions.weivel_unregister)(core, handle.0, pending) }
        })
    }

    pub(crate) fn handle_state(&mut self, handle: Handle) -> Result<HandleCondition, Answer> {
        let mut condition = 0u32;
        self.invoke(Parts::BuiltByTheLayer, |functions, core, pending| {
            // SAFETY: `core` is live and the out pointer is the crate's own.
            unsafe { (functions.weivel_handle_state)(core, handle.0, &mut condition, pending) }
        })?;
        Ok(HandleCondition(condition))
    }

    pub(crate) fn get_service(&mut self, plugin: Handle, id: &str) -> Result<Handle, Answer> {
        self.handle_call(plugin, id, |functions| functions.weivel_get_service)
    }

    pub(crate) fn want_service(
        &mut self,
        plugin: Handle,
        id: &str,
        options: &WantOptions,
    ) -> Result<Handle, Answer> {
        let id = checked_text("the id", id)?;
        let options = abi::WantOptions {
            size: abi::size_of::<abi::WantOptions>(),
            timed: choice(options.timed),
            now: options.now,
            timeout_millis: options.timeout_millis,
            budgeted: choice(options.budgeted),
            budget_millis: options.budget_millis,
            poll_millis: options.poll_millis,
        };
        let mut want = 0u64;
        self.invoke(Parts::CarriesTheCallers, |functions, core, pending| {
            // SAFETY: the id and the options live until the call returns.
            unsafe {
                (functions.weivel_want_service)(
                    core,
                    plugin.0,
                    id.as_ptr(),
                    &options,
                    &mut want,
                    pending,
                )
            }
        })?;
        Ok(Handle(want))
    }

    pub(crate) fn report_capacity(
        &mut self,
        want: Handle,
        registration: Handle,
        capacity: &Capacity,
        now: i64,
    ) -> Result<(), Answer> {
        let mut arena = Arena::default();
        let built = abi::Capacity {
            size: abi::size_of::<abi::Capacity>(),
            request: capacity.request,
            attributes: arena.numbers("an attribute name of the answer", &capacity.attributes)?,
            serves: u32::from(capacity.serves),
        };
        self.invoke(Parts::CarriesTheCallers, |functions, core, pending| {
            // SAFETY: the answer's names live in `arena` until the call returns.
            unsafe {
                (functions.weivel_report_capacity)(
                    core,
                    want.0,
                    registration.0,
                    &built,
                    now,
                    pending,
                )
            }
        })
    }

    pub(crate) fn watch_service(&mut self, plugin: Handle, id: &str) -> Result<Handle, Answer> {
        self.handle_call(plugin, id, |functions| functions.weivel_watch_service)
    }

    pub(crate) fn service_ids(&mut self, plugin: Handle) -> Result<Vec<String>, Answer> {
        self.service_list(plugin, false)
    }

    pub(crate) fn unbound_services(&mut self, plugin: Handle) -> Result<Vec<String>, Answer> {
        self.service_list(plugin, true)
    }

    pub(crate) fn provide_capability(
        &mut self,
        plugin: Handle,
        id: &str,
    ) -> Result<Handle, Answer> {
        self.handle_call(plugin, id, |functions| functions.weivel_provide_capability)
    }

    pub(crate) fn verify_activation(&mut self, plugin: Handle) -> Result<(), Answer> {
        self.invoke(Parts::BuiltByTheLayer, |functions, core, pending| {
            // SAFETY: `core` is live.
            unsafe { (functions.weivel_verify_activation)(core, plugin.0, pending) }
        })
    }

    pub(crate) fn all_capabilities(
        &mut self,
        plugin: Handle,
        id: &str,
    ) -> Result<Vec<CapabilityImplementation>, Answer> {
        let id = checked_text("the id", id)?;
        let mut answer = abi::answer_struct::<abi::CapabilityList>();
        self.invoke(Parts::CarriesTheCallers, |functions, core, pending| {
            // SAFETY: the id lives until the call returns.
            unsafe {
                (functions.weivel_all_capabilities)(
                    core,
                    plugin.0,
                    id.as_ptr(),
                    &mut answer,
                    pending,
                )
            }
        })?;
        // SAFETY: the list's elements live in its storage until its release below.
        let items: Vec<abi::CapabilityImplementation> =
            unsafe { read::strided(answer.items, answer.count) };
        // SAFETY: the list was filled by the core and is released exactly once.
        self.released("weivel_capability_list_release", unsafe {
            (self.functions().weivel_capability_list_release)(&mut answer)
        });
        Ok(items
            .into_iter()
            .map(|item| CapabilityImplementation {
                implementation: Handle(item.implementation),
                provider: Handle(item.provider),
            })
            .collect())
    }

    pub(crate) fn watch_capability(&mut self, plugin: Handle, id: &str) -> Result<Handle, Answer> {
        self.handle_call(plugin, id, |functions| functions.weivel_watch_capability)
    }

    pub(crate) fn ledger(&mut self) -> Result<LedgerSnapshot, Answer> {
        let mut answer = abi::answer_struct::<abi::LedgerSnapshot>();
        self.invoke(Parts::BuiltByTheLayer, |functions, core, pending| {
            // SAFETY: `core` is live and the snapshot is the crate's own.
            unsafe { (functions.weivel_ledger)(core, &mut answer, pending) }
        })?;
        // SAFETY: the snapshot's rows and refusals live in its storage until its release below.
        let snapshot = unsafe { read_snapshot(&answer) };
        // SAFETY: the snapshot was filled by the core and is released exactly once.
        self.released("weivel_ledger_snapshot_release", unsafe {
            (self.functions().weivel_ledger_snapshot_release)(&mut answer)
        });
        Ok(snapshot)
    }

    pub(crate) fn ledger_entry(&mut self, plugin: Handle) -> Result<LedgerRow, Answer> {
        let mut answer = abi::answer_struct::<abi::LedgerRow>();
        self.invoke(Parts::BuiltByTheLayer, |functions, core, pending| {
            // SAFETY: `core` is live and the row is the crate's own.
            unsafe { (functions.weivel_ledger_entry)(core, plugin.0, &mut answer, pending) }
        })?;
        // SAFETY: the row's texts and lists live in its storage until its release below.
        let row = unsafe { read_row(&answer) };
        // SAFETY: the row was filled by the core and is released exactly once.
        self.released("weivel_ledger_row_release", unsafe {
            (self.functions().weivel_ledger_row_release)(&mut answer)
        });
        Ok(row)
    }

    fn handle_call(
        &mut self,
        plugin: Handle,
        id: &str,
        function: impl FnOnce(&super::table::CoreFunctions) -> HandleCall,
    ) -> Result<Handle, Answer> {
        let id = checked_text("the id", id)?;
        let mut answered = 0u64;
        self.invoke(Parts::CarriesTheCallers, |functions, core, pending| {
            // SAFETY: the id lives until the call returns and the out pointer is the crate's own.
            unsafe { function(functions)(core, plugin.0, id.as_ptr(), &mut answered, pending) }
        })?;
        Ok(Handle(answered))
    }

    fn service_list(&mut self, plugin: Handle, unbound: bool) -> Result<Vec<String>, Answer> {
        let mut answer = abi::answer_struct::<abi::ServiceList>();
        self.invoke(Parts::BuiltByTheLayer, |functions, core, pending| {
            let function = if unbound {
                functions.weivel_unbound_services
            } else {
                functions.weivel_service_ids
            };
            // SAFETY: `core` is live and the list is the crate's own.
            unsafe { function(core, plugin.0, &mut answer, pending) }
        })?;
        // SAFETY: the ids live in the list's storage until its release below.
        let ids = unsafe { Reader::default().strings(answer.ids) };
        // SAFETY: the list was filled by the core and is released exactly once.
        self.released("weivel_service_list_release", unsafe {
            (self.functions().weivel_service_list_release)(&mut answer)
        });
        Ok(ids)
    }

    fn released(&mut self, function: &str, status: i32) {
        if status != abi::STATUS_OK {
            self.note(Failure {
                kind: FailureKind::LayerDefect,
                text: format!("the core refused {function} for an answer the layer read"),
                sequence: None,
            });
        }
    }
}

unsafe fn read_plan(plan: &abi::ActivationPlan) -> ActivationPlan {
    let mut reader = Reader::default();
    // SAFETY: every array lives in the plan's storage, and `refusal_subjects` is NULL or one entry per refusal.
    unsafe {
        let subjects = read::slice(
            plan.refusal_subjects,
            if plan.refusal_subjects.is_null() {
                0
            } else {
                plan.refusal_count
            },
        );
        ActivationPlan {
            order: read::handles(plan.order, plan.order_count),
            edges: read::slice(plan.edges, plan.edge_count)
                .iter()
                .map(|edge| Edge {
                    provider: Handle(edge.provider),
                    consumer: Handle(edge.consumer),
                })
                .collect(),
            cycles: read::slice(plan.cycles, plan.cycle_count)
                .iter()
                .map(|cycle| read::handles(cycle.members, cycle.count))
                .collect(),
            refusals: read::slice(plan.refusals, plan.refusal_count)
                .iter()
                .enumerate()
                .map(|(index, refused)| {
                    let (subject, has_fix) =
                        subjects.get(index).map_or((std::ptr::null(), 0), |found| {
                            (found.subject, found.has_fix)
                        });
                    PluginRefusal {
                        plugin: Handle(refused.plugin),
                        refusal: reader.refusal(refused.refusal, subject, has_fix),
                    }
                })
                .collect(),
        }
    }
}

unsafe fn read_row(row: &abi::LedgerRow) -> LedgerRow {
    let mut reader = Reader::default();
    // SAFETY: every text and list lives in the row's storage, or its snapshot's, until the release.
    unsafe {
        LedgerRow {
            plugin: Handle(row.plugin),
            import_name: reader.text(row.import_name),
            identity: reader.text(row.identity),
            status: PluginStatus(row.status),
            verified: row.verified != 0,
            refusal: reader.refusal(row.refusal, row.subject, row.has_fix),
            capabilities_declared: reader.strings(row.capabilities_declared),
            permissions: reader.strings(row.permissions),
            services_provided: reader.strings(row.services_provided),
            capabilities_provided: reader.strings(row.capabilities_provided),
            services_consumed: reader.strings(row.services_consumed),
            capabilities_consumed: reader.strings(row.capabilities_consumed),
        }
    }
}

unsafe fn read_snapshot(snapshot: &abi::LedgerSnapshot) -> LedgerSnapshot {
    let mut reader = Reader::default();
    // SAFETY: the rows and refusals live in the snapshot's storage, each element of the stride the first declares.
    unsafe {
        let rows: Vec<abi::LedgerRow> = read::strided(snapshot.rows, snapshot.row_count);
        let refusals: Vec<abi::DeclarationRefusal> =
            read::strided(snapshot.refusals, snapshot.refusal_count);
        LedgerSnapshot {
            rows: rows.iter().map(|row| read_row(row)).collect(),
            refusals: refusals
                .iter()
                .map(|refused| DeclarationRefusal {
                    kind: ReachKind(refused.kind),
                    plugin: Handle(refused.plugin),
                    id: reader.text(refused.id),
                    code: RefusalCode(refused.code),
                    subject: reader.optional_text(refused.subject),
                    has_fix: refused.has_fix != 0,
                })
                .collect(),
        }
    }
}
