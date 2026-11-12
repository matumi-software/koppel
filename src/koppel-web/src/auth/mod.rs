//! Signing in to the UI: the one password in `ui-auth.json`, the first-run setup window, the sessions
//! (kept in memory, see [`session`]) and how a session ends. See "UI sign-in" in docs/security.md.

pub(crate) mod endpoints;
pub(crate) mod password;
pub(crate) mod session;
pub(crate) mod throttle;

use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use koppel_config::files::UiCredentials;
use koppel_config::{ConfigFile, ConfigFileError, KoppelPaths, Secret, SecretBox};
use ring::rand::{SecureRandom, SystemRandom};
use time::OffsetDateTime;
use tokio::sync::{Semaphore, broadcast};

use session::Sessions;
pub(crate) use session::SignedIn;
use throttle::{Reservation, Throttle};

use crate::sync::Unpoisoned;

/// Password hashes that may run at once, for the whole server.
const HASH_SLOTS: usize = 2;

/// `bytes` random bytes.
pub(crate) fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("the system has random bytes");
    bytes
}

/// Why a password could not be set.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SetPasswordError {
    #[error(transparent)]
    Hash(#[from] HashError),
    #[error(transparent)]
    Save(#[from] ConfigFileError),
}

/// What first-run setup decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetupOutcome {
    Done,
    AlreadySet,
    Closed,
}

/// Owns `ui-auth.json`, the setup window, the sessions and the limits on guessing.
pub(crate) struct UiAuth {
    file: ConfigFile<UiCredentials>,
    window: Duration,
    /// Where the log sends people to set the first password.
    address: String,
    /// Whether first-run setup needs the token the log shows (Linux and Docker, when the UI is not local only).
    setup_token_required: bool,
    /// The token of the open window, made when it opens and gone once the password is set.
    setup_token: Mutex<Option<String>>,
    setup_until: Mutex<Option<OffsetDateTime>>,
    sessions: Sessions,
    /// The password hash this server last wrote or saw. A file that differs from it was changed from
    /// outside (a reset, a hand edit), which ends every session; a change made here sets it first.
    seen_hash: Mutex<Option<Secret<String>>>,
    setup: tokio::sync::Mutex<()>,
    /// Told when the file changed, for whoever shows the password and setup state (the tray).
    changes: broadcast::Sender<()>,
    slots: Arc<Semaphore>,
    throttle: Mutex<Throttle>,
}

