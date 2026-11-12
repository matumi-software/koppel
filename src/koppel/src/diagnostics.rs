//! `koppel diagnostics [FILE]`: the report `koppel_config::diagnostics` makes, for a support request.
//! It goes to `file` when one is named and to standard output otherwise. Run it as an administrator
//! (Windows) or as root (Linux): the config folder is closed to everyone else.

use std::path::Path;
use std::process::ExitCode;

use koppel_config::diagnostics::{Facts, report};

use crate::config::Settings;

/// Writes the report to `file`, or to standard output, and returns the exit code.
pub fn run(settings: &Settings, file: Option<&str>) -> ExitCode {
    let paths = match std::env::current_dir() {
        Ok(root) => settings.paths(&root),
        Err(e) => {
            eprintln!("Cannot read the working folder: {e}");
            return ExitCode::FAILURE;
        }
    };
    let facts = Facts {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        os: koppel_web::os_label(),
        machine: koppel_web::machine_name(),
    };
    let text = report(&facts, &paths, &koppel_config::log_folder());
    match file {
        Some(file) => match std::fs::write(Path::new(file), text) {
            Ok(()) => {
                println!("Wrote {file}. Attach it to the support request.");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Could not write {file}: {e}");
                ExitCode::FAILURE
            }
        },
        None => {
            print!("{text}");
            ExitCode::SUCCESS
        }
    }
}
