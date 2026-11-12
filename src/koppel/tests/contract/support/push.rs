//! The push contract: what the server pushes to the UI after it wrote a change.
//!
//! The program pushes these over Server-Sent Events at `/api/v1/events`, with the names and payloads
//! below. Each test in `push.rs` makes a REST change and checks that its event follows, carrying the
//! shape the REST read of the same thing has.
//!
//! A push test starts from [`super::Session::events`] with the session cookie, reads events until the
//! one it waits for or a timeout, and checks the payload with [`super::shape`]. Besides the events,
//! the contract has the stream answering 401 without a session, and closing when its session ends
//! (sign-out, a password change elsewhere, the cookie running out).
//!
//! The remote hub other Koppel machines use (Skakel's) is not the UI's push and is not part of this
//! list.

use super::shape::{self, Kind};

/// One event: its name, what makes the server send it, and the shape of its payload.
pub struct Event {
    pub name: &'static str,
    pub when: &'static str,
    pub payload: Payload,
}

/// The payload's shape.
pub enum Payload {
    /// An object with these fields.
    Object(&'static [(&'static str, Kind)]),
    /// A list of objects with these fields.
    List(&'static [(&'static str, Kind)]),
}

/// Every event the UI's push carries.
pub const EVENTS: &[Event] = &[
    Event {
        name: "SettingsChanged",
        when: "settings.json was written, by a PATCH /api/v1/settings or a valid hand edit",
        payload: Payload::Object(shape::SETTINGS),
    },
    Event {
        name: "ClientsChanged",
        when: "server: a client was added, paired, revoked or had its secret rotated",
        payload: Payload::List(shape::CLIENT),
    },
    Event {
        name: "ServerKeysChanged",
        when: "server: a certificate was prepared, cancelled or switched to",
        payload: Payload::Object(shape::KEYS),
    },
    Event {
        name: "ConnectionChanged",
        when: "client: connection.json was written, by a PUT, a pairing or a server that moved",
        payload: Payload::Object(shape::CONNECTION),
    },
    Event {
        name: "DevicesChanged",
        when: "server: a share changed, a device was plugged or unplugged, attached or released",
        payload: Payload::Object(shape::SERVER_DEVICES),
    },
    Event {
        name: "ClientDevicesChanged",
        when: "client: the link changed, or the server's devices or what is attached here did",
        payload: Payload::Object(shape::CLIENT_DEVICES),
    },
    Event {
        name: "ActivityRecorded",
        when: "anything went into the activity log",
        payload: Payload::Object(shape::ACTIVITY),
    },
    Event {
        name: "ServersDiscovered",
        when: "client: the list of servers found on the network changed",
        payload: Payload::List(shape::DISCOVERED_SERVER),
    },
    Event {
        name: "PairingStatusChanged",
        when: "client: pairing moved on (requesting, waiting with its code, confirm, paired, denied, expired, failed, idle)",
        payload: Payload::Object(shape::PAIRING_STATUS),
    },
    Event {
        name: "PairingRequestsChanged",
        when: "server: the pairing window opened or closed, a pairing request arrived, was approved, refused, withdrawn or expired, or the budget filled",
        payload: Payload::Object(shape::PAIRING_OVERVIEW),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_has_its_own_name() {
        let mut names: Vec<&str> = EVENTS.iter().map(|e| e.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), EVENTS.len());
    }
}
