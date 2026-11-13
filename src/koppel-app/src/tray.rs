//! What the Windows service tells its tray icons: the tooltip and whether something needs the user,
//! and the notifications. Ported from 0.1.x's `TrayStatus`, `TrayFeed` and `TrayNoticeRules`. Kept
//! apart from Win32, so it is tested on every platform; `koppel-windows` writes it to the trays.
//!
//! Names and counts only: nothing here is a secret or a fingerprint, so the tray shows nothing a user
//! who is not signed in to the UI could not be shown.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use koppel_config::files::{KoppelSettings, Role};
use koppel_usb::{ClientDevices, DeviceState, LinkState, ServerDevices};
use tokio::sync::broadcast::{self, error::TryRecvError};

use crate::activity::{ActivityEvent, ActivityOutcome, ActivityReason};
use crate::app::App;
use crate::push::Pushed;
use crate::roles::{PairingRequestView, RequestState};

/// A notification icon's tooltip holds 127 characters.
pub const MAX_TIP: usize = 127;
/// A notification's title holds 63 characters and its text 255.
pub const MAX_TITLE: usize = 63;
pub const MAX_TEXT: usize = 255;

/// How often the feed looks again when nothing woke it, which catches the setup window closing.
const ROUND: Duration = Duration::from_secs(5);

/// What the tray icon shows: its tooltip, and whether something needs the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayState {
    pub tip: String,
    pub attention: bool,
}

impl TrayState {
    /// Before the service has said anything, and for a message the tray does not understand: the
    /// plain icon.
    pub fn plain() -> Self {
        Self {
            tip: "Koppel".to_owned(),
            attention: false,
        }
    }
}

/// A Windows notification the tray shows. `page` is the UI page a click opens, such as `#clients`,
/// or empty for the UI's start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayNotice {
    pub title: String,
    pub text: String,
    pub page: String,
}

/// What the tray's state is made from.
#[derive(Debug, Clone)]
pub struct TrayFacts {
    pub role: Role,
    pub password_set: bool,
    pub setup_open: bool,
    pub client: ClientDevices,
    /// The host of the server a client is linked to, as the UI shows it.
    pub server_host: Option<String>,
    pub server: ServerDevices,
}

/// The tooltip and the attention flag for `facts`.
pub fn describe(facts: &TrayFacts) -> TrayState {
    if !facts.password_set {
        let tip = if facts.setup_open {
            "Koppel: setting up. Open Koppel to set the password."
        } else {
            "Koppel: setup has timed out. Restart Koppel to set it up."
        };
        return TrayState {
            tip: tip.to_owned(),
            attention: true,
        };
    }
    let mut lines = Vec::new();
    if matches!(facts.role, Role::Server | Role::Both) {
        lines.push(server_line(&facts.server));
    }
    if matches!(facts.role, Role::Client | Role::Both) {
        lines.push(client_line(&facts.client, facts.server_host.as_deref()));
    }
    let tip = if lines.len() == 1 {
        format!("Koppel: {}", lines[0].0)
    } else {
        let mut tip = "Koppel".to_owned();
        for (text, _) in &lines {
            tip.push('\n');
            tip.push_str(&capitalise(text));
        }
        tip
    };
    TrayState {
        tip: cut(&tip, MAX_TIP),
        attention: lines.iter().any(|(_, attention)| *attention),
    }
}

fn server_line(server: &ServerDevices) -> (String, bool) {
    if server.unavailable.is_some() {
        return ("server, sharing is not available".to_owned(), true);
    }
    let shared = server
        .devices
        .iter()
        .filter(|d| {
            matches!(
                d.state,
                DeviceState::Shared | DeviceState::Attached | DeviceState::Offline
            )
        })
        .count();
    let in_use = server
        .devices
        .iter()
        .filter(|d| d.state == DeviceState::Attached)
        .count();
    (
        format!(
            "server, {} shared, {in_use} in use",
            count(shared, "device")
        ),
        false,
    )
}

