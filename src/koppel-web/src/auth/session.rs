//! The sessions of the UI, kept in memory, and the cookie that names one.
//!
//! A session is a random 256-bit id, handed to the browser in a `__Host-` cookie and looked up here
//! on every request. Nothing about it is written to disk, so a restart signs everyone out (the
//! price of having no key file, no stamp and no list of signed-out sessions). A session lasts 12 hours
//! without use, and a request in the second half of that extends it; it never lasts more than 7 days
//! from the sign-in. Each session owns a [`CancellationToken`] that ends its live push when the
//! session ends, however it ends: signing out, a password change or reset, running out, or being the
//! oldest when the table is full.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use axum::http::HeaderMap;
use axum::http::header::COOKIE;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cookie::{Cookie, SameSite};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use koppel_app::sync::Unpoisoned;

/// The cookie's name. The `__Host-` prefix makes a browser take it only from a secure origin, with
/// `Path=/` and no `Domain`, so a sibling host cannot set it. It is not the name 0.1.x used
/// (`koppel.ui`), so everyone signs in once at the upgrade.
pub(crate) const COOKIE_NAME: &str = "__Host-koppel-session";

/// How long a session lasts without use. A request in its second half extends it.
pub(crate) const LIFETIME: Duration = Duration::from_secs(12 * 3600);
/// How long a session lasts at most, however much it is used, counted from the sign-in.
pub(crate) const ABSOLUTE_LIFETIME: Duration = Duration::from_secs(7 * 24 * 3600);
/// The most sessions kept. Signing in takes the password, so this is never reached by anyone who
/// has not got it; when it is, the session that would end first goes.
const MAX_SESSIONS: usize = 256;
/// How often the sessions that ran out are dropped.
const SWEEP: Duration = Duration::from_secs(60);
/// A session id: 32 random bytes as URL-safe base64 without padding.
const ID_LENGTH: usize = 43;

/// The session a request carries, once checked.
#[derive(Debug, Clone)]
pub(crate) struct SignedIn {
    pub id: String,
    /// How long it stays valid without another request: until it runs out or the 7 day cap.
    pub valid_for: Duration,
    /// Cancelled when the session ends. The live push ends with it.
    pub ended: CancellationToken,
}

struct Session {
    signed_in: Instant,
    /// When it runs out unless a request extends it.
    expires: Instant,
    ended: CancellationToken,
}

impl Session {
    fn last_moment(&self) -> Instant {
        self.expires.min(self.signed_in + ABSOLUTE_LIFETIME)
    }

    fn is_over(&self, now: Instant) -> bool {
        now >= self.last_moment()
    }
}

#[derive(Default)]
struct Table {
    sessions: HashMap<String, Session>,
    swept: Option<Instant>,
}

impl Table {
    fn sweep(&mut self, now: Instant) {
        if self.swept.is_some_and(|at| now.duration_since(at) < SWEEP) {
            return;
        }
        self.swept = Some(now);
        self.sessions.retain(|_, session| {
            let over = session.is_over(now);
            if over {
                session.ended.cancel();
            }
            !over
        });
    }
}

/// The signed-in sessions.
#[derive(Default)]
pub(crate) struct Sessions {
    table: Mutex<Table>,
}

impl Sessions {
    fn table(&self) -> std::sync::MutexGuard<'_, Table> {
        self.table.locked()
    }

    /// A new session, and its id for the cookie.
    pub(crate) fn start(&self) -> String {
        self.start_at(Instant::now())
    }

    fn start_at(&self, now: Instant) -> String {
        let id = URL_SAFE_NO_PAD.encode(super::random_bytes::<32>());
        let mut table = self.table();
        table.sweep(now);
        if table.sessions.len() >= MAX_SESSIONS
            && let Some(first) = table
                .sessions
                .iter()
                .min_by_key(|(_, session)| session.last_moment())
                .map(|(id, _)| id.clone())
            && let Some(session) = table.sessions.remove(&first)
        {
            session.ended.cancel();
        }
        table.sessions.insert(
            id.clone(),
            Session {
                signed_in: now,
                expires: now + LIFETIME,
                ended: CancellationToken::new(),
            },
        );
        id
    }

    /// The session behind a cookie value, if it is one and still good. A request in the second half
    /// of the session's life extends it, up to the cap.
    pub(crate) fn check(&self, id: &str) -> Option<SignedIn> {
        self.check_at(id, Instant::now())
    }

    fn check_at(&self, id: &str, now: Instant) -> Option<SignedIn> {
        if id.len() != ID_LENGTH {
            return None;
        }
        let mut table = self.table();
        table.sweep(now);
        let session = table.sessions.get_mut(id)?;
        if session.is_over(now) {
            if let Some(session) = table.sessions.remove(id) {
                session.ended.cancel();
            }
            return None;
        }
        if session.expires.duration_since(now) < LIFETIME / 2 {
            session.expires = now + LIFETIME;
        }
        Some(SignedIn {
            id: id.to_owned(),
            valid_for: session.last_moment().duration_since(now),
            ended: session.ended.clone(),
        })
    }

    /// Ends one session. False when there was none.
    pub(crate) fn end(&self, id: &str) -> bool {
        match self.table().sessions.remove(id) {
            Some(session) => {
                session.ended.cancel();
                true
            }
            None => false,
        }
    }

    /// Ends every session but `keep`.
    pub(crate) fn end_all_but(&self, keep: Option<&str>) {
        self.table().sessions.retain(|id, session| {
            let kept = keep == Some(id.as_str());
            if !kept {
                session.ended.cancel();
            }
            kept
        });
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.table().sessions.len()
    }
}

