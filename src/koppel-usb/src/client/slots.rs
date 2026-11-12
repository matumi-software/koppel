//! What each device on the client is doing, as a state machine with no I/O.
//!
//! A device on a server is one slot. A slot is in exactly one [`SlotState`]: attaching, attached,
//! releasing, or idle with a fault (a device with no entry is idle). The client's actor owns the
//! only [`Slots`], feeds it the events that happen (an attach was asked for, a stream was
//! registered, a release finished) and does what it answers. Because every change goes through
//! these methods, a device cannot be attaching and attached at once, and an attach that is no
//! longer wanted finds out from the machine, not from a check made after the fact.
//!
//! Naming: the UI's word for a device (`local`, `shared`, `attached`, `offline`) is
//! [`crate::DeviceState`]; this is the client's own lifecycle for one device.

use std::collections::HashMap;

use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;

use crate::BusId;

/// A device on a server. A bus ID is only a port on one server, so a slot is told apart by the
/// connection epoch as well: the same bus ID on another server, or after the connection changed,
/// is another slot. The epoch is made when the connection changes and carried by everything that
/// started under it, so nothing has to work out which connection it belongs to afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct AttachKey {
    pub epoch: u64,
    pub bus_id: BusId,
}

/// Who made an attachment and how it reaches the kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Kind {
    /// This client's own attach, through this kernel's virtual host controller.
    Kernel,
    /// This client's own attach, made by a driver on this machine through the gateway.
    Driver,
    /// Another machine's driver, through the gateway; it came from this address.
    Visitor { from: String },
}

/// One attachment.
#[derive(Debug, Clone)]
pub(crate) struct Attachment {
    /// Told apart from another attachment of the same device made later.
    pub id: u64,
    /// The virtual port, or the driver's port for a driver attach; `None` for a visitor and while
    /// the driver has not said yet.
    pub port: Option<u32>,
    /// Ends the relay.
    pub stop: CancellationToken,
    pub since: OffsetDateTime,
    pub kind: Kind,
}

/// What the driver holds for one device: the gateway port it was pointed at and its own port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DriverHold {
    pub gateway_port: u16,
    pub driver_port: u32,
}

/// How an attach reaches the kernel, chosen when it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Kernel,
    /// Through the driver. The gateway serves the driver's import, once.
    Driver,
}

#[derive(Debug, Clone)]
pub(crate) enum SlotState {
    /// An attach is in flight. A driver attach is registered by the gateway before the driver
    /// returns its port, so it can carry the attachment already.
    Attaching {
        attempt: u64,
        mode: Mode,
        cancel: CancellationToken,
        /// A driver attach: the gateway has not served the driver's import yet.
        booked: bool,
        registered: Option<Attachment>,
    },
    Attached(Attachment),
    /// Being undone. `attempt` is an attach still in flight that was told to stop and will report
    /// when it returns; `releases` are the undoing steps still running.
    Releasing {
        attempt: Option<u64>,
        releases: u32,
        fault: Option<String>,
    },
    /// Idle, and the last thing that went wrong shows on the device.
    Faulted(String),
}

/// What undoing a slot involves.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    pub attachment: Option<Attachment>,
    pub hold: Option<DriverHold>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.attachment.is_none() && self.hold.is_none()
    }
}

/// Why a registration was turned down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegisterError {
    /// The attach it belongs to is gone: cancelled, replaced or never started.
    Cancelled,
    /// The device is attached or on its way already.
    AlreadyAttached,
}

/// How an attach worker ended.
#[derive(Debug)]
pub(crate) enum Outcome {
    /// It worked. A driver attach says what the driver holds.
    Done(Option<DriverHold>),
    Cancelled,
    Failed(String),
}

/// What the actor does about a finished attach.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Finish {
    /// Nothing more.
    Nothing,
    /// The attach failed and the device shows why.
    Failed { reason: String },
    /// The attach finished after the device was released: undo what the driver made.
    ReleaseHold(DriverHold),
}

#[derive(Default)]
pub(crate) struct Slots {
    map: HashMap<AttachKey, SlotState>,
    /// What each driver holds. Kept apart from the states because a driver goes on attaching a
    /// device by itself after its connection drops, so a hold outlives the attachment until it is
    /// told to stop.
    holds: HashMap<AttachKey, DriverHold>,
    next: u64,
}

