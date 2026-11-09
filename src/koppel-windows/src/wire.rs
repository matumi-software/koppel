//! The lines the service writes to each tray's standard input, as 0.1.x's `TrayWire`: one JSON object
//! per line, with a `type`. A tray skips a line it cannot read or a type it does not know, so a newer
//! service can add messages without breaking an older tray.
//!
//! - `{"type":"state","tip":"Koppel: ...","attention":false}`
//! - `{"type":"notice","title":"...","text":"...","page":"#clients"}`

use koppel_web::tray::{MAX_TEXT, MAX_TIP, MAX_TITLE, TrayNotice, TrayState, cut};
use serde_json::{Value, json};

/// The `state` line for `state`, without the newline.
pub fn state_line(state: &TrayState) -> String {
    json!({ "type": "state", "tip": state.tip, "attention": state.attention }).to_string()
}

/// The `notice` line for `notice`, without the newline.
pub fn notice_line(notice: &TrayNotice) -> String {
    json!({ "type": "notice", "title": notice.title, "text": notice.text, "page": notice.page })
        .to_string()
}

/// What one line said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    State(TrayState),
    Notice(TrayNotice),
}

/// The message in `line`, or `None` when it is not one this tray can read.
pub fn read(line: &str) -> Option<Message> {
    let value: Value = serde_json::from_str(line).ok()?;
    let object = value.as_object()?;
    let text = |name: &str| object.get(name).and_then(Value::as_str);
    match text("type")? {
        "state" => {
            let tip = text("tip")?;
            let attention = object.get("attention") == Some(&Value::Bool(true));
            Some(Message::State(TrayState {
                tip: truncate(tip, MAX_TIP),
                attention,
            }))
        }
        "notice" => {
            let title = text("title")?;
            let body = text("text")?;
            Some(Message::Notice(TrayNotice {
                title: cut(title, MAX_TITLE),
                text: cut(body, MAX_TEXT),
                page: text("page").filter(|p| is_page(p)).unwrap_or("").to_owned(),
            }))
        }
        _ => None,
    }
}

/// One of the UI's own pages: `#` and lower-case letters, so a line can never send the browser
/// anywhere else.
fn is_page(page: &str) -> bool {
    page.len() > 1
        && page.len() <= 32
        && page.starts_with('#')
        && page[1..].bytes().all(|b| b.is_ascii_lowercase())
}

/// `text` cut to `max` UTF-16 units, without the `...`, as 0.1.x read a tooltip.
fn truncate(text: &str, max: usize) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= max
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_goes_both_ways() {
        let state = TrayState {
            tip: "Koppel: server, 1 device shared, 0 in use".into(),
            attention: true,
        };
        assert_eq!(read(&state_line(&state)), Some(Message::State(state)));
    }

    #[test]
    fn a_notice_goes_both_ways() {
        let notice = TrayNotice {
            title: "A client wants to pair".into(),
            text: "lab-pc is waiting.".into(),
            page: "#clients".into(),
        };
        assert_eq!(read(&notice_line(&notice)), Some(Message::Notice(notice)));
    }

    #[test]
    fn what_it_cannot_read_is_skipped() {
        for line in [
            "",
            "not json",
            "[]",
            r#"{"type":"later","tip":"x"}"#,
            r#"{"type":"state"}"#,
            r#"{"type":"state","tip":5}"#,
            r#"{"type":"notice","title":"x"}"#,
        ] {
            assert_eq!(read(line), None, "{line}");
        }
    }

    #[test]
    fn a_page_is_one_of_the_uis_own() {
        for (page, kept) in [
            ("#clients", "#clients"),
            ("https://example.com", ""),
            ("#", ""),
            ("#Clients", ""),
            ("#a/b", ""),
            ("javascript:alert(1)", ""),
        ] {
            let line = format!(r#"{{"type":"notice","title":"t","text":"x","page":"{page}"}}"#);
            let Some(Message::Notice(notice)) = read(&line) else {
                panic!("{line}")
            };
            assert_eq!(notice.page, kept, "{page}");
        }
    }

    #[test]
    fn long_values_are_cut_to_what_windows_holds() {
        let line = json!({ "type": "state", "tip": "x".repeat(300) }).to_string();
        let Some(Message::State(state)) = read(&line) else {
            panic!()
        };
        assert_eq!(state.tip.len(), MAX_TIP);
        assert!(!state.attention);
        let line = json!({ "type": "notice", "title": "t".repeat(100), "text": "x".repeat(300) })
            .to_string();
        let Some(Message::Notice(notice)) = read(&line) else {
            panic!()
        };
        assert_eq!(notice.title.len(), MAX_TITLE);
        assert_eq!(notice.text.len(), MAX_TEXT);
        assert!(notice.text.ends_with("..."));
    }
}