/// The session cookie's value in the request, if it has one.
pub(crate) fn from_request(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(Cookie::split_parse)
        .filter_map(Result::ok)
        .find(|cookie| cookie.name() == COOKIE_NAME)
        .map(|cookie| cookie.value().to_owned())
        .filter(|value| !value.is_empty())
}

/// `Set-Cookie` for a session: HttpOnly, SameSite=Strict, Secure, `Path=/`, and a session cookie,
/// since how long the session lasts is the server's to say.
pub(crate) fn set_cookie(id: &str) -> String {
    Cookie::build((COOKIE_NAME, id.to_owned()))
        .path("/")
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Strict)
        .build()
        .to_string()
}

/// `Set-Cookie` that removes the session cookie.
pub(crate) fn clear_cookie() -> String {
    let mut removal = Cookie::build((COOKIE_NAME, ""))
        .path("/")
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Strict)
        .build();
    removal.make_removal();
    removal.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(cookie: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.append(COOKIE, cookie.parse().unwrap());
        headers
    }

    #[tokio::test(start_paused = true)]
    async fn an_id_is_256_random_bits_and_opens_its_session_only() {
        let sessions = Sessions::default();
        let one = sessions.start();
        let two = sessions.start();
        assert_eq!(one.len(), ID_LENGTH);
        assert_eq!(URL_SAFE_NO_PAD.decode(&one).unwrap().len(), 32);
        assert_ne!(one, two);
        assert_eq!(sessions.check(&one).unwrap().id, one);
        assert!(sessions.check(&two).is_some());
        for wrong in ["", "x", &"A".repeat(ID_LENGTH), &format!("{one}A")] {
            assert!(sessions.check(wrong).is_none(), "{wrong:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_runs_out_after_12_hours_without_use_and_use_in_the_second_half_extends_it() {
        let sessions = Sessions::default();
        let id = sessions.start();

        // Used in the first half: no extension, so it still ends 12 hours after the sign-in.
        tokio::time::advance(LIFETIME / 4).await;
        let early = sessions.check(&id).unwrap();
        assert_eq!(early.valid_for, LIFETIME * 3 / 4);

        // Used in the second half: it now lasts 12 hours from here.
        tokio::time::advance(LIFETIME / 2).await;
        let late = sessions.check(&id).unwrap();
        assert_eq!(late.valid_for, LIFETIME);

        tokio::time::advance(LIFETIME - Duration::from_secs(1)).await;
        assert!(sessions.check(&id).is_some());
        tokio::time::advance(LIFETIME + Duration::from_secs(1)).await;
        assert!(sessions.check(&id).is_none(), "12 hours without a request");
    }

    #[tokio::test(start_paused = true)]
    async fn use_never_keeps_a_session_past_7_days_from_the_sign_in() {
        let sessions = Sessions::default();
        let id = sessions.start();
        let ended = sessions.check(&id).unwrap().ended;
        let mut seen = Duration::ZERO;
        while seen + LIFETIME / 2 < ABSOLUTE_LIFETIME {
            tokio::time::advance(LIFETIME / 2 + Duration::from_secs(1)).await;
            seen += LIFETIME / 2 + Duration::from_secs(1);
            assert!(sessions.check(&id).is_some(), "after {seen:?}");
        }
        let left = ABSOLUTE_LIFETIME - seen;
        assert_eq!(sessions.check(&id).unwrap().valid_for, left);
        assert!(!ended.is_cancelled());
        tokio::time::advance(left).await;
        assert!(sessions.check(&id).is_none(), "the cap");
        assert!(ended.is_cancelled(), "its push ends");
    }

    #[tokio::test(start_paused = true)]
    async fn ending_one_session_cancels_its_token_and_leaves_the_others() {
        let sessions = Sessions::default();
        let (one, two) = (sessions.start(), sessions.start());
        let (a, b) = (
            sessions.check(&one).unwrap().ended,
            sessions.check(&two).unwrap().ended,
        );
        assert!(sessions.end(&one));
        assert!(a.is_cancelled() && !b.is_cancelled());
        assert!(sessions.check(&one).is_none());
        assert!(sessions.check(&two).is_some());
        assert!(!sessions.end(&one), "a copy of its cookie ends nothing");
    }

    #[tokio::test(start_paused = true)]
    async fn ending_all_but_one_keeps_that_one() {
        let sessions = Sessions::default();
        let ids: Vec<_> = (0..4).map(|_| sessions.start()).collect();
        let tokens: Vec<_> = ids
            .iter()
            .map(|id| sessions.check(id).unwrap().ended)
            .collect();
        sessions.end_all_but(Some(&ids[2]));
        assert_eq!(sessions.len(), 1);
        for (i, token) in tokens.iter().enumerate() {
            assert_eq!(token.is_cancelled(), i != 2, "session {i}");
        }
        assert!(sessions.check(&ids[2]).is_some());
        sessions.end_all_but(None);
        assert_eq!(sessions.len(), 0);
        assert!(tokens[2].is_cancelled());
    }

    #[tokio::test(start_paused = true)]
    async fn sessions_that_ran_out_are_dropped_and_the_table_is_bounded() {
        let sessions = Sessions::default();
        let old = sessions.start();
        let token = sessions.check(&old).unwrap().ended;
        tokio::time::advance(LIFETIME + SWEEP).await;
        let fresh = sessions.start();
        assert_eq!(sessions.len(), 1, "the old one was swept");
        assert!(token.is_cancelled());
        assert!(sessions.check(&fresh).is_some());

        let first = sessions.start();
        let first_token = sessions.check(&first).unwrap().ended;
        for _ in 0..MAX_SESSIONS * 2 {
            tokio::time::advance(Duration::from_millis(1)).await;
            sessions.start();
        }
        assert_eq!(sessions.len(), MAX_SESSIONS);
        assert!(sessions.check(&first).is_none(), "the oldest went first");
        assert!(first_token.is_cancelled());
    }

    #[test]
    fn the_cookie_has_the_prefix_and_the_attributes_it_needs() {
        let id = "A".repeat(ID_LENGTH);
        let header = set_cookie(&id);
        assert!(
            header.starts_with(&format!("{COOKIE_NAME}={id};")),
            "{header}"
        );
        let lower = header.to_ascii_lowercase();
        for attribute in ["path=/", "secure", "httponly", "samesite=strict"] {
            assert!(lower.contains(attribute), "{attribute} in {header}");
        }
        for absent in ["domain=", "expires=", "max-age="] {
            assert!(!lower.contains(absent), "{absent} in {header}");
        }
        assert!(COOKIE_NAME.starts_with("__Host-"));

        let cleared = clear_cookie();
        assert!(
            cleared.starts_with(&format!("{COOKIE_NAME}=;")),
            "{cleared}"
        );
        let lower = cleared.to_ascii_lowercase();
        assert!(
            lower.contains("max-age=0") && lower.contains("expires="),
            "{cleared}"
        );
        for attribute in ["path=/", "secure"] {
            assert!(lower.contains(attribute), "{attribute} in {cleared}");
        }
    }

    #[test]
    fn the_cookie_is_found_among_others_and_the_old_name_is_ignored() {
        let found = headers(&format!("a=1; {COOKIE_NAME}=xyz; b=2"));
        assert_eq!(from_request(&found).as_deref(), Some("xyz"));
        assert_eq!(from_request(&headers(&format!("{COOKIE_NAME}="))), None);
        assert_eq!(from_request(&headers("koppel.ui=xyz")), None);
        assert_eq!(from_request(&HeaderMap::new()), None);
        let mut two = HeaderMap::new();
        two.append(COOKIE, "a=1".parse().unwrap());
        two.append(COOKIE, format!("{COOKIE_NAME}=second").parse().unwrap());
        assert_eq!(from_request(&two).as_deref(), Some("second"));
    }
}