impl Slots {
    pub fn new() -> Self {
        Self::default()
    }

    /// A fresh id for an attempt or an attachment.
    pub fn next_id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    #[cfg(test)]
    pub fn state(&self, key: &AttachKey) -> Option<&SlotState> {
        self.map.get(key)
    }

    #[cfg(test)]
    pub fn hold(&self, key: &AttachKey) -> Option<DriverHold> {
        self.holds.get(key).copied()
    }

    /// The attachment shown as attached here, if any.
    pub fn attachment(&self, key: &AttachKey) -> Option<&Attachment> {
        match self.map.get(key)? {
            SlotState::Attached(a) => Some(a),
            SlotState::Attaching { registered, .. } => registered.as_ref(),
            _ => None,
        }
    }

    pub fn fault(&self, key: &AttachKey) -> Option<&str> {
        match self.map.get(key)? {
            SlotState::Faulted(reason) => Some(reason),
            SlotState::Releasing { fault, .. } => fault.as_deref(),
            _ => None,
        }
    }

    /// True when an attach or an attachment is there, or being undone: a new attach waits.
    pub fn is_busy(&self, key: &AttachKey) -> bool {
        matches!(
            self.map.get(key),
            Some(
                SlotState::Attaching { .. } | SlotState::Attached(_) | SlotState::Releasing { .. }
            )
        )
    }

    /// Every key that has a state or a hold.
    pub fn keys(&self) -> Vec<AttachKey> {
        let mut keys: Vec<AttachKey> = self.map.keys().chain(self.holds.keys()).cloned().collect();
        keys.sort();
        keys.dedup();
        keys
    }

    /// Clears a fault the person has seen, such as when they ask again.
    pub fn clear_fault(&mut self, key: &AttachKey) {
        if matches!(self.map.get(key), Some(SlotState::Faulted(_))) {
            self.map.remove(key);
        }
    }

    /// Starts an attach unless the device is attached, on its way or being undone. Returns the
    /// attempt's id and its token, a child of `parent`.
    pub fn begin_attach(
        &mut self,
        key: &AttachKey,
        mode: Mode,
        parent: &CancellationToken,
    ) -> Option<(u64, CancellationToken)> {
        if self.is_busy(key) {
            return None;
        }
        let attempt = self.next_id();
        let cancel = parent.child_token();
        self.map.insert(
            key.clone(),
            SlotState::Attaching {
                attempt,
                mode,
                cancel: cancel.clone(),
                booked: mode == Mode::Driver,
                registered: None,
            },
        );
        Some((attempt, cancel))
    }

    /// The gateway serves the driver's import for this device, once. Returns the attempt it
    /// belongs to.
    pub fn claim_booking(&mut self, key: &AttachKey) -> Option<u64> {
        match self.map.get_mut(key)? {
            SlotState::Attaching {
                attempt,
                booked,
                cancel,
                ..
            } if *booked && !cancel.is_cancelled() => {
                *booked = false;
                Some(*attempt)
            }
            _ => None,
        }
    }

    /// An attachment is made: by this client's attach `attempt`, or by a visitor (`None`).
    pub fn register(
        &mut self,
        key: &AttachKey,
        attempt: Option<u64>,
        attachment: Attachment,
    ) -> Result<(), RegisterError> {
        match (self.map.get_mut(key), attempt) {
            (
                Some(SlotState::Attaching {
                    attempt: current,
                    mode,
                    cancel,
                    registered,
                    ..
                }),
                Some(wanted),
            ) if *current == wanted && !cancel.is_cancelled() => {
                if *mode == Mode::Driver {
                    // Complete once the driver returns its port.
                    *registered = Some(attachment);
                } else {
                    self.map
                        .insert(key.clone(), SlotState::Attached(attachment));
                }
                Ok(())
            }
            (_, Some(_)) => Err(RegisterError::Cancelled),
            (None | Some(SlotState::Faulted(_)), None) => {
                self.map
                    .insert(key.clone(), SlotState::Attached(attachment));
                Ok(())
            }
            (Some(_), None) => Err(RegisterError::AlreadyAttached),
        }
    }

