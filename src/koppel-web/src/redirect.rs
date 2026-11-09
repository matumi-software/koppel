//! Plain HTTP on an HTTPS port gets a redirect to the same address over HTTPS, so
//! `http://host:5240` in a browser lands on the UI instead of an empty page, as 0.1.x's
//! `PlainHttpRedirect` does. The first byte decides: a TLS handshake starts with 0x16 and goes on
//! untouched; anything else is read as a request head and gets a fixed 301, then the connection is
//! closed. The plain request never reaches the app.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The first byte of every TLS handshake.
pub(crate) const TLS_HANDSHAKE: u8 = 0x16;
/// A request head larger than this is not a browser asking for a page: the connection is closed.
const MAX_HEAD: usize = 8 * 1024;
/// Long enough for a slow client, short enough that a silent connection does not sit here.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Reads a plain HTTP request head from `stream` and answers it with a 301 to HTTPS, or closes it
/// when it is not one.
pub(crate) async fn answer(mut stream: TcpStream, local: SocketAddr) {
    let read = async {
        let mut head = Vec::with_capacity(1024);
        let mut buffer = [0u8; 1024];
        loop {
            if let Some(location) = location(&head, local) {
                return Some(location);
            }
            if head.len() >= MAX_HEAD || end_of_head(&head).is_some() {
                return None;
            }
            let n = stream.read(&mut buffer).await.ok()?;
            if n == 0 {
                return None;
            }
            head.extend_from_slice(&buffer[..n]);
        }
    };
    let Ok(Some(location)) = tokio::time::timeout(READ_TIMEOUT, read).await else {
        return;
    };
    let answer = format!(
        "HTTP/1.1 301 Moved Permanently\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let _ = tokio::time::timeout(READ_TIMEOUT, async {
        let _ = stream.write_all(answer.as_bytes()).await;
        let _ = stream.shutdown().await;
    })
    .await;
}

fn end_of_head(head: &[u8]) -> Option<usize> {
    head.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Where a complete plain HTTP request head should go over HTTPS, or `None` when the head is not
/// complete or not an HTTP request. The host comes from the Host header when it is a plain host name
/// or address with an optional port, otherwise from the address the connection arrived on, so a
/// request cannot put anything else into the Location header.
fn location(head: &[u8], local: SocketAddr) -> Option<String> {
    let end = end_of_head(head)?;
    let text = std::str::from_utf8(&head[..end]).ok()?;
    let mut lines = text.split("\r\n");
    let parts: Vec<&str> = lines.next()?.split(' ').collect();
    let [_, target, version] = parts.as_slice() else {
        return None;
    };
    if !version.starts_with("HTTP/1.") || !target.starts_with('/') {
        return None;
    }
    let host = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("host").then(|| value.trim())
        })
        .find(|host| is_plain_host(host))
        .map(str::to_owned)
        .unwrap_or_else(|| local.to_string());
    Some(format!("https://{host}{target}"))
}

fn is_plain_host(host: &str) -> bool {
    (1..=255).contains(&host.len())
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> SocketAddr {
        "10.0.0.5:5240".parse().unwrap()
    }

    #[test]
    fn a_request_goes_to_the_same_host_and_path_over_https() {
        let head = b"GET /some/page?x=1 HTTP/1.1\r\nHost: localhost:15240\r\n\r\n";
        assert_eq!(
            location(head, local()).as_deref(),
            Some("https://localhost:15240/some/page?x=1")
        );
    }

    #[test]
    fn a_host_header_with_anything_else_in_it_is_not_used() {
        let head = b"GET / HTTP/1.1\r\nHost: evil.example/\"x\r\n\r\n";
        assert_eq!(
            location(head, local()).as_deref(),
            Some("https://10.0.0.5:5240/")
        );
    }

    #[test]
    fn an_incomplete_or_foreign_head_gets_nothing() {
        assert_eq!(location(b"GET / HTTP/1.1\r\nHost: a", local()), None);
        assert_eq!(location(b"SSH-2.0-OpenSSH\r\n\r\n", local()), None);
        assert_eq!(location(b"GET x HTTP/1.1\r\n\r\n", local()), None);
    }
}