fn client_line(client: &ClientDevices, server_host: Option<&str>) -> (String, bool) {
    let server = server_host.unwrap_or("the server");
    let attached = client.devices.iter().filter(|d| d.attached_here).count();
    match client.link {
        LinkState::Connected => (
            format!("linked to {server}, {} attached", count(attached, "device")),
            false,
        ),
        LinkState::Connecting => (format!("connecting to {server}"), false),
        LinkState::Unconfigured => ("no server yet".to_owned(), false),
        LinkState::Error => (format!("cannot reach {server}"), true),
    }
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `text` cut to `max` UTF-16 units, as Windows counts them, with `...` when it was longer.
pub fn cut(text: &str, max: usize) -> String {
    if text.encode_utf16().count() <= max {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut units = 0;
    for c in text.chars() {
        if units + c.len_utf16() > max - 3 {
            break;
        }
        units += c.len_utf16();
        out.push(c);
    }
    out.push_str("...");
    out
}

/// The host of a server's address, as the UI's sidebar names it: `lab-pc` for
/// `https://lab-pc:5240/`, `[fd00::1]` for an IPv6 address.
pub fn host_of(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let host = if authority.starts_with('[') {
        &authority[..=authority.find(']')?]
    } else {
        authority.split(':').next()?
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Decides which events become a notification, and remembers enough to say each thing once: a lost
/// link once until it is back, its return only after a loss was told, and each pairing request once.
/// A failed attach is already recorded once per reason, not on every retry. The settings turn each
/// kind off.
#[derive(Debug, Default)]
pub struct NoticeRules {
    seen_requests: HashSet<String>,
    link_lost: bool,
}

impl NoticeRules {
    pub fn from_activity(
        &mut self,
        event: &ActivityEvent,
        settings: &KoppelSettings,
    ) -> Option<TrayNotice> {
        let notice = |title: &str, text: String| TrayNotice {
            title: title.to_owned(),
            text,
            page: "#attach".to_owned(),
        };
        match (event.reason, event.outcome) {
            (ActivityReason::LinkDown, ActivityOutcome::Failed) => {
                self.link_lost = true;
                settings.notify_link.then(|| {
                    notice(
                        "Koppel lost the link to the server",
                        format!(
                            "{}. Koppel keeps trying, and attaches your devices again when it is back.",
                            event.summary
                        ),
                    )
                })
            }
            (ActivityReason::LinkUp, ActivityOutcome::Succeeded) if self.link_lost => {
                self.link_lost = false;
                settings.notify_link.then(|| {
                    notice(
                        "Koppel is linked again",
                        format!("{} again.", event.summary),
                    )
                })
            }
            (ActivityReason::Attached, ActivityOutcome::Failed) => {
                settings.notify_attach_failures.then(|| {
                    let text = match event.detail.as_deref() {
                        Some(reason) if !reason.is_empty() => {
                            format!("{}: {reason}", event.summary)
                        }
                        _ => event.summary.clone(),
                    };
                    notice("A device was not attached", text)
                })
            }
            _ => None,
        }
    }

    /// A notice for each request that is waiting and was not told before.
    pub fn from_pairing(
        &mut self,
        requests: &[PairingRequestView],
        settings: &KoppelSettings,
    ) -> Vec<TrayNotice> {
        let mut notices = Vec::new();
        for request in requests {
            if request.state == RequestState::Waiting
                && self.seen_requests.insert(request.id.clone())
                && settings.notify_pairing
            {
                notices.push(TrayNotice {
                    title: "A client wants to pair".to_owned(),
                    text: format!(
                        "{} is waiting. Open Clients and type the code it shows.",
                        request.client_name
                    ),
                    page: "#clients".to_owned(),
                });
            }
        }
        // Forget the ones that went, so the set stays as small as the list.
        self.seen_requests
            .retain(|id| requests.iter().any(|request| &request.id == id));
        notices
    }
}

/// What the tray needs of the sign-in: whether a password is set, whether the setup window is open,
/// and a signal when either changes. The web server's sign-in provides it.
pub trait SignInState: Send + Sync {
    fn has_password(&self) -> bool;
    fn setup_open(&self) -> bool;
    /// A signal when the password or the setup window changed.
    fn changes(&self) -> broadcast::Receiver<()>;
}

/// The feed one tray launcher reads: the state now, the notices since it last asked, and a wait that
/// ends when something the state is made from may have changed.
pub struct TrayFeed {
    app: Arc<App>,
    auth_state: Arc<dyn SignInState>,
    pushes: broadcast::Receiver<Arc<Pushed>>,
    /// The password or the setup window changed.
    auth: broadcast::Receiver<()>,
    activity: broadcast::Receiver<ActivityEvent>,
    rules: NoticeRules,
}

impl TrayFeed {
    /// Starts listening now. The pairing requests already waiting are not told; a request is told
    /// when it arrives.
    pub fn new(app: &Arc<App>, auth: Arc<dyn SignInState>) -> Self {
        let mut rules = NoticeRules::default();
        let quiet = KoppelSettings {
            notify_pairing: false,
            ..KoppelSettings::default()
        };
        rules.from_pairing(&app.server.pairing().requests, &quiet);
        Self {
            app: app.clone(),
            auth: auth.changes(),
            auth_state: auth,
            pushes: app.push.subscribe(),
            activity: app.activity.watch(),
            rules,
        }
    }

    /// What the trays show now.
    pub fn state(&self) -> TrayState {
        describe(&self.facts())
    }

    fn facts(&self) -> TrayFacts {
        let app = &self.app;
        TrayFacts {
            role: app.settings.current().role,
            password_set: self.auth_state.has_password(),
            setup_open: self.auth_state.setup_open(),
            client: app.usb_client.current(),
            server_host: app
                .connection
                .current()
                .server_url
                .as_deref()
                .and_then(host_of),
            server: app.usb_server.current(),
        }
    }

    /// The notices since the last call, oldest first.
    pub fn notices(&mut self) -> Vec<TrayNotice> {
        let settings = self.app.settings.current();
        let mut notices = Vec::new();
        loop {
            match self.activity.try_recv() {
                Ok(event) => notices.extend(self.rules.from_activity(&event, &settings)),
                // Too many at once: the ones dropped are not told.
                Err(TryRecvError::Lagged(_)) => {}
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            }
        }
        notices.extend(
            self.rules
                .from_pairing(&self.app.server.pairing().requests, &settings),
        );
        notices
    }

    /// Ends when the app pushed a change, a session ended (a password reset), or after a few seconds
    /// without either.
    pub async fn changed(&mut self) {
        tokio::select! {
            _ = self.pushes.recv() => {}
            _ = self.auth.recv() => {}
            () = tokio::time::sleep(ROUND) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{EntityKind, EntityRef};
    use koppel_usb::{ClientDeviceView, DeviceView};
    use time::OffsetDateTime;

    fn device(state: DeviceState) -> DeviceView {
        DeviceView {
            vid_pid: "046d:c52b".into(),
            name: "Receiver".into(),
            speed: "full".into(),
            state,
            ..DeviceView::new("1-1".parse().unwrap())
        }
    }

    fn client(link: LinkState, attached: usize) -> ClientDevices {
        ClientDevices {
            link,
            link_message: None,
            devices: (0..attached + 1)
                .map(|i| ClientDeviceView {
                    device: device(DeviceState::Shared),
                    attached_here: i < attached,
                    port: None,
                    attached_since: None,
                    fault: None,
                    attached_via: None,
                })
                .collect(),
            unavailable: None,
            gateway: false,
            gateway_fault: None,
        }
    }

    fn facts(role: Role) -> TrayFacts {
        TrayFacts {
            role,
            password_set: true,
            setup_open: false,
            client: client(LinkState::Connected, 2),
            server_host: Some("t520-client".into()),
            server: ServerDevices {
                devices: vec![
                    device(DeviceState::Shared),
                    device(DeviceState::Attached),
                    device(DeviceState::Offline),
                    device(DeviceState::Local),
                ],
                unavailable: None,
            },
        }
    }

    #[test]
    fn a_client_says_where_it_is_linked_and_what_is_attached() {
        let state = describe(&facts(Role::Client));
        assert_eq!(
            state.tip,
            "Koppel: linked to t520-client, 2 devices attached"
        );
        assert!(!state.attention);
    }

    #[test]
    fn a_server_counts_what_it_shares_and_what_is_in_use() {
        let state = describe(&facts(Role::Server));
        assert_eq!(state.tip, "Koppel: server, 3 devices shared, 1 in use");
        assert!(!state.attention);
    }

    #[test]
    fn both_roles_get_a_line_each() {
        let state = describe(&facts(Role::Both));
        assert_eq!(
            state.tip,
            "Koppel\nServer, 3 devices shared, 1 in use\nLinked to t520-client, 2 devices attached"
        );
    }

    #[test]
    fn setup_and_a_lost_link_need_attention() {
        let mut f = facts(Role::Client);
        f.password_set = false;
        f.setup_open = true;
        assert_eq!(
            describe(&f),
            TrayState {
                tip: "Koppel: setting up. Open Koppel to set the password.".into(),
                attention: true
            }
        );
        f.setup_open = false;
        assert!(describe(&f).tip.contains("timed out"));

        let mut f = facts(Role::Client);
        f.client = client(LinkState::Error, 0);
        f.server_host = None;
        assert_eq!(
            describe(&f),
            TrayState {
                tip: "Koppel: cannot reach the server".into(),
                attention: true
            }
        );
        f.client = client(LinkState::Unconfigured, 0);
        assert_eq!(describe(&f).tip, "Koppel: no server yet");
        assert!(!describe(&f).attention);

        let mut f = facts(Role::Server);
        f.server.unavailable = Some("no USB/IP".into());
        assert!(describe(&f).attention);
    }

    #[test]
    fn long_text_is_cut_in_utf16_units() {
        assert_eq!(cut("short", 10), "short");
        let long = "é".repeat(200);
        let cut = cut(&long, MAX_TIP);
        assert_eq!(cut.encode_utf16().count(), MAX_TIP);
        assert!(cut.ends_with("..."));
    }

    #[test]
    fn hosts_are_read_as_the_ui_shows_them() {
        assert_eq!(
            host_of("https://T520-Client:5240/").as_deref(),
            Some("t520-client")
        );
        assert_eq!(
            host_of("https://[fd00::1]:5240").as_deref(),
            Some("[fd00::1]")
        );
        assert_eq!(host_of("https://10.0.0.2").as_deref(), Some("10.0.0.2"));
        assert_eq!(host_of("not a url"), None);
    }

    fn event(
        reason: ActivityReason,
        outcome: ActivityOutcome,
        detail: Option<&str>,
    ) -> ActivityEvent {
        ActivityEvent {
            id: "1".into(),
            timestamp_utc: OffsetDateTime::UNIX_EPOCH,
            entity: EntityRef::new("link", "lab-pc", EntityKind::Link),
            reason,
            outcome,
            summary: "Linked to lab-pc".into(),
            detail: detail.map(str::to_owned),
        }
    }

    #[test]
    fn a_lost_link_is_told_once_and_its_return_only_after_a_loss() {
        let settings = KoppelSettings::default();
        let mut rules = NoticeRules::default();
        let up = event(ActivityReason::LinkUp, ActivityOutcome::Succeeded, None);
        let down = event(ActivityReason::LinkDown, ActivityOutcome::Failed, None);
        assert_eq!(rules.from_activity(&up, &settings), None);
        let lost = rules.from_activity(&down, &settings).unwrap();
        assert_eq!(lost.title, "Koppel lost the link to the server");
        assert_eq!(lost.page, "#attach");
        let back = rules.from_activity(&up, &settings).unwrap();
        assert_eq!(back.text, "Linked to lab-pc again.");
        assert_eq!(rules.from_activity(&up, &settings), None);
    }

    #[test]
    fn a_failed_attach_carries_its_reason_and_settings_turn_kinds_off() {
        let mut settings = KoppelSettings::default();
        let mut rules = NoticeRules::default();
        let failed = event(
            ActivityReason::Attached,
            ActivityOutcome::Failed,
            Some("the server is busy"),
        );
        assert_eq!(
            rules.from_activity(&failed, &settings).unwrap().text,
            "Linked to lab-pc: the server is busy"
        );
        settings.notify_attach_failures = false;
        settings.notify_link = false;
        assert_eq!(rules.from_activity(&failed, &settings), None);
        let down = event(ActivityReason::LinkDown, ActivityOutcome::Failed, None);
        assert_eq!(rules.from_activity(&down, &settings), None);
    }

    fn request(id: &str, state: RequestState) -> PairingRequestView {
        PairingRequestView {
            id: id.into(),
            client_name: "lab-pc".into(),
            from: "10.0.0.2".into(),
            requested_at: OffsetDateTime::UNIX_EPOCH,
            expires_at: OffsetDateTime::UNIX_EPOCH,
            state,
            message: None,
        }
    }

    #[test]
    fn each_waiting_request_is_told_once() {
        let settings = KoppelSettings::default();
        let mut rules = NoticeRules::default();
        let first = rules.from_pairing(&[request("a", RequestState::Waiting)], &settings);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].page, "#clients");
        assert_eq!(
            first[0].text,
            "lab-pc is waiting. Open Clients and type the code it shows."
        );
        assert!(
            rules
                .from_pairing(
                    &[
                        request("a", RequestState::Waiting),
                        request("b", RequestState::Denied)
                    ],
                    &settings
                )
                .is_empty()
        );
        assert!(rules.from_pairing(&[], &settings).is_empty());
        assert_eq!(
            rules
                .from_pairing(&[request("a", RequestState::Waiting)], &settings)
                .len(),
            1
        );
    }
}