    /// An attach worker ended.
    pub fn finish_attach(&mut self, key: &AttachKey, attempt: u64, outcome: Outcome) -> Finish {
        let hold = match &outcome {
            Outcome::Done(Some(hold)) => Some(*hold),
            _ => None,
        };
        match self.map.remove(key) {
            Some(SlotState::Attaching {
                attempt: current,
                registered,
                ..
            }) if current == attempt => match outcome {
                Outcome::Done(hold) => {
                    if let Some(hold) = hold {
                        self.holds.insert(key.clone(), hold);
                    }
                    if let Some(mut attachment) = registered {
                        attachment.port = hold.map(|h| h.driver_port).or(attachment.port);
                        self.map
                            .insert(key.clone(), SlotState::Attached(attachment));
                    }
                    Finish::Nothing
                }
                Outcome::Cancelled => {
                    if let Some(attachment) = registered {
                        attachment.stop.cancel();
                    }
                    Finish::Nothing
                }
                Outcome::Failed(reason) => {
                    if let Some(attachment) = registered {
                        // The driver may have attached it by itself, which is what was wanted.
                        self.map
                            .insert(key.clone(), SlotState::Attached(attachment));
                        return Finish::Nothing;
                    }
                    self.map
                        .insert(key.clone(), SlotState::Faulted(reason.clone()));
                    Finish::Failed { reason }
                }
            },
            Some(SlotState::Releasing {
                attempt: Some(current),
                releases,
                fault,
            }) if current == attempt => {
                // Told to stop while in flight: what it made is undone, by a release of its own.
                let release = hold.is_some();
                let releases = releases + u32::from(release);
                self.settle(
                    key,
                    SlotState::Releasing {
                        attempt: None,
                        releases,
                        fault,
                    },
                );
                match hold {
                    Some(hold) => Finish::ReleaseHold(hold),
                    None => Finish::Nothing,
                }
            }
            other => {
                // The slot moved on without this attach. A driver port it made is still its
                // to undo, and until that is done the device is being released.
                match (other, hold) {
                    (None | Some(SlotState::Faulted(_)), Some(hold)) => {
                        self.map.insert(
                            key.clone(),
                            SlotState::Releasing {
                                attempt: None,
                                releases: 1,
                                fault: None,
                            },
                        );
                        Finish::ReleaseHold(hold)
                    }
                    (other, hold) => {
                        if let Some(other) = other {
                            self.map.insert(key.clone(), other);
                        }
                        hold.map_or(Finish::Nothing, Finish::ReleaseHold)
                    }
                }
            }
        }
    }

    /// Records a failure the device shows, for a gateway request. A device that is busy keeps its
    /// state.
    pub fn record_fault(&mut self, key: &AttachKey, reason: String) {
        if matches!(self.map.get(key), None | Some(SlotState::Faulted(_))) {
            self.map.insert(key.clone(), SlotState::Faulted(reason));
        }
    }

    /// The relay of an attachment ended, so the device is no longer attached. Returns the
    /// attachment when it was this very one.
    pub fn relay_ended(&mut self, key: &AttachKey, attachment_id: u64) -> Option<Attachment> {
        match self.map.get_mut(key)? {
            SlotState::Attached(a) if a.id == attachment_id => {}
            SlotState::Attaching { registered, .. }
                if registered.as_ref().is_some_and(|a| a.id == attachment_id) => {}
            _ => return None,
        }
        match self.map.remove(key)? {
            SlotState::Attached(a) => Some(a),
            SlotState::Attaching {
                attempt,
                mode,
                cancel,
                booked,
                registered,
            } => {
                self.map.insert(
                    key.clone(),
                    SlotState::Attaching {
                        attempt,
                        mode,
                        cancel,
                        booked,
                        registered: None,
                    },
                );
                registered
            }
            _ => None,
        }
    }

