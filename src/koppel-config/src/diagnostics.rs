//! `koppel diagnostics`: one text report to attach to a support request. It holds the version, the OS,
//! the folders Koppel uses, the config files (name, size and date), the plain ones with every
//! value that could be a secret replaced, the USB module state and the tail of the log file.
//!
//! What is left out on purpose: the encrypted files (`secrets.json`, `connection.json`,
//! `ui-auth.json`) and everything in the keys folder are listed by name and size and never read.
//! A plain file is read as JSON and walked: a value under a name that contains `password`,
//! `secret`, `token`, `key`, `credential`, `passphrase`, `hash` or `stamp` is replaced by
//! `"<removed>"`, so a name added to a file later is covered without changing this module.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use serde_json::Value;
use time::OffsetDateTime;

use crate::paths::{KoppelPaths, default_root};

/// The most of the log's end the report holds, in bytes.
const LOG_TAIL_BYTES: u64 = 64 * 1024;
/// A plain config file larger than this is named but not included.
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// The service's current log file, written by `koppel-windows`.
const LOG_FILE: &str = "koppel.log";
/// What a value under a secret-looking name becomes.
const REMOVED: &str = "<removed>";
/// Parts of a name that make its value a secret.
const SECRET_NAMES: [&str; 8] = [
    "password",
    "secret",
    "token",
    "key",
    "credential",
    "passphrase",
    "hash",
    "stamp",
];
/// The config files the report includes, with the secret-looking names removed.
const PLAIN_FILES: [&str; 3] = ["settings.json", "devices.json", "attached.json"];

/// What the caller knows and this crate does not.
pub struct Facts {
    pub version: String,
    pub os: String,
    pub machine: String,
}

/// The report, as text.
pub fn report(facts: &Facts, paths: &KoppelPaths, logs: &Path) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Koppel diagnostics");
    let _ = writeln!(out, "Made at: {}", OffsetDateTime::now_utc());
    let _ = writeln!(out, "Version: {}", facts.version);
    let _ = writeln!(out, "OS: {}", facts.os);
    let _ = writeln!(out, "Machine: {}", facts.machine);
    let _ = writeln!(
        out,
        "Program: {}",
        std::env::current_exe()
            .map_or_else(|e| format!("unknown ({e})"), |p| p.display().to_string())
    );

    section(&mut out, "Folders");
    let _ = writeln!(out, "Config: {}", paths.config.display());
    let _ = writeln!(out, "Keys: {}", paths.keys.display());
    let _ = writeln!(out, "Logs: {}", logs.display());

    section(&mut out, "Config files");
    list(&mut out, &paths.config);
    section(&mut out, "Keys folder (names only, never read)");
    list(&mut out, &paths.keys);
    for name in PLAIN_FILES {
        section(&mut out, &format!("{name} (secrets removed)"));
        out.push_str(&plain_file(&paths.config.join(name)));
        out.push('\n');
    }

    section(&mut out, "USB");
    out.push_str(&usb_state(&default_root()));

    section(&mut out, "Recent log");
    out.push_str(&log_tail(logs));
    out
}

fn section(out: &mut String, title: &str) {
    let _ = write!(out, "\n== {title} ==\n");
}

/// The files in `folder`, one a line: name, size, modified time. Never their content.
fn list(out: &mut String, folder: &Path) {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) => {
            let _ = writeln!(out, "Cannot read the folder: {e}");
            return;
        }
    };
    let mut lines: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            match entry.metadata() {
                Ok(meta) => {
                    let when = meta
                        .modified()
                        .ok()
                        .map(OffsetDateTime::from)
                        .map_or_else(|| "unknown".to_owned(), |t| t.to_string());
                    let kind = if meta.is_dir() { " (folder)" } else { "" };
                    format!("{name}{kind}: {} bytes, modified {when}", meta.len())
                }
                Err(e) => format!("{name}: {e}"),
            }
        })
        .collect();
    lines.sort();
    if lines.is_empty() {
        let _ = writeln!(out, "(empty)");
    }
    for line in lines {
        let _ = writeln!(out, "{line}");
    }
}

/// `path` as JSON with the secrets removed, or why it is not there.
fn plain_file(path: &Path) -> String {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) => return format!("(not read: {e})"),
    };
    if meta.len() > MAX_FILE_BYTES {
        return format!("(not included: {} bytes)", meta.len());
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => return format!("(not read: {e})"),
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(mut value) => {
            remove_secrets(&mut value);
            serde_json::to_string_pretty(&value).unwrap_or_else(|e| format!("(not written: {e})"))
        }
        // The error says where, not what the text was, so it cannot repeat a secret.
        Err(e) => format!("(not valid JSON, so not included: {e})"),
    }
}

