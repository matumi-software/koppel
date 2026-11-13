//! `koppel prepare-data-folder [--check] [PATH]`: what the Windows installer runs, as SYSTEM, to make
//! `%ProgramData%\Koppel` usable by SYSTEM and administrators only (see `koppel_config::data_folder`
//! for the rules). It exits 0 when the folder is safe and 1 when it is not or a call failed, with the
//! reason on standard error. What it changed goes to standard output, which the MSI log captures.

use std::process::ExitCode;

/// Runs the command and returns the exit code.
pub fn run(check_only: bool, path: Option<&str>) -> ExitCode {
    imp::run(check_only, path)
}

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;
    use std::process::ExitCode;

    use koppel_config::{SystemAccounts, SystemFiles, check_data_folder, prepare_data_folder};

    pub fn run(check_only: bool, path: Option<&str>) -> ExitCode {
        let path = path.map_or_else(koppel_config::default_root, PathBuf::from);
        let files = match SystemFiles::new() {
            Ok(files) => files,
            Err(e) => {
                eprintln!("Cannot tell whether this process is an administrator: {e}");
                return ExitCode::FAILURE;
            }
        };
        let result = if check_only {
            check_data_folder(&path, &SystemAccounts, &files)
        } else if !files.privileged() {
            eprintln!(
                "{} cannot be prepared from here: run this as an administrator (an elevated prompt) or as SYSTEM.",
                path.display()
            );
            return ExitCode::FAILURE;
        } else {
            prepare_data_folder(&path, &SystemAccounts, &files, &mut |line| {
                println!("{line}");
            })
        };
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::process::ExitCode;

    pub fn run(_check_only: bool, _path: Option<&str>) -> ExitCode {
        eprintln!(
            "prepare-data-folder is for Windows, where the installer makes the data folder. Koppel's other systems restrict it when they start."
        );
        ExitCode::FAILURE
    }
}