    /// Starts undoing the device: stops an attach in flight, and hands back what is to be
    /// released. An empty plan means there is nothing to undo right now. The slot is releasing
    /// while a plan is running or an attach in flight has not reported.
    pub fn begin_release(&mut self, key: &AttachKey) -> Plan {
        let state = self.map.remove(key);
        let hold = self.holds.remove(key);
        let (attachment, attempt, fault) = match state {
            Some(SlotState::Attaching {
                attempt,
                cancel,
                registered,
                ..
            }) => {
                cancel.cancel();
                (registered, Some(attempt), None)
            }
            Some(SlotState::Attached(a)) => (Some(a), None, None),
            Some(SlotState::Releasing {
                attempt,
                releases,
                fault,
            }) => {
                // Already being undone; what is left to undo is only a hold that came back.
                let started = hold.is_some();
                self.settle(
                    key,
                    SlotState::Releasing {
                        attempt,
                        releases: releases + u32::from(started),
                        fault,
                    },
                );
                return Plan {
                    attachment: None,
                    hold,
                };
            }
            Some(SlotState::Faulted(_)) | None => (None, None, None),
        };
        let plan = Plan { attachment, hold };
        let releases = u32::from(!plan.is_empty());
        self.settle(
            key,
            SlotState::Releasing {
                attempt,
                releases,
                fault,
            },
        );
        plan
    }

    /// A release step ended. `failure` shows on the device; `hold` is what the driver still
    /// holds when it could not be told to stop.
    pub fn release_finished(
        &mut self,
        key: &AttachKey,
        failure: Option<String>,
        hold: Option<DriverHold>,
    ) {
        if let Some(hold) = hold {
            self.holds.insert(key.clone(), hold);
        }
        match self.map.remove(key) {
            Some(SlotState::Releasing {
                attempt,
                releases,
                fault,
            }) => self.settle(
                key,
                SlotState::Releasing {
                    attempt,
                    releases: releases.saturating_sub(1),
                    fault: failure.or(fault),
                },
            ),
            Some(other) => {
                self.map.insert(key.clone(), other);
            }
            None => {
                if let Some(failure) = failure {
                    self.map.insert(key.clone(), SlotState::Faulted(failure));
                }
            }
        }
    }

    /// Puts `state` in the slot, or settles a releasing slot that has nothing left running:
    /// idle, or faulted with what went wrong.
    fn settle(&mut self, key: &AttachKey, state: SlotState) {
        match state {
            SlotState::Releasing {
                attempt: None,
                releases: 0,
                fault,
            } => {
                if let Some(reason) = fault {
                    self.map.insert(key.clone(), SlotState::Faulted(reason));
                }
            }
            state => {
                self.map.insert(key.clone(), state);
            }
        }
    }

    /// True while a release or an attach told to stop has not reported for this key.
    #[cfg(test)]
    pub fn is_releasing(&self, key: &AttachKey) -> bool {
        matches!(self.map.get(key), Some(SlotState::Releasing { .. }))
    }

    /// The number of release steps running, over every slot.
    #[cfg(test)]
    pub fn releases_running(&self) -> u32 {
        self.map
            .values()
            .map(|s| match s {
                SlotState::Releasing { releases, .. } => *releases,
                _ => 0,
            })
            .sum()
    }

