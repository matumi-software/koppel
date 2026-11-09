//! The session cookie, `koppel.ui`: what it carries, signed so it cannot be made up or changed.
//!
//! The value is the session's id, the security stamp in force when it was signed in, when that was,
//! and when the cookie runs out, with an HMAC-SHA256 over them. Nothing in it is secret, and the
//! server still checks the stamp, the revoked ids and the 7 day cap on every request. The key is 32
//! random bytes in `ui-session.key` in the keys directory, sealed like the secret files, so sessions
//! survive a restart as they did in 0.1.x. 0.1.x's cookies were sealed by ASP.NET Core's own Data
//! Protection keys, so they end once, at the upgrade.

use std::path::Path;

use axum::http::HeaderMap;
use axum::http::header::COOKIE;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use koppel_config::SecretBox;
use ring::hmac;

/// The cookie's name, as 0.1.x named it.
pub(crate) const COOKIE_NAME: &str = "koppel.ui";
/// The key file's name in the keys directory.
pub(crate) const KEY_FILE: &str = "ui-session.key";
const VERSION: &str = "1";

/// One session, as the cookie carries it. Times are Unix seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ticket {
    pub session: String,
    pub stamp: String,
    pub issued: i64,
    pub expires: i64,
}

/// Signs and checks tickets.
pub(crate) struct CookieKey(hmac::Key);

impl CookieKey {
    /// The key in `keys/ui-session.key`, made when there is none. A key file that cannot be opened,
    /// such as one from another machine, is replaced: the sessions it signed end, nothing else.
    pub(crate) fn load(keys: &Path, secrets: &SecretBox) -> Result<Self, String> {
        let path = keys.join(KEY_FILE);
        if let Ok(sealed) = std::fs::read(&path) {
            match secrets.unprotect(&sealed) {
                Ok(key) if key.len() == 32 => return Ok(Self::from_bytes(&key)),
                Ok(_) => tracing::warn!(
                    "{} is not a session key; making a new one, which signs everyone out",
                    path.display()
                ),
                Err(e) => tracing::warn!(
                    "{} cannot be opened ({e}); making a new one, which signs everyone out",
                    path.display()
                ),
            }
        }
        let key = super::random_bytes::<32>();
        let sealed = secrets
            .protect(&key)
            .map_err(|e| format!("the session key could not be sealed: {e}"))?;
        write_new(&path, &sealed)
            .map_err(|e| format!("{} could not be written: {e}", path.display()))?;
        Ok(Self::from_bytes(&key))
    }

    pub(crate) fn from_bytes(key: &[u8]) -> Self {
        Self(hmac::Key::new(hmac::HMAC_SHA256, key))
    }

    /// The cookie's value for `ticket`.
    pub(crate) fn seal(&self, ticket: &Ticket) -> String {
        let payload = format!(
            "{VERSION}|{}|{}|{}|{}",
            ticket.session, ticket.stamp, ticket.issued, ticket.expires
        );
        let tag = hmac::sign(&self.0, payload.as_bytes());
        format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(payload),
            URL_SAFE_NO_PAD.encode(tag.as_ref())
        )
    }

    /// The ticket in a cookie value this key signed, or `None` for anything else.
    pub(crate) fn open(&self, value: &str) -> Option<Ticket> {
        let (payload, tag) = value.split_once('.')?;
        let payload = URL_SAFE_NO_PAD.decode(payload).ok()?;
        let tag = URL_SAFE_NO_PAD.decode(tag).ok()?;
        hmac::verify(&self.0, &payload, &tag).ok()?;
        let payload = String::from_utf8(payload).ok()?;
        let mut parts = payload.split('|');
        if parts.next()? != VERSION {
            return None;
        }
        let ticket = Ticket {
            session: parts.next()?.to_owned(),
            stamp: parts.next()?.to_owned(),
            issued: parts.next()?.parse().ok()?,
            expires: parts.next()?.parse().ok()?,
        };
        parts.next().is_none().then_some(ticket)
    }
}

/// Writes a new file through a temp name, so a crash never leaves half a key.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temp = path.with_extension(format!(
        "{}.tmp",
        crate::activity::hex(&super::random_bytes::<4>())
    ));
    std::fs::write(&temp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// The session cookie's value in the request, if it has one.
pub(crate) fn from_request(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE_NAME)
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// `Set-Cookie` for a session: HttpOnly, SameSite=Strict, Secure, and a session cookie, since how
/// long it lasts is in the ticket. The attributes are written as ASP.NET wrote them.
pub(crate) fn set_cookie(value: &str) -> String {
    format!("{COOKIE_NAME}={value}; path=/; secure; samesite=strict; httponly")
}

/// `Set-Cookie` that removes the session cookie.
pub(crate) fn clear_cookie() -> String {
    format!(
        "{COOKIE_NAME}=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/; secure; samesite=strict; httponly"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_ticket_opens_and_a_changed_one_does_not() {
        let key = CookieKey::from_bytes(&[7; 32]);
        let ticket = Ticket {
            session: "abc".into(),
            stamp: "def".into(),
            issued: 1,
            expires: 2,
        };
        let value = key.seal(&ticket);
        assert_eq!(key.open(&value), Some(ticket));
        assert_eq!(CookieKey::from_bytes(&[8; 32]).open(&value), None);
        let (payload, tag) = value.split_once('.').unwrap();
        let forged = URL_SAFE_NO_PAD.encode("1|abc|def|1|999999");
        assert_eq!(key.open(&format!("{forged}.{tag}")), None);
        assert_eq!(key.open(payload), None);
        assert_eq!(key.open("not-a-session"), None);
    }

    #[test]
    fn the_cookie_is_found_among_others() {
        let mut headers = HeaderMap::new();
        headers.append(COOKIE, "a=1; koppel.ui=xyz; b=2".parse().unwrap());
        assert_eq!(from_request(&headers).as_deref(), Some("xyz"));
        let mut empty = HeaderMap::new();
        empty.append(COOKIE, "koppel.ui=".parse().unwrap());
        assert_eq!(from_request(&empty), None);
    }
}
