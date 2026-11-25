//! The one listener on the core that routes each event to the plugin it concerns, by the handle it names.

use super::plugin::Context;
use super::records::{Object, PluginEvent, Record, Revocation};
use super::state::ContextState;
use crate::core::{
    Answer, CapabilityChange, Capacity, CapacityQuestion, Event, EventPayload, FailureKind, Handle,
    Number, RefusalStatus, WatchChange,
};
use crate::handles::HandleKind;

use super::log::LogRecord;

impl ContextState {
    pub(crate) fn route(&self, event: &Event) {
        match &event.payload {
            EventPayload::WantSettled {
                want,
                plugin,
                implementation,
                refusal,
                degraded,
            } => self.settle(event, *want, *plugin, *implementation, refusal, *degraded),
            EventPayload::Watch {
                watch,
                plugin,
                implementation,
                change,
            } => self.watched(event, *watch, *plugin, *implementation, *change),
            EventPayload::Capability {
                watch,
                plugin,
                implementation,
                change,
                ..
            } => self.capability_changed(event, *watch, *plugin, *implementation, *change),
            EventPayload::Revoked {
                plugin,
                implementation,
            } => self.revoked(event, *plugin, *implementation),
            EventPayload::HoldingsReleased {
                owner,
                registration,
            } => self.holdings_released(event, *owner, *registration),
            EventPayload::ReachRefused {
                kind,
                plugin,
                id,
                refusal,
            } => {
                if let Some(context) = self.context_of(*plugin, event) {
                    context.tell(&PluginEvent::ReachRefused {
                        kind: *kind,
                        id: id.clone(),
                        refusal: refusal.clone(),
                    });
                }
            }
            EventPayload::CapacityWanted {
                want,
                registration,
                request,
                question,
                constraints,
                ..
            } if !event.incomplete => self.answer_capacity(
                event,
                *want,
                *registration,
                *request,
                *question,
                constraints,
            ),
            EventPayload::Log { level, text } => self.write(&LogRecord {
                level: *level,
                plugin: None,
                text: text.clone(),
            }),
            _ => {}
        }
    }

    fn settle(
        &self,
        event: &Event,
        want: Handle,
        plugin: Handle,
        implementation: Handle,
        refusal: &crate::core::Refusal,
        degraded: bool,
    ) {
        let Some(Record::Want(record)) = self.handles.end(want).map(|entry| entry.record) else {
            return;
        };
        let room = record.take_holding_room();
        if implementation.0 == 0 {
            record.settle(Err(Answer::Refused {
                status: RefusalStatus::Refused,
                refusal: Some(refusal.clone()),
            }));
            return;
        }
        let Some(service) = self.service_of(implementation, event) else {
            record.settle(Err(Answer::Defect(
                "the core settled a want with a registration the layer holds no object for"
                    .to_owned(),
            )));
            return;
        };
        if let Some(room) = room {
            room.hold(plugin, implementation, service.clone());
        }
        if degraded && let Some(context) = self.context_of(plugin, event) {
            context.tell(&PluginEvent::Degraded {
                id: record.id.clone(),
                service: service.clone(),
                unmet_constraint: refusal.clone(),
            });
        }
        record.settle(Ok(service));
    }

    fn watched(
        &self,
        event: &Event,
        watch: Handle,
        plugin: Handle,
        implementation: Handle,
        change: WatchChange,
    ) {
        let Some(Record::ServiceWatch(watcher)) = self.record_of(watch, HandleKind::Watch) else {
            return;
        };
        let text = "a plugin's service watcher panicked";
        if change == WatchChange::REGISTERED {
            let Some(service) = self.service_of(implementation, event) else {
                return;
            };
            self.hold(plugin, implementation, service.clone());
            self.call_back(
                || watcher.registered(&service),
                FailureKind::PluginCallback,
                text,
                Some(event.sequence),
            );
        } else if change == WatchChange::UNREGISTERED {
            self.call_back(
                || watcher.unregistered(),
                FailureKind::PluginCallback,
                text,
                Some(event.sequence),
            );
        }
    }

    fn capability_changed(
        &self,
        event: &Event,
        watch: Handle,
        plugin: Handle,
        implementation: Handle,
        change: CapabilityChange,
    ) {
        let Some(Record::CapabilityWatch(watcher)) = self.record_of(watch, HandleKind::Watch)
        else {
            return;
        };
        let Some(object) = self.object_of_implementation(implementation) else {
            self.unknown(
                "a capability watch heard of an implementation the layer holds no object for",
                event,
            );
            return;
        };
        let text = "a plugin's capability watcher panicked";
        if change == CapabilityChange::PROVIDED {
            self.hold(plugin, implementation, object.clone());
            self.call_back(
                || watcher.provided(&object),
                FailureKind::PluginCallback,
                text,
                Some(event.sequence),
            );
        } else if change == CapabilityChange::WITHDRAWN {
            self.call_back(
                || watcher.withdrawn(&object),
                FailureKind::PluginCallback,
                text,
                Some(event.sequence),
            );
        }
    }