/// Replaces, in place, every value under a secret-looking name, at any depth.
pub fn remove_secrets(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (name, child) in map.iter_mut() {
                let lower = name.to_ascii_lowercase();
                if SECRET_NAMES.iter().any(|part| lower.contains(part)) {
                    *child = Value::String(REMOVED.to_owned());
                } else {
                    remove_secrets(child);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(remove_secrets),
        _ => {}
    }
}

/// The USB/IP kernel modules on Linux, or the driver record the installer keeps on Windows.
fn usb_state(root: &Path) -> String {
    #[cfg(target_os = "linux")]
    {
        let _ = root;
        let mut out = String::new();
        for module in ["usbip_core", "usbip_host", "vhci_hcd"] {
            let loaded = Path::new("/sys/module").join(module).exists();
            let _ = writeln!(
                out,
                "{module}: {}",
                if loaded { "loaded" } else { "not loaded" }
            );
        }
        out
    }
    #[cfg(not(target_os = "linux"))]
    {
        let record = root.join("driver-state.json");
        let mut out = format!("Driver record {}:\n", record.display());
        out.push_str(&plain_file(&record));
        out.push('\n');
        out
    }
}

/// The end of the current log file, from a line start, or why there is none. The rotated files are
/// left out: the end of the current one is what a support request needs.
fn log_tail(folder: &Path) -> String {
    let path = folder.join(LOG_FILE);
    if !path.is_file() {
        let why = if cfg!(windows) {
            "the service writes it once it has run as the Windows service"
        } else {
            "on Linux the service logs to the journal, `journalctl -u koppel`"
        };
        return format!(
            "(no log file in {}: {why})
",
            folder.display()
        );
    }
    match read_tail(&path, LOG_TAIL_BYTES) {
        Ok(text) => text,
        Err(e) => format!("(could not read {}: {e})\n", path.display()),
    }
}

/// The last `max` bytes of `path`, starting at a line.
fn read_tail(path: &Path, max: u64) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(max);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0
        && let Some(newline) = text.find('\n')
    {
        text.drain(..=newline);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("koppel-diag-{name}-{}", crate::random_hex(6)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn secret_names_are_removed_at_any_depth() {
        let mut value: Value = serde_json::from_str(
            r#"{"port":5240,"uiPassword":"hunter2","usb":{"upstream":{"ApiToken":"abc","host":"h"}},
                "list":[{"secret":"s","name":"n"}],"stamp":1}"#,
        )
        .unwrap();
        remove_secrets(&mut value);
        let text = value.to_string();
        for secret in ["hunter2", "abc", "\"s\"", ":1}"] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
        assert!(text.contains("5240") && text.contains("\"host\":\"h\"") && text.contains("\"n\""));
        assert!(text.contains(REMOVED));
    }

    #[test]
    fn the_report_names_the_encrypted_files_and_never_reads_them() {
        let base = folder("report");
        let paths = KoppelPaths::new(base.join("config"), base.join("keys"));
        fs::create_dir_all(&paths.config).unwrap();
        fs::create_dir_all(&paths.keys).unwrap();
        fs::write(paths.config.join("secrets.json"), "TOP-SECRET-BYTES").unwrap();
        fs::write(paths.keys.join("key.bin"), "KEY-BYTES").unwrap();
        fs::write(
            paths.config.join("settings.json"),
            r#"{"role":"server","password":"PW-VALUE","token":"TOKEN-VALUE"}"#,
        )
        .unwrap();
        let logs = base.join("logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join("koppel.log"), "first\nsecond\n").unwrap();

        let text = report(
            &Facts {
                version: "9.9.9".into(),
                os: "Test OS".into(),
                machine: "box".into(),
            },
            &paths,
            &logs,
        );
        assert!(text.contains("Version: 9.9.9") && text.contains("OS: Test OS"));
        assert!(text.contains("secrets.json: 16 bytes"));
        assert!(text.contains("key.bin: 9 bytes"));
        assert!(text.contains("\"role\": \"server\""));
        for hidden in ["TOP-SECRET-BYTES", "KEY-BYTES", "PW-VALUE", "TOKEN-VALUE"] {
            assert!(!text.contains(hidden), "{hidden} leaked");
        }
        assert!(text.contains("first\nsecond\n"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn a_missing_log_and_a_broken_file_are_said_not_fatal() {
        let base = folder("missing");
        let paths = KoppelPaths::new(base.join("config"), base.join("keys"));
        fs::create_dir_all(&paths.config).unwrap();
        fs::write(paths.config.join("settings.json"), "{ not json PW-VALUE").unwrap();
        let text = report(
            &Facts {
                version: "1".into(),
                os: "x".into(),
                machine: "m".into(),
            },
            &paths,
            &base.join("logs"),
        );
        assert!(text.contains("no log file"));
        assert!(text.contains("not valid JSON"));
        assert!(!text.contains("PW-VALUE"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn the_log_tail_starts_at_a_line() {
        let base = folder("tail");
        let file = base.join("koppel.log");
        fs::write(&file, "aaaa\nbbbb\ncccc\n").unwrap();
        assert_eq!(read_tail(&file, 8).unwrap(), "cccc\n");
        assert_eq!(read_tail(&file, 100).unwrap(), "aaaa\nbbbb\ncccc\n");
        let _ = fs::remove_dir_all(base);
    }
}