    /// Forgets faults, as a new connection starts clean.
    pub fn clear_faults(&mut self) {
        self.map.retain(|_, s| !matches!(s, SlotState::Faulted(_)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(epoch: u64) -> AttachKey {
        AttachKey {
            epoch,
            bus_id: "1-1.4".parse().unwrap(),
        }
    }

    fn attachment(slots: &mut Slots, kind: Kind) -> Attachment {
        Attachment {
            id: slots.next_id(),
            port: Some(8),
            stop: CancellationToken::new(),
            since: OffsetDateTime::UNIX_EPOCH,
            kind,
        }
    }

    #[test]
    fn a_device_is_never_attaching_twice() {
        let mut slots = Slots::new();
        let root = CancellationToken::new();

        assert!(slots.begin_attach(&key(1), Mode::Kernel, &root).is_some());
        assert!(slots.begin_attach(&key(1), Mode::Kernel, &root).is_none());
        // Another epoch is another device.
        assert!(slots.begin_attach(&key(2), Mode::Kernel, &root).is_some());
    }

    #[test]
    fn a_kernel_attach_is_attached_once_it_registers() {
        let mut slots = Slots::new();
        let (attempt, _) = slots
            .begin_attach(&key(1), Mode::Kernel, &CancellationToken::new())
            .unwrap();
        let attached = attachment(&mut slots, Kind::Kernel);

        slots.register(&key(1), Some(attempt), attached).unwrap();

        assert!(matches!(slots.state(&key(1)), Some(SlotState::Attached(_))));
        // The worker's own report afterwards changes nothing.
        assert_eq!(
            slots.finish_attach(&key(1), attempt, Outcome::Done(None)),
            Finish::Nothing
        );
        assert!(slots.attachment(&key(1)).is_some());
    }

    #[test]
    fn a_registration_from_an_attach_that_was_cancelled_is_turned_down() {
        let mut slots = Slots::new();
        let (attempt, _) = slots
            .begin_attach(&key(1), Mode::Kernel, &CancellationToken::new())
            .unwrap();
        slots.begin_release(&key(1));
        let attached = attachment(&mut slots, Kind::Kernel);

        let registered = slots.register(&key(1), Some(attempt), attached);

        assert_eq!(registered, Err(RegisterError::Cancelled));
    }

    #[test]
    fn a_driver_attach_shows_attached_before_the_driver_returns_its_port() {
        let mut slots = Slots::new();
        let (attempt, _) = slots
            .begin_attach(&key(1), Mode::Driver, &CancellationToken::new())
            .unwrap();
        assert_eq!(slots.claim_booking(&key(1)), Some(attempt));
        assert_eq!(
            slots.claim_booking(&key(1)),
            None,
            "the booking serves once"
        );
        let mut mine = attachment(&mut slots, Kind::Driver);
        mine.port = None;
        slots.register(&key(1), Some(attempt), mine).unwrap();
        assert!(slots.attachment(&key(1)).is_some());

        let hold = DriverHold {
            gateway_port: 3242,
            driver_port: 1,
        };
        let finish = slots.finish_attach(&key(1), attempt, Outcome::Done(Some(hold)));

        assert_eq!(finish, Finish::Nothing);
        assert_eq!(slots.attachment(&key(1)).unwrap().port, Some(1));
        assert_eq!(slots.hold(&key(1)), Some(hold));
    }

    #[test]
    fn releasing_while_the_driver_is_still_attaching_undoes_what_it_makes_when_it_returns() {
        let mut slots = Slots::new();
        let (attempt, cancel) = slots
            .begin_attach(&key(1), Mode::Driver, &CancellationToken::new())
            .unwrap();

        let plan = slots.begin_release(&key(1));
        assert!(plan.is_empty(), "the driver's port is not known yet");
        assert!(cancel.is_cancelled());
        assert!(slots.is_releasing(&key(1)));

        let hold = DriverHold {
            gateway_port: 3242,
            driver_port: 1,
        };
        let finish = slots.finish_attach(&key(1), attempt, Outcome::Done(Some(hold)));
        assert_eq!(finish, Finish::ReleaseHold(hold));
        assert!(slots.is_releasing(&key(1)));

        slots.release_finished(&key(1), None, None);
        assert!(slots.state(&key(1)).is_none());
    }

    #[test]
    fn a_failed_attach_shows_its_reason() {
        let mut slots = Slots::new();
        let root = CancellationToken::new();
        let (attempt, _) = slots.begin_attach(&key(1), Mode::Kernel, &root).unwrap();

        let first = slots.finish_attach(&key(1), attempt, Outcome::Failed("busy".into()));
        assert_eq!(
            first,
            Finish::Failed {
                reason: "busy".into()
            }
        );
        assert_eq!(slots.fault(&key(1)), Some("busy"));

        // A gateway request that fails on an idle device shows too; one on a busy device does not.
        slots.clear_fault(&key(1));
        slots.record_fault(&key(1), "again".into());
        assert_eq!(slots.fault(&key(1)), Some("again"));
        slots.clear_fault(&key(1));
        let _ = slots.begin_attach(&key(1), Mode::Kernel, &root);
        slots.record_fault(&key(1), "ignored".into());
        assert_eq!(slots.fault(&key(1)), None);
    }

    #[test]
    fn a_relay_that_ends_frees_only_its_own_attachment() {
        let mut slots = Slots::new();
        let (attempt, _) = slots
            .begin_attach(&key(1), Mode::Kernel, &CancellationToken::new())
            .unwrap();
        let first = attachment(&mut slots, Kind::Kernel);
        let first_id = first.id;
        slots.register(&key(1), Some(attempt), first).unwrap();
        slots.begin_release(&key(1));
        slots.release_finished(&key(1), None, None);
        let (attempt, _) = slots
            .begin_attach(&key(1), Mode::Kernel, &CancellationToken::new())
            .unwrap();
        let second = attachment(&mut slots, Kind::Kernel);
        slots.register(&key(1), Some(attempt), second).unwrap();

        // The first attach's relay ends late.
        assert!(slots.relay_ended(&key(1), first_id).is_none());
        assert!(slots.attachment(&key(1)).is_some());
    }

    #[test]
    fn a_visitor_cannot_take_a_device_that_is_attached() {
        let mut slots = Slots::new();
        let first = attachment(
            &mut slots,
            Kind::Visitor {
                from: "10.0.0.2".into(),
            },
        );
        slots.register(&key(1), None, first).unwrap();
        let second = attachment(
            &mut slots,
            Kind::Visitor {
                from: "10.0.0.3".into(),
            },
        );

        assert_eq!(
            slots.register(&key(1), None, second),
            Err(RegisterError::AlreadyAttached)
        );
    }

    #[test]
    fn a_hold_without_an_attachment_is_released_with_the_next_release() {
        let mut slots = Slots::new();
        let (attempt, _) = slots
            .begin_attach(&key(1), Mode::Driver, &CancellationToken::new())
            .unwrap();
        let hold = DriverHold {
            gateway_port: 3242,
            driver_port: 1,
        };
        slots.finish_attach(&key(1), attempt, Outcome::Done(Some(hold)));
        assert!(slots.state(&key(1)).is_none());

        let plan = slots.begin_release(&key(1));

        assert_eq!(plan.hold, Some(hold));
        assert_eq!(slots.hold(&key(1)), None);
    }

    #[test]
    fn a_release_that_fails_leaves_the_fault_and_gives_the_hold_back() {
        let mut slots = Slots::new();
        let hold = DriverHold {
            gateway_port: 3242,
            driver_port: 1,
        };
        let (attempt, _) = slots
            .begin_attach(&key(1), Mode::Driver, &CancellationToken::new())
            .unwrap();
        let mut mine = attachment(&mut slots, Kind::Driver);
        mine.port = None;
        slots.register(&key(1), Some(attempt), mine).unwrap();
        slots.finish_attach(&key(1), attempt, Outcome::Done(Some(hold)));
        let plan = slots.begin_release(&key(1));
        assert_eq!(plan.hold, Some(hold));

        slots.release_finished(&key(1), Some("usbip.exe failed".into()), Some(hold));

        assert_eq!(slots.fault(&key(1)), Some("usbip.exe failed"));
        assert_eq!(slots.hold(&key(1)), Some(hold));
    }
}

#[cfg(test)]
mod model {
    //! A model of the machine, driven by random events. The model says in plain terms what each
    //! event should do to one device; the machine must agree, and its invariants must hold.

    use proptest::prelude::*;
    use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Abstract {
        Idle,
        Faulted,
        Attaching,
        Attached,
        Releasing,
    }

    #[derive(Debug, Clone)]
    enum Op {
        Begin(Mode),
        /// The gateway or the worker registers the current attempt.
        Register,
        FinishDone,
        FinishFailed,
        FinishCancelled,
        RelayEnds,
        Release,
        ReleaseDone,
        ReleaseFailed,
    }

    #[derive(Debug, Clone)]
    struct Model {
        state: Abstract,
        mode: Mode,
        /// An attach worker is in flight.
        worker: bool,
        /// A release step is in flight.
        releases: u32,
        /// The registered attachment exists (driver attach in flight, or attached).
        registered: bool,
        /// The driver holds a port for the device.
        hold: bool,
        /// A release step failed while the device was being released.
        rfault: bool,
        /// The device is being released and an attach told to stop has not reported.
        attempt: bool,
    }

    struct Reference;

    impl ReferenceStateMachine for Reference {
        type State = Model;
        type Transition = Op;

        fn init_state() -> BoxedStrategy<Model> {
            Just(Model {
                state: Abstract::Idle,
                mode: Mode::Kernel,
                worker: false,
                releases: 0,
                registered: false,
                hold: false,
                rfault: false,
                attempt: false,
            })
            .boxed()
        }

        fn transitions(model: &Model) -> BoxedStrategy<Op> {
            let mut ops: Vec<BoxedStrategy<Op>> = vec![
                Just(Op::Begin(Mode::Kernel)).boxed(),
                Just(Op::Begin(Mode::Driver)).boxed(),
                Just(Op::Release).boxed(),
                Just(Op::RelayEnds).boxed(),
            ];
            if model.worker {
                ops.push(Just(Op::Register).boxed());
                ops.push(Just(Op::FinishDone).boxed());
                ops.push(Just(Op::FinishFailed).boxed());
                ops.push(Just(Op::FinishCancelled).boxed());
            }
            if model.releases > 0 {
                ops.push(Just(Op::ReleaseDone).boxed());
                ops.push(Just(Op::ReleaseFailed).boxed());
            }
            proptest::strategy::Union::new(ops).boxed()
        }

        fn apply(mut model: Model, op: &Op) -> Model {
            let driver = model.mode == Mode::Driver;
            match (op, model.state) {
                (Op::Begin(mode), Abstract::Idle | Abstract::Faulted) => {
                    model.state = Abstract::Attaching;
                    model.mode = *mode;
                    model.worker = true;
                    model.registered = false;
                    model.attempt = false;
                }
                (Op::Register, Abstract::Attaching) => {
                    if !model.registered {
                        model.registered = true;
                        if model.mode == Mode::Kernel {
                            model.state = Abstract::Attached;
                        }
                    }
                }
                (Op::FinishDone, Abstract::Attaching | Abstract::Attached) if model.worker => {
                    model.worker = false;
                    model.hold |= driver;
                    model.state = if model.registered {
                        Abstract::Attached
                    } else {
                        Abstract::Idle
                    };
                }
                (Op::FinishFailed, Abstract::Attaching | Abstract::Attached) if model.worker => {
                    model.worker = false;
                    model.state = if model.registered {
                        Abstract::Attached
                    } else {
                        Abstract::Faulted
                    };
                }
                (Op::FinishCancelled, Abstract::Attaching) if model.worker => {
                    model.worker = false;
                    model.state = Abstract::Idle;
                    model.registered = false;
                }
                (Op::FinishCancelled, Abstract::Attached) if model.worker => {
                    model.worker = false;
                }
                (Op::RelayEnds, Abstract::Attached) => {
                    model.state = Abstract::Idle;
                    model.registered = false;
                }
                (Op::RelayEnds, Abstract::Attaching) => {
                    // A driver attach whose connection ended before the driver returned.
                    model.registered = false;
                }
                (Op::Release, Abstract::Attaching | Abstract::Attached) => {
                    model.attempt = model.state == Abstract::Attaching && model.worker;
                    if model.registered || model.hold {
                        model.releases += 1;
                    }
                    model.registered = false;
                    model.hold = false;
                    model.rfault = false;
                    model.state = Abstract::Releasing;
                }
                (Op::Release, Abstract::Idle | Abstract::Faulted) => {
                    if model.hold {
                        model.hold = false;
                        model.releases = 1;
                        model.rfault = false;
                        model.state = Abstract::Releasing;
                    } else {
                        model.state = Abstract::Idle;
                    }
                }
                (Op::ReleaseDone | Op::ReleaseFailed, Abstract::Releasing)
                    if model.releases > 0 =>
                {
                    model.releases -= 1;
                    model.rfault |= matches!(op, Op::ReleaseFailed);
                }
                (Op::FinishDone | Op::FinishFailed | Op::FinishCancelled, Abstract::Releasing)
                    if model.worker =>
                {
                    model.worker = false;
                    if model.attempt && matches!(op, Op::FinishDone) && driver {
                        model.releases += 1;
                    }
                    model.attempt = false;
                }
                (Op::FinishDone, Abstract::Idle | Abstract::Faulted) if model.worker => {
                    // The slot moved on without this attach; a driver port it made is undone.
                    model.worker = false;
                    if driver {
                        model.state = Abstract::Releasing;
                        model.releases = 1;
                        model.rfault = false;
                        model.attempt = false;
                    }
                }
                (Op::FinishFailed | Op::FinishCancelled, Abstract::Idle | Abstract::Faulted)
                    if model.worker =>
                {
                    model.worker = false;
                }
                _ => {}
            }
            if model.state == Abstract::Releasing && model.releases == 0 && !model.attempt {
                model.state = if model.rfault {
                    Abstract::Faulted
                } else {
                    Abstract::Idle
                };
                model.rfault = false;
            }
            model
        }

        fn preconditions(model: &Model, op: &Op) -> bool {
            match op {
                Op::Register | Op::FinishDone | Op::FinishFailed | Op::FinishCancelled => {
                    model.worker
                }
                Op::ReleaseDone | Op::ReleaseFailed => model.releases > 0,
                _ => true,
            }
        }
    }

    struct Real {
        slots: Slots,
        attempt: u64,
        attachment: Option<u64>,
        root: CancellationToken,
        mode: Mode,
    }

    fn abstract_of(slots: &Slots, key: &AttachKey) -> Abstract {
        match slots.state(key) {
            None => Abstract::Idle,
            Some(SlotState::Faulted(_)) => Abstract::Faulted,
            Some(SlotState::Attaching { .. }) => Abstract::Attaching,
            Some(SlotState::Attached(_)) => Abstract::Attached,
            Some(SlotState::Releasing { .. }) => Abstract::Releasing,
        }
    }

    struct Test;

    impl StateMachineTest for Test {
        type SystemUnderTest = Real;
        type Reference = Reference;

        fn init_test(_: &Model) -> Real {
            Real {
                slots: Slots::new(),
                attempt: 0,
                attachment: None,
                root: CancellationToken::new(),
                mode: Mode::Kernel,
            }
        }

        fn apply(mut real: Real, model: &Model, op: Op) -> Real {
            let key = AttachKey {
                epoch: 1,
                bus_id: "1-1.4".parse().unwrap(),
            };
            let hold = DriverHold {
                gateway_port: 3242,
                driver_port: 1,
            };
            match op {
                Op::Begin(mode) => {
                    if let Some((attempt, _)) = real.slots.begin_attach(&key, mode, &real.root) {
                        real.attempt = attempt;
                        real.mode = mode;
                        real.attachment = None;
                    }
                }
                Op::Register => {
                    if real.attachment.is_none() {
                        let id = real.slots.next_id();
                        let attachment = Attachment {
                            id,
                            port: None,
                            stop: CancellationToken::new(),
                            since: OffsetDateTime::UNIX_EPOCH,
                            kind: if real.mode == Mode::Driver {
                                Kind::Driver
                            } else {
                                Kind::Kernel
                            },
                        };
                        if real
                            .slots
                            .register(&key, Some(real.attempt), attachment)
                            .is_ok()
                        {
                            real.attachment = Some(id);
                        }
                    }
                }
                Op::FinishDone => {
                    let hold = (real.mode == Mode::Driver).then_some(hold);
                    let _ = real
                        .slots
                        .finish_attach(&key, real.attempt, Outcome::Done(hold));
                }
                Op::FinishFailed => {
                    let _ = real.slots.finish_attach(
                        &key,
                        real.attempt,
                        Outcome::Failed("failed".into()),
                    );
                }
                Op::FinishCancelled => {
                    let _ = real
                        .slots
                        .finish_attach(&key, real.attempt, Outcome::Cancelled);
                }
                Op::RelayEnds => {
                    if let Some(id) = real.attachment
                        && real.slots.relay_ended(&key, id).is_some()
                    {
                        real.attachment = None;
                    }
                }
                Op::Release => {
                    let _ = real.slots.begin_release(&key);
                }
                Op::ReleaseDone => real.slots.release_finished(&key, None, None),
                Op::ReleaseFailed => {
                    real.slots
                        .release_finished(&key, Some("could not".into()), None);
                }
            }
            let _ = model;
            real
        }

        fn check_invariants(real: &Real, model: &Model) {
            let key = AttachKey {
                epoch: 1,
                bus_id: "1-1.4".parse().unwrap(),
            };
            assert_eq!(abstract_of(&real.slots, &key), model.state);
            assert_eq!(real.slots.releases_running(), model.releases);
            // A device shown as attached has an attachment, and one that is not does not.
            let shown = real.slots.attachment(&key).is_some();
            match real.slots.state(&key) {
                Some(SlotState::Attached(_)) => assert!(shown),
                Some(SlotState::Releasing { .. } | SlotState::Faulted(_)) | None => {
                    assert!(!shown)
                }
                Some(SlotState::Attaching { .. }) => {}
            }
        }
    }

    prop_state_machine! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        #[test]
        fn the_machine_follows_its_model(sequential 1..40 => Test);
    }
}
