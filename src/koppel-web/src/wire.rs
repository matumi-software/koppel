//! The TypeScript types of the HTTP wire, generated from the Rust types the API sends and reads.
//!
//! The UI imports them from `ui/app/src/api/generated/wire.ts`. That file is checked in, and the
//! `generated_types_are_current` test fails when it differs from what the Rust types produce, so a
//! payload change cannot reach the UI unseen. There is no CI: this runs with `cargo test`.
//! After changing a type, regenerate with
//!
//! ```sh
//! KOPPEL_WRITE_WIRE=1 cargo test -p koppel-web wire
//! ```
//!
//! Dates are RFC 3339 strings. `time`'s serde module is not something ts-rs reads, so those fields
//! carry `#[ts(type = "string")]`; the fixtures test below pins what the strings look like.

use std::path::PathBuf;

use ts_rs::{Config, TS};

use crate::activity::ActivityEvent;
use crate::api::client::{ConnectionUpdate, ConnectionView, PairStart};
use crate::api::server::{Approve, CreateClient, OpenPairing, ServerIdentity};
use crate::api::settings::{SettingsPatch, SystemInfo};
use crate::auth::endpoints::{AuthState, ChangeRequest, PasswordRequest};
use crate::roles::{
    ClientView, ConnectionTest, DiscoveredServer, KeysView, NewClient, Outcome, PairingOverview,
    PairingRequestView, PairingState, PairingStatus, PairingWindowView, RequestState,
};
use koppel_config::files::{KoppelSettings, Role};
use koppel_usb::{ActivityOutcome, ActivityReason, EntityKind, EntityRef, SystemCheck};
use koppel_usb::{
    ClientDeviceView, ClientDevices, DeviceState, DeviceView, LinkState, ServerDevices,
};

/// Where the checked-in file lives, from the crate's folder.
fn target() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui/app/src/api/generated/wire.ts")
}

fn decls() -> String {
    let cfg = Config::default();
    let mut out = String::from(
        "// Generated from the Rust types by `KOPPEL_WRITE_WIRE=1 cargo test -p koppel-web wire`.\n\
         // Do not edit: the test `generated_types_are_current` fails when this differs.\n\n",
    );
    macro_rules! decls {
        ($($t:ty),+ $(,)?) => {$(
            out.push_str("export ");
            out.push_str(&<$t as TS>::decl(&cfg));
            out.push_str("\n\n");
        )+};
    }
    decls!(
        // Answers and pushes.
        ActivityEvent,
        ActivityOutcome,
        ActivityReason,
        AuthState,
        ClientDeviceView,
        ClientDevices,
        ClientView,
        ConnectionTest,
        ConnectionView,
        DeviceState,
        DeviceView,
        DiscoveredServer,
        EntityKind,
        EntityRef,
        KeysView,
        KoppelSettings,
        LinkState,
        NewClient,
        Outcome,
        PairingOverview,
        PairingRequestView,
        PairingState,
        PairingStatus,
        PairingWindowView,
        RequestState,
        Role,
        ServerDevices,
        ServerIdentity<'static>,
        SystemCheck,
        SystemInfo<'static>,
        // Request bodies.
        Approve,
        ChangeRequest,
        ConnectionUpdate,
        CreateClient,
        OpenPairing,
        PairStart,
        PasswordRequest,
        SettingsPatch,
    );
    out.truncate(out.trim_end().len());
    out.push('\n');
    out
}

#[test]
fn generated_types_are_current() {
    let want = decls();
    let path = target();
    if std::env::var_os("KOPPEL_WRITE_WIRE").is_some() {
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("makes the folder");
        std::fs::write(&path, &want).expect("writes the file");
        return;
    }
    let have = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        have == want,
        "{} is out of date with the Rust types. Regenerate it with \
         `KOPPEL_WRITE_WIRE=1 cargo test -p koppel-web wire` and commit it.",
        path.display()
    );
}

/// The JSON fixtures the UI's mocks use too (`ui/app/src/test/wire/`). Each one is read into the
/// Rust type and written back: it must come out as the same JSON, so a field renamed, added or
/// dropped on either side fails here, and in the UI, where the mocks are checked against the
/// generated types.
mod fixtures {
    use serde::Serialize;
    use serde::de::Deserialize;
    use serde_json::Value;

    use super::*;
    use crate::api::server::ServerIdentity;
    use crate::api::settings::SystemInfo;

    fn fixture(name: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../ui/app/src/test/wire")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn round_trips<'a, T: Serialize + Deserialize<'a>>(text: &'a str, name: &str) {
        let want: Value = serde_json::from_str(text).expect("the fixture is JSON");
        let value: T = serde_json::from_str(text)
            .unwrap_or_else(|e| panic!("{name} does not read as the Rust type: {e}"));
        let got = serde_json::to_value(&value).expect("writes");
        assert_eq!(got, want, "{name} is not what the Rust type sends");
    }

    macro_rules! fixtures {
        ($($file:literal => $t:ty),+ $(,)?) => {
            #[test]
            fn the_fixtures_are_what_rust_sends() {
                $({
                    let text = fixture($file);
                    round_trips::<$t>(&text, $file);
                })+
            }
        };
    }

    fixtures!(
        "activity-event.json" => ActivityEvent,
        "auth-state.json" => AuthState,
        "client-devices.json" => ClientDevices,
        "clients.json" => Vec<ClientView>,
        "connection-test.json" => ConnectionTest,
        "connection-view.json" => ConnectionView,
        "discovered-servers.json" => Vec<DiscoveredServer>,
        "keys-view.json" => KeysView,
        "new-client.json" => NewClient,
        "pairing-overview.json" => PairingOverview,
        "pairing-status.json" => PairingStatus,
        "server-devices.json" => ServerDevices,
        "server-identity.json" => ServerIdentity<'_>,
        "settings.json" => KoppelSettings,
        "system-checks.json" => Vec<SystemCheck>,
        "system-info.json" => SystemInfo<'_>,
    );
}
