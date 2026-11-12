//! Signing in to the UI: the one password in `ui-auth.json`, the first-run setup window, the session
//! cookie and how a session ends, as 0.1.x's `Login/*` does it. See "UI sign-in" in
//! docs/security.md.

pub(crate) mod endpoints;
pub(crate) mod password;
pub(crate) mod throttle;
pub(crate) mod ticket;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use koppel_config::files::UiCredentials;
use koppel_config::json::OrderedMap;
use koppel_config::{ConfigFile, ConfigFileError, KoppelPaths, SecretBox};
use ring::rand::{SecureRandom, SystemRandom};
use time::OffsetDateTime;
use tokio::sync::{Semaphore, broadcast};

use throttle::{Reservation, Throttle};
use ticket::{CookieKey, Ticket};

/// How long a session lasts without use. A request in its second half extends it.
pub(crate) const LIFETIME: Duration = Duration::from_secs(12 * 3600);
/// How long a session lasts at most, however much it is used. Counted from the sign-in, and a
/// password change does not restart it.
pub(crate) const ABSOLUTE_LIFETIME: Duration = Duration::from_secs(7 * 24 * 3600);
/// Password hashes that may run at once, for the whole server.
const HASH_SLOTS: usize = 2;
/// The most signed-out session ids kept.
const MAX_REVOKED: usize = 4096;

/// `bytes` random bytes.
pub(crate) fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("the system has random bytes");
    bytes
}

/// A new id or stamp, 32 hex digits as .NET's `Guid.NewGuid().ToString("N")`.
pub(crate) fn new_id() -> String {
    crate::activity::hex(&random_bytes::<16>())
}

/// Sessions that end, told to the live push so their streams close.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionEnd {
    /// One session signed out.
    Session(String),
    /// The stamp changed: every session ends but the one that changed the password, if any.
    AllBut(Option<String>),
}

impl SessionEnd {
    pub(crate) fn ends(&self, session: &str) -> bool {
        match self {
            Self::Session(ended) => ended == session,
            Self::AllBut(kept) => kept.as_deref() != Some(session),
        }
    }
}

/// The session a request carries, once checked.
#[derive(Debug, Clone)]
pub(crate) struct SignedIn {
    pub id: String,
    /// When it was first signed in, Unix seconds.
    pub issued: i64,
    /// How long it stays valid without another request: until the cookie runs out or the 7 day cap.
    pub valid_for: Duration,
    /// A cookie with a later expiry, when this request is in the second half of the session.
    pub renewed: Option<String>,
}

/// What first-run setup decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetupOutcome {
    Done,
    AlreadySet,
    Closed,
}