    /// Drops the holding the revocation names, even for a plugin forgotten, and tells the plugin of it. A holding of a
    /// registration made awaiting releases is kept, revoked, until the plugin lets go of the object.
    fn revoked(&self, event: &Event, plugin: Handle, implementation: Handle) {
        let Some(object) = self.handles.revoke(plugin, implementation) else {
            return;
        };
        let awaits_release = self.releases.awaits(implementation);
        if awaits_release {
            self.releases
                .keep_revoked(plugin, implementation, object.clone());
        }
        if let Some(context) = self.context_of(plugin, event) {
            context.tell(&PluginEvent::Revoked(Revocation {
                object,
                registration: implementation,
                awaits_release,
            }));
        }
    }

    /// Hands the end of a registration's last holding to its owner: the host's own record runs, and a plugin hears it
    /// as it hears any event about it, so one its own release or quarantine queued still reaches it, and one queued
    /// after those events, or after its forget, reaches no one.
    fn holdings_released(&self, event: &Event, owner: Handle, registration: Handle) {
        let Some(awaiting) = self.releases.released(registration) else {
            return;
        };
        if let Some(released) = awaiting.released {
            self.call_back(
                released,
                FailureKind::LayerDefect,
                "the record of a released registration panicked",
                Some(event.sequence),
            );
        }
        if owner == Handle::HOST {
            return;
        }
        if let Some(context) = self.context_of(owner, event) {
            context.tell(&PluginEvent::HoldingsReleased {
                id: awaiting.id,
                service: awaiting.service,
            });
        }
    }

    fn answer_capacity(
        &self,
        event: &Event,
        want: Handle,
        registration: Handle,
        request: u64,
        question: CapacityQuestion,
        constraints: &[Number],
    ) {
        let Some(Record::Service { answerer, .. }) =
            self.record_of(registration, HandleKind::ServiceRegistration)
        else {
            return;
        };
        let mut answer = None;
        self.call_back(
            || {
                answer = match question {
                    CapacityQuestion::ATTRIBUTES => Some(Capacity {
                        request,
                        attributes: answerer
                            .as_ref()
                            .map_or_else(Vec::new, |answerer| answerer.attributes()),
                        serves: false,
                    }),
                    CapacityQuestion::SERVE => Some(Capacity {
                        request,
                        attributes: Vec::new(),
                        serves: answerer
                            .as_ref()
                            .is_none_or(|answerer| answerer.serves(constraints)),
                    }),
                    _ => None,
                }
            },
            FailureKind::PluginCallback,
            "a capacity answerer panicked, so its question went unanswered",
            Some(event.sequence),
        );
        if let Some(answer) = answer {
            self.report_capacity(event, want, registration, &answer);
        }
    }

    fn report_capacity(
        &self,
        event: &Event,
        want: Handle,
        registration: Handle,
        answer: &Capacity,
    ) {
        let Some(now) = self.timer.time() else {
            self.report(
                FailureKind::CapacityUnanswered,
                "a capacity question has no clock to be answered by".to_owned(),
                Some(event.sequence),
            );
            return;
        };
        match self.core.report_capacity(want, registration, answer, now) {
            Ok(()) | Err(Answer::Absent) | Err(Answer::Destroyed) => {}
            Err(other) => self.report(
                FailureKind::CapacityUnanswered,
                format!("the core did not take a capacity answer: {other}"),
                Some(event.sequence),
            ),
        }
    }

    fn service_of(&self, registration: Handle, event: &Event) -> Option<Object> {
        let service = self.object_of_registration(registration);
        if service.is_none() {
            self.unknown(
                "the core handed a plugin a registration the layer holds no object for",
                event,
            );
        }
        service
    }

    fn hold(&self, plugin: Handle, implementation: Handle, object: Object) {
        match self.handles.reserve() {
            Ok(room) => room.hold(plugin, implementation, object),
            Err(answer) => self.report(
                FailureKind::ObjectUnknown,
                format!("a holding could not be kept: {answer}"),
                None,
            ),
        }
    }

    fn unknown(&self, text: &str, event: &Event) {
        self.report(
            FailureKind::ObjectUnknown,
            text.to_owned(),
            Some(event.sequence),
        );
    }

    fn context_of(&self, plugin: Handle, event: &Event) -> Option<Context> {
        match self.record_of(plugin, HandleKind::Plugin) {
            Some(Record::Plugin(context)) if context.hears(event.sequence) => Some(context),
            _ => None,
        }
    }
}
