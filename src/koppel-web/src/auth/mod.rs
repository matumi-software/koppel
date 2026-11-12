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

use indexmap::IndexMap;
use koppel_config::files::UiCredentials;
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
    /// Whether first-run setup needs the token the log shows (Linux and Docker, when the UI is not local only).
    setup_token_required: bool,
    /// The token of the open window, made when it opens and gone once the password is set.
    setup_token: Mutex<Option<String>>,
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
        setup_token_required: bool,
        key: CookieKey,
    ) -> Arc<Self> {
        let stamp = stamp_of(&file.current());
        let auth = Arc::new(Self {
            file: file.clone(),
            window,
            address,
            setup_token_required,
            setup_token: Mutex::new(None),
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

    /// True when setup asks for the token the log shows.
    pub(crate) fn setup_token_required(&self) -> bool {
        self.setup_token_required
    }

    /// True when `given` is the open window's token. Always true when none is required.
    pub(crate) fn setup_token_is(&self, given: Option<&str>) -> bool {
        if !self.setup_token_required {
            return true;
        }
        let (Some(given), Some(token)) = (given, lock(&self.setup_token).clone()) else {
            return false;
        };
        same_bytes(given.trim().as_bytes(), token.as_bytes())
    }

    pub(crate) fn ends(&self) -> broadcast::Receiver<SessionEnd> {
        self.ends.subscribe()
    }

    /// A hash slot, or `None` when both are busy. Nothing waits for one.
    pub(crate) fn hash_slot(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.slots.clone().try_acquire_owned().ok()
    }

    /// How long until `key` may try a password, or `None`: the global budget and the caller's lock,
    /// read before a hash slot is taken so a locked caller cannot hold one.
    pub(crate) fn precheck(&self, key: &str) -> Option<Duration> {
        lock(&self.throttle).check(key, Instant::now())
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
                password_hash: Some(hash.into()),
                security_stamp: Some(stamp),
                revoked_sessions: IndexMap::new(),
            })
            .await?;
        *lock(&self.setup_token) = None;
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
                password_hash: Some(hash.into()),
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
                let mut kept: IndexMap<String, OffsetDateTime> = current
                    .revoked_sessions
                    .iter()
                    .filter(|(id, ends)| **ends > now && *id != session)
                    .map(|(id, ends)| (id.clone(), *ends))
                    .collect();
                while kept.len() >= MAX_REVOKED {
                    let soonest = kept
                        .iter()
                        .min_by_key(|(_, ends)| **ends)
                        .map(|(id, _)| id.clone());
                    match soonest {
                        Some(id) => kept.shift_remove(&id),
                        None => break,
                    };
                }
                kept.insert(session.to_owned(), until);
                UiCredentials {
                    revoked_sessions: kept,
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
            .get(session)
            .copied()
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
                *lock(&self.setup_token) = None;
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
            let closes_at = closes
                .replace_nanosecond(0)
                .unwrap_or(closes)
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default();
            if self.setup_token_required {
                let token = crate::activity::hex(&random_bytes::<16>());
                *lock(&self.setup_token) = Some(token.clone());
                tracing::warn!(
                    "Koppel is waiting for its first password. Open {} and set it before {} (UTC). The setup token is {token}: the page asks for it, and it works once. After that time, restart Koppel to set it.",
                    self.address,
                    closes_at
                );
            } else {
                tracing::warn!(
                    "Koppel is waiting for its first password. Open {} and set it before {} (UTC). Whoever opens the UI first sets it. After that time, restart Koppel to set it.",
                    self.address,
                    closes_at
                );
            }
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

/// Equal bytes, taking as long for a near miss as for a far one. The length is not secret.
fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
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
        revoked_sessions: IndexMap::new(),
    })
    .await?;
    file.stop();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::throttle_key;

    struct Folder(std::path::PathBuf);

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn auth(token_required: bool) -> (Folder, Arc<UiAuth>) {
        let root =
            std::env::temp_dir().join(format!("koppel-auth-{}-{}", std::process::id(), new_id()));
        let (config, keys) = (root.join("config"), root.join("keys"));
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&keys).unwrap();
        let folder = Folder(root);
        let paths = KoppelPaths::new(&config, &keys);
        let secrets = Arc::new(SecretBox::open(&keys).unwrap());
        let file = koppel_config::open::ui_auth(&paths, &secrets);
        file.start().await.unwrap();
        let key = CookieKey::load(&keys, &secrets).unwrap();
        let auth = UiAuth::new(
            file,
            Duration::from_secs(600),
            "https://bench:5240".into(),
            token_required,
            key,
        );
        (folder, auth)
    }

    fn window_token(auth: &UiAuth) -> Option<String> {
        lock(&auth.setup_token).clone()
    }

    #[tokio::test]
    async fn a_required_setup_token_is_made_with_the_window_and_works_once() {
        let (_folder, auth) = auth(true).await;
        let token = window_token(&auth).expect("the open window made a token");
        assert_eq!(token.len(), 32);
        assert!(auth.setup_token_required());
        assert!(!auth.setup_token_is(None));
        assert!(!auth.setup_token_is(Some("")));
        assert!(!auth.setup_token_is(Some(&"0".repeat(32))));
        assert!(!auth.setup_token_is(Some(&token[..31])));
        assert!(auth.setup_token_is(Some(&token)));
        assert!(
            auth.setup_token_is(Some(&format!(" {token}\n"))),
            "as pasted"
        );

        let done = auth
            .set_first_password("a long enough password".into())
            .await;
        assert_eq!(done.unwrap(), SetupOutcome::Done);
        assert!(window_token(&auth).is_none(), "the token is spent");
        assert!(!auth.setup_token_is(Some(&token)));
    }

    #[tokio::test]
    async fn a_reset_makes_a_new_token_and_without_the_requirement_there_is_none() {
        let (_folder, auth) = auth(true).await;
        let first = window_token(&auth).unwrap();
        auth.sync_window(true);
        let second = window_token(&auth).unwrap();
        assert_ne!(first, second, "a reopened window has a token of its own");
        assert!(!auth.setup_token_is(Some(&first)));

        let (_folder, open) = super::tests::auth(false).await;
        assert!(!open.setup_token_required());
        assert!(window_token(&open).is_none());
        assert!(open.setup_token_is(None), "nothing is asked for");
    }

    #[tokio::test]
    async fn a_shared_64_shares_the_lock_and_a_flood_of_addresses_meets_the_budget() {
        let (_folder, auth) = auth(false).await;
        let one = format!("login:{}", throttle_key("2001:db8:1:2::1".parse().unwrap()));
        let other = format!(
            "login:{}",
            throttle_key("2001:db8:1:2:dead:beef:0:7".parse().unwrap())
        );
        assert_eq!(one, other);
        for _ in 0..throttle::FREE_FAILURES {
            assert!(auth.reserve(&one).granted);
        }
        assert!(
            auth.precheck(&other).is_some(),
            "the neighbour is locked too"
        );
        let elsewhere = format!("login:{}", throttle_key("2001:db8:1:3::1".parse().unwrap()));
        assert!(auth.precheck(&elsewhere).is_none());

        // Each from a /64 of its own, so no caller is locked: only the budget stops them.
        for i in 0..throttle::GLOBAL_BUDGET {
            auth.reserve(&format!("login:2001:db8:{i:x}::/64"));
        }
        assert!(auth.precheck("login:2001:db8:ffff::/64").is_some());
        assert!(!auth.reserve("login:2001:db8:ffff::/64").granted);
    }
}