/// Owns `ui-auth.json`, the setup window, the cookie key and the limits on guessing.
pub(crate) struct UiAuth {
    file: ConfigFile<UiCredentials>,
    window: Duration,
    /// Where the log sends people to set the first password.
    address: String,
    key: CookieKey,
    setup_until: Mutex<Option<OffsetDateTime>>,
    seen_stamp: Mutex<String>,
    /// The session to keep for each new stamp a password change wrote, so two changes at once each
    /// keep their own.
    keep: Mutex<HashMap<String, String>>,
    setup: tokio::sync::Mutex<()>,
    ends: broadcast::Sender<SessionEnd>,
    slots: Arc<Semaphore>,
    throttle: Mutex<Throttle>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl UiAuth {
    /// Takes the started `ui-auth.json`, opens the setup window when there is no password, and
    /// follows the file for a reset or a hand edit.
    pub(crate) fn new(
        file: ConfigFile<UiCredentials>,
        window: Duration,
        address: String,
        key: CookieKey,
    ) -> Arc<Self> {
        let stamp = stamp_of(&file.current());
        let auth = Arc::new(Self {
            file: file.clone(),
            window,
            address,
            key,
            setup_until: Mutex::new(None),
            seen_stamp: Mutex::new(stamp),
            keep: Mutex::new(HashMap::new()),
            setup: tokio::sync::Mutex::new(()),
            ends: broadcast::channel(64).0,
            slots: Arc::new(Semaphore::new(HASH_SLOTS)),
            throttle: Mutex::new(Throttle::default()),
        });
        auth.sync_window(false);
        let weak: Weak<Self> = Arc::downgrade(&auth);
        file.subscribe(move |credentials| {
            let weak = weak.clone();
            async move {
                if let Some(auth) = weak.upgrade() {
                    auth.changed(&credentials);
                }
            }
        });
        auth
    }

    pub(crate) fn has_password(&self) -> bool {
        self.file.current().has_password()
    }

    fn stamp(&self) -> String {
        stamp_of(&self.file.current())
    }

    /// Until when the first password may be set, or `None` once there is one. It may lie in the
    /// past: setup has timed out.
    pub(crate) fn setup_until(&self) -> Option<OffsetDateTime> {
        let until = *lock(&self.setup_until);
        if self.has_password() { None } else { until }
    }

    pub(crate) fn setup_open(&self) -> bool {
        self.setup_until()
            .is_some_and(|until| OffsetDateTime::now_utc() < until)
    }

    /// What the UI is told when the first password comes too late.
    pub(crate) fn closed_reason(&self) -> String {
        format!(
            "Setup has timed out. Koppel takes the first password only in the first {} minutes after it starts. Restart Koppel, then set it.",
            self.window.as_secs() / 60
        )
    }

    pub(crate) fn ends(&self) -> broadcast::Receiver<SessionEnd> {
        self.ends.subscribe()
    }

    /// A hash slot, or `None` when both are busy. Nothing waits for one.
    pub(crate) fn hash_slot(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.slots.clone().try_acquire_owned().ok()
    }

    pub(crate) fn reserve(&self, key: &str) -> Reservation {
        lock(&self.throttle).reserve(key, Instant::now())
    }

    pub(crate) fn clear(&self, key: &str) {
        lock(&self.throttle).clear(key);
    }

    /// True when `password` is the one set. An empty or overlong one is not hashed at all.
    pub(crate) async fn verify(&self, password: Option<String>) -> bool {
        let Some(hash) = self.file.current().password_hash.clone() else {
            return false;
        };
        let Some(password) =
            password.filter(|p| !p.is_empty() && password::length(p) <= password::MAX_LENGTH)
        else {
            return false;
        };
        tokio::task::spawn_blocking(move || password::verify(&password, &hash))
            .await
            .unwrap_or(false)
    }

    /// Sets the first password while setup is open. Works once: later calls find a password.
    pub(crate) async fn set_first_password(
        &self,
        password: String,
    ) -> Result<SetupOutcome, ConfigFileError> {
        let _setup = self.setup.lock().await;
        if self.has_password() {
            return Ok(SetupOutcome::AlreadySet);
        }
        if !self.setup_open() {
            return Ok(SetupOutcome::Closed);
        }
        let hash = hash_off_thread(password).await;
        let stamp = new_id();
        self.file
            .update(|_| UiCredentials {
                password_hash: Some(hash),
                security_stamp: Some(stamp),
                revoked_sessions: None,
            })
            .await?;
        Ok(SetupOutcome::Done)
    }

    /// Replaces the password and the stamp. Every session but `keep` ends.
    pub(crate) async fn change_password(
        &self,
        next: String,
        keep: &str,
    ) -> Result<(), ConfigFileError> {
        let hash = hash_off_thread(next).await;
        let stamp = new_id();
        lock(&self.keep).insert(stamp.clone(), keep.to_owned());
        self.file
            .update(|current| UiCredentials {
                password_hash: Some(hash),
                security_stamp: Some(stamp),
                revoked_sessions: current.revoked_sessions.clone(),
            })
            .await?;
        Ok(())
    }

    /// Revokes a session that signed out, written to the file so a copy of its cookie is refused
    /// after a restart too. Each id is kept until its session could no longer have been valid.
    pub(crate) async fn revoke(&self, session: &str, issued: i64) -> Result<(), ConfigFileError> {
        let until = OffsetDateTime::from_unix_timestamp(issued)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH)
            + ABSOLUTE_LIFETIME;
        let now = OffsetDateTime::now_utc();
        self.file
            .update(|current| {
                let mut kept: Vec<(String, OffsetDateTime)> = current
                    .revoked_sessions
                    .as_ref()
                    .map(|map| map.0.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(id, ends)| *ends > now && id != session)
                    .collect();
                while kept.len() >= MAX_REVOKED {
                    let soonest = kept
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, (_, ends))| *ends)
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    kept.remove(soonest);
                }
                kept.push((session.to_owned(), until));
                UiCredentials {
                    revoked_sessions: Some(OrderedMap(kept)),
                    ..current.clone()
                }
            })
            .await?;
        let _ = self.ends.send(SessionEnd::Session(session.to_owned()));
        Ok(())
    }

    fn is_revoked(&self, session: &str) -> bool {
        let now = OffsetDateTime::now_utc();
        self.file
            .current()
            .revoked_sessions
            .as_ref()
            .and_then(|map| map.get(session).copied())
            .is_some_and(|until| until > now)
    }

    /// A cookie for a new session, or for `session` going on under the stamp now in force.
    pub(crate) fn cookie(&self, session: Option<&str>, issued: Option<i64>) -> String {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let ticket = Ticket {
            session: session.map_or_else(new_id, str::to_owned),
            stamp: self.stamp(),
            issued: issued.unwrap_or(now),
            expires: now + LIFETIME.as_secs() as i64,
        };
        ticket::set_cookie(&self.key.seal(&ticket))
    }

    /// The session behind a cookie value, if it is one and still good: signed by this key, under
    /// the stamp in force, with a password set, not signed out, not run out, and within 7 days of
    /// its sign-in.
    pub(crate) fn check(&self, value: &str) -> Result<SignedIn, Rejected> {
        let ticket = self.key.open(value).ok_or(Rejected::NotOurs)?;
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let cap = ticket.issued + ABSOLUTE_LIFETIME.as_secs() as i64;
        if ticket.stamp != self.stamp()
            || !self.has_password()
            || cap <= now
            || ticket.expires <= now
            || self.is_revoked(&ticket.session)
        {
            return Err(Rejected::Ended);
        }
        let renewed = (ticket.expires - now < LIFETIME.as_secs() as i64 / 2)
            .then(|| self.cookie(Some(&ticket.session), Some(ticket.issued)));
        let until = if renewed.is_some() {
            (now + LIFETIME.as_secs() as i64).min(cap)
        } else {
            ticket.expires.min(cap)
        };
        Ok(SignedIn {
            id: ticket.session,
            issued: ticket.issued,
            valid_for: Duration::from_secs((until - now).max(0) as u64),
            renewed,
        })
    }

    fn changed(&self, credentials: &UiCredentials) {
        let stamp = stamp_of(credentials);
        let fresh = {
            let mut seen = lock(&self.seen_stamp);
            let fresh = *seen != stamp;
            *seen = stamp.clone();
            fresh
        };
        // No password under a new stamp is a reset, which opens setup again even when it was open.
        self.sync_window(fresh);
        if fresh {
            let keep = lock(&self.keep).remove(&stamp);
            let _ = self.ends.send(SessionEnd::AllBut(keep));
        }
    }

    // A password closes the window. No password opens it when none was open yet (a start, or the
    // password edited out of the file), or again when asked (a reset).
    fn sync_window(&self, reopen: bool) {
        let opened = {
            let mut until = lock(&self.setup_until);
            if self.has_password() {
                *until = None;
                None
            } else if until.is_none() || reopen {
                let closes = OffsetDateTime::now_utc() + self.window;
                *until = Some(closes);
                Some(closes)
            } else {
                None
            }
        };
        if let Some(closes) = opened {
            tracing::warn!(
                "Koppel is waiting for its first password. Open {} and set it before {} (UTC). Whoever opens the UI first sets it. After that time, restart Koppel to set it.",
                self.address,
                koppel_config::json::format_date(closes.replace_nanosecond(0).unwrap_or(closes))
            );
        }
    }
}