impl UiAuth {
    /// Takes the started `ui-auth.json`, opens the setup window when there is no password, and
    /// follows the file for a reset or a hand edit.
    pub(crate) fn new(
        file: ConfigFile<UiCredentials>,
        window: Duration,
        address: String,
        setup_token_required: bool,
    ) -> Arc<Self> {
        let seen = file.current().password_hash.clone();
        let auth = Arc::new(Self {
            file: file.clone(),
            window,
            address,
            setup_token_required,
            setup_token: Mutex::new(None),
            setup_until: Mutex::new(None),
            sessions: Sessions::default(),
            seen_hash: Mutex::new(seen),
            setup: tokio::sync::Mutex::new(()),
            changes: broadcast::channel(8).0,
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

    /// Until when the first password may be set, or `None` once there is one. It may lie in the
    /// past: setup has timed out.
    pub(crate) fn setup_until(&self) -> Option<OffsetDateTime> {
        let until = *self.setup_until.locked();
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
        let (Some(given), Some(token)) = (given, self.setup_token.locked().clone()) else {
            return false;
        };
        same_bytes(given.trim().as_bytes(), token.as_bytes())
    }

    /// Wakes when the password was set, changed or cleared, or the setup window opened or closed.
    pub(crate) fn changes(&self) -> broadcast::Receiver<()> {
        self.changes.subscribe()
    }

    /// A hash slot, or `None` when both are busy. Nothing waits for one.
    pub(crate) fn hash_slot(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.slots.clone().try_acquire_owned().ok()
    }

    /// How long until `key` may try a password, or `None`: the global budget and the caller's lock,
    /// read before a hash slot is taken so a locked caller cannot hold one.
    pub(crate) fn precheck(&self, key: &str) -> Option<Duration> {
        self.throttle.locked().check(key, Instant::now())
    }

    pub(crate) fn reserve(&self, key: &str) -> Reservation {
        self.throttle.locked().reserve(key, Instant::now())
    }

    pub(crate) fn clear(&self, key: &str) {
        self.throttle.locked().clear(key);
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
    ) -> Result<SetupOutcome, SetPasswordError> {
        let _setup = self.setup.lock().await;
        if self.has_password() {
            return Ok(SetupOutcome::AlreadySet);
        }
        if !self.setup_open() {
            return Ok(SetupOutcome::Closed);
        }
        let hash: Secret<String> = hash_off_thread(password).await?.into();
        // Set before the write, so the file's own change is not taken for one from outside.
        *self.seen_hash.locked() = Some(hash.clone());
        self.file
            .update(|_| UiCredentials {
                password_hash: Some(hash),
            })
            .await?;
        *self.setup_token.locked() = None;
        Ok(SetupOutcome::Done)
    }

    /// A new session, and the `Set-Cookie` value that names it.
    pub(crate) fn sign_in(&self) -> String {
        session::set_cookie(&self.sessions.start())
    }

    /// The session behind a cookie value, if it is one and still good: one this server started, not
    /// ended, not run out, within 7 days of its sign-in, and with a password set.
    pub(crate) fn check(&self, value: &str) -> Option<SignedIn> {
        if !self.has_password() {
            return None;
        }
        self.sessions.check(value)
    }

    /// Signs one session out. A copy of its cookie is refused from now on, and its live push ends.
    pub(crate) fn sign_out(&self, session: &str) {
        self.sessions.end(session);
    }

    /// Replaces the password. Every session but `keep` ends, and so do their live pushes.
    pub(crate) async fn change_password(
        &self,
        next: String,
        keep: &str,
    ) -> Result<(), SetPasswordError> {
        let hash: Secret<String> = hash_off_thread(next).await?.into();
        *self.seen_hash.locked() = Some(hash.clone());
        self.file
            .update(|_| UiCredentials {
                password_hash: Some(hash),
            })
            .await?;
        self.sessions.end_all_but(Some(keep));
        Ok(())
    }

    // The file changed. A password that is not the one this server wrote is a change from outside, a
    // reset or a hand edit: every session ends, and no password opens setup again even when it was
    // open.
    fn changed(&self, credentials: &UiCredentials) {
        let outside = {
            let mut seen = self.seen_hash.locked();
            let outside = *seen != credentials.password_hash;
            seen.clone_from(&credentials.password_hash);
            outside
        };
        if outside {
            self.sessions.end_all_but(None);
        }
        self.sync_window(outside);
        let _ = self.changes.send(());
    }

    // A password closes the window. No password opens it when none was open yet (a start, or the
    // password edited out of the file), or again when asked (a reset).
    fn sync_window(&self, reopen: bool) {
        let opened = {
            let mut until = self.setup_until.locked();
            if self.has_password() {
                *until = None;
                *self.setup_token.locked() = None;
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
                *self.setup_token.locked() = Some(token.clone());
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

/// 0.2.0 signed its session cookies with a key in `keys/ui-session.key`. Sessions are in memory now,
/// so the file is of no use: it is deleted at the first start of this version. (`ui-auth.json` stays,
/// it holds the password; the stamp and the list of signed-out sessions an older version wrote to it
/// are ignored, and gone the next time the password changes.)
pub(crate) fn remove_session_key(keys: &std::path::Path) {
    let path = keys.join("ui-session.key");
    match std::fs::remove_file(&path) {
        Ok(()) => tracing::info!(
            "Deleted {}: UI sessions are kept in memory now, so everyone signs in once",
            path.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("Could not delete {}: {e}", path.display()),
    }
}

/// Equal bytes, taking as long for a near miss as for a far one. The length is not secret.
fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// Why a password could not be hashed: the blocking task did not finish, as when the runtime is
/// shutting down.
#[derive(Debug, thiserror::Error)]
#[error("the password could not be hashed: {0}")]
pub(crate) struct HashError(#[source] tokio::task::JoinError);

async fn hash_off_thread(password: String) -> Result<String, HashError> {
    tokio::task::spawn_blocking(move || password::hash(&password))
        .await
        .map_err(HashError)
}

/// `koppel reset-password`: clears the UI password, which ends every session of a running Koppel (a
/// restart ends them anyway). It edits the file on disk, so it works while Koppel runs (the running one sees the
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
    file.update(|_| UiCredentials {
        password_hash: None,
    })
    .await?;
    file.stop();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::throttle_key;
    use tokio_util::sync::CancellationToken;

    struct Folder(std::path::PathBuf);

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn auth(token_required: bool) -> (Folder, Arc<UiAuth>) {
        let root = std::env::temp_dir().join(format!(
            "koppel-auth-{}-{}",
            std::process::id(),
            crate::activity::hex(&random_bytes::<8>())
        ));
        let (config, keys) = (root.join("config"), root.join("keys"));
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&keys).unwrap();
        let folder = Folder(root);
        let paths = KoppelPaths::new(&config, &keys);
        let secrets = Arc::new(SecretBox::open(&keys).unwrap());
        let file = koppel_config::open::ui_auth(&paths, &secrets);
        file.start().await.unwrap();
        let auth = UiAuth::new(
            file,
            Duration::from_secs(600),
            "https://bench:5240".into(),
            token_required,
        );
        (folder, auth)
    }

    fn window_token(auth: &UiAuth) -> Option<String> {
        auth.setup_token.locked().clone()
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

    const PASSWORD: &str = "a long enough password";

    /// A password set and `n` sessions signed in: the cookie values.
    async fn signed_in(auth: &UiAuth, n: usize) -> Vec<String> {
        assert_eq!(
            auth.set_first_password(PASSWORD.into()).await.unwrap(),
            SetupOutcome::Done
        );
        (0..n).map(|_| id_of(&auth.sign_in())).collect()
    }

    fn id_of(set_cookie: &str) -> String {
        let (name, rest) = set_cookie.split_once('=').unwrap();
        assert_eq!(name, session::COOKIE_NAME);
        rest.split(';').next().unwrap().to_owned()
    }

    #[tokio::test]
    async fn setup_signs_in_and_the_first_password_does_not_end_its_own_session() {
        let (_folder, auth) = auth(false).await;
        assert!(
            auth.check(&"A".repeat(43)).is_none(),
            "no password, no session"
        );
        let ids = signed_in(&auth, 1).await;
        assert!(auth.check(&ids[0]).is_some());
    }

    #[tokio::test]
    async fn a_password_change_ends_every_session_but_the_one_that_changed_it() {
        let (_folder, auth) = auth(false).await;
        let ids = signed_in(&auth, 3).await;
        let tokens: Vec<_> = ids.iter().map(|id| auth.check(id).unwrap().ended).collect();

        auth.change_password("another long password".into(), &ids[1])
            .await
            .unwrap();

        // The write came back to this server as a change it made itself: it ends nothing more.
        assert!(auth.check(&ids[1]).is_some(), "the changing session stays");
        assert!(auth.check(&ids[0]).is_none() && auth.check(&ids[2]).is_none());
        assert_eq!(
            tokens.iter().map(|t| t.is_cancelled()).collect::<Vec<_>>(),
            [true, false, true]
        );
        assert!(auth.verify(Some("another long password".into())).await);
        assert!(!auth.verify(Some(PASSWORD.into())).await);
    }

    #[tokio::test]
    async fn signing_out_ends_one_session_and_its_push_and_a_copy_of_the_cookie() {
        let (_folder, auth) = auth(false).await;
        let ids = signed_in(&auth, 2).await;
        let token = auth.check(&ids[0]).unwrap().ended;
        auth.sign_out(&ids[0]);
        assert!(token.is_cancelled());
        assert!(auth.check(&ids[0]).is_none());
        assert!(auth.check(&ids[1]).is_some());
    }

    #[tokio::test]
    async fn a_reset_from_outside_ends_every_session_and_opens_setup() {
        let (_folder, auth) = auth(false).await;
        let ids = signed_in(&auth, 2).await;
        let tokens: Vec<_> = ids.iter().map(|id| auth.check(id).unwrap().ended).collect();
        assert!(!auth.setup_open(), "a password is set");
        let mut changes = auth.changes();

        // As `koppel reset-password` writes it, while this server runs.
        auth.file
            .update(|_| UiCredentials {
                password_hash: None,
            })
            .await
            .unwrap();

        assert!(tokens.iter().all(CancellationToken::is_cancelled));
        assert!(ids.iter().all(|id| auth.check(id).is_none()));
        assert!(auth.setup_open(), "setup is open again");
        assert!(changes.try_recv().is_ok(), "the tray is told");
    }
}
