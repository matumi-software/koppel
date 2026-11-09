//! Writes what `tracing` logs to the Application event log, under the source `Koppel`, as 0.1.x's
//! `Logging:EventLog` did: warnings and errors from everything, and information from the service
//! itself, so a start, a stop and its reason (the tray's Shut down and Restart) are there too.
//!
//! The installer registers the source with Koppel.exe as its message file. Koppel.exe carries a
//! message table (`koppel-bin/build.rs`) whose messages 1 to 3 are just `%1`, so Event Viewer shows
//! each entry's own text. Without the registration Windows still takes the entries, and Event Viewer
//! says it cannot find the description before it shows the text.

use std::fmt::Write as _;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::EventLog::{
    DeregisterEventSource, EVENTLOG_ERROR_TYPE, EVENTLOG_INFORMATION_TYPE, EVENTLOG_WARNING_TYPE,
    REPORT_EVENT_TYPE, RegisterEventSourceW, ReportEventW,
};
use windows::core::{HSTRING, PCWSTR};

use crate::SERVICE_NAME;

/// What goes to the event log: warnings and errors, and the service's own information.
pub fn filter() -> Targets {
    Targets::new()
        .with_default(Level::WARN)
        .with_target(module_path!().replace("event_log", "service"), Level::INFO)
}

/// The layer, filtered as [`filter`] says, or `None` when the event log cannot be opened.
pub fn layer<S>() -> Option<impl Layer<S>>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    EventLog::open().map(|log| log.with_filter(filter()))
}

/// The open event source. Its handle may be used from any thread.
struct EventLog(HANDLE);

// SAFETY: an event source handle is not tied to the thread that opened it; ReportEventW may be called
// with it from any thread.
unsafe impl Send for EventLog {}
// SAFETY: as above, ReportEventW is safe to call concurrently with one handle.
unsafe impl Sync for EventLog {}

impl EventLog {
    fn open() -> Option<Self> {
        // SAFETY: a local source by name; the string lives through the call.
        unsafe { RegisterEventSourceW(PCWSTR::null(), &HSTRING::from(SERVICE_NAME)) }
            .ok()
            .map(Self)
    }

    fn write(&self, level: Level, text: &str) {
        let (kind, id): (REPORT_EVENT_TYPE, u32) = match level {
            Level::ERROR => (EVENTLOG_ERROR_TYPE, 1),
            Level::WARN => (EVENTLOG_WARNING_TYPE, 2),
            _ => (EVENTLOG_INFORMATION_TYPE, 3),
        };
        let text = HSTRING::from(text);
        let strings = [PCWSTR(text.as_ptr())];
        // SAFETY: the handle is open, and the one string lives through the call. A write that fails
        // (the log is full, say) is dropped: there is nowhere else to say so.
        let _ = unsafe { ReportEventW(self.0, kind, 0, id, None, 0, Some(&strings), None) };
    }
}

impl Drop for EventLog {
    fn drop(&mut self) {
        // SAFETY: opened in `open` and closed once, here.
        let _ = unsafe { DeregisterEventSource(self.0) };
    }
}

impl<S: Subscriber> Layer<S> for EventLog {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut text = Text::default();
        event.record(&mut text);
        self.write(*event.metadata().level(), &text.finish());
    }
}

/// The entry's text: the message, then any other fields, one per line.
#[derive(Default)]
struct Text {
    message: String,
    fields: String,
}

impl Text {
    fn finish(self) -> String {
        if self.fields.is_empty() {
            self.message
        } else {
            format!("{}\r\n{}", self.message, self.fields)
        }
    }
}

impl Visit for Text {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, "\r\n{}: {value}", field.name());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, "\r\n{}: {value:?}", field.name());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_logs_information_and_the_rest_only_warnings() {
        let filter = filter();
        assert!(filter.would_enable("koppel_windows::service", &Level::INFO));
        assert!(!filter.would_enable("koppel_windows::service", &Level::DEBUG));
        assert!(!filter.would_enable("koppel_web::app", &Level::INFO));
        assert!(filter.would_enable("koppel_web::app", &Level::WARN));
    }
}