/// Why a cookie was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rejected {
    /// Not a cookie this server signed: left alone, as ASP.NET left a cookie it could not read.
    NotOurs,
    /// A session that has ended: the answer clears the cookie.
    Ended,
}

fn stamp_of(credentials: &UiCredentials) -> String {
    credentials.security_stamp.clone().unwrap_or_default()
}

async fn hash_off_thread(password: String) -> String {
    tokio::task::spawn_blocking(move || password::hash(&password))
        .await
        .expect("hashing does not panic")
}

/// `koppel --reset-password`: clears the UI password and changes the stamp, which ends every
/// session. It edits the file on disk, so it works while Koppel runs (the running one sees the
/// change and opens setup again at once) and when it does not (the next start opens it). A file
/// that cannot be read is replaced: what an admin who forgot the password and lost the keys has
/// left.
pub async fn reset_password(
    paths: &KoppelPaths,
    secrets: &Arc<SecretBox>,
) -> Result<(), ConfigFileError> {
    let mut file = koppel_config::open::ui_auth(paths, secrets);
    if file.start().await.is_err() {
        file.stop();
        let _ = std::fs::remove_file(paths.ui_auth_file());
        file = koppel_config::open::ui_auth(paths, secrets);
        file.start().await?;
    }
    let stamp = new_id();
    file.update(|_| UiCredentials {
        password_hash: None,
        security_stamp: Some(stamp),
        revoked_sessions: None,
    })
    .await?;
    file.stop();
    Ok(())
}
