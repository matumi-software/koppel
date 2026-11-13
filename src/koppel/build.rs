//! Writes `KOPPEL_LONG_VERSION`, what `koppel --version` prints: the version, the git commit and the
//! build date, and `KOPPEL_COMMIT` and `KOPPEL_BUILT` for the system info. The commit is `KOPPEL_GIT_COMMIT` when set (the image build has no `.git`, so it passes
//! it in), else `git rev-parse`, else `unknown`. The date is UTC, from `SOURCE_DATE_EPOCH` when set so a
//! build can be repeated.
//!
//! On Windows, gives Koppel.exe Koppel's icon, its version, and the message table the event log
//! reads its entries with (see koppel-windows' `event_log`): the installer names Koppel.exe as the
//! event source's message file.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=KOPPEL_GIT_COMMIT");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let version = std::env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
    let (commit, built) = (git_commit(), build_date());
    println!("cargo:rustc-env=KOPPEL_LONG_VERSION={version} (commit {commit}, built {built})");
    // The same two facts alone, for the UI's system info.
    println!("cargo:rustc-env=KOPPEL_COMMIT={commit}");
    println!("cargo:rustc-env=KOPPEL_BUILT={built}");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let icon = "../../resources/design-system/assets/Logos/koppel-app-icon.ico";
        println!("cargo:rerun-if-changed={icon}");
        let out =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
        let table = out.join("messages.bin");
        std::fs::write(&table, message_table()).expect("the message table is written");
        winresource::WindowsResource::new()
            .set_icon(icon)
            .set("FileDescription", "Koppel")
            .set("ProductName", "Koppel")
            .set("CompanyName", "Matumi (Pty) Ltd")
            .set("LegalCopyright", "Copyright (C) 2026 Matumi (Pty) Ltd")
            .append_rc_content(&format!(
                "1 MESSAGETABLE \"{}\"",
                table.display().to_string().replace('\\', "/")
            ))
            .compile()
            .expect("the Windows resource compiler embeds the icon and the message table");
    }
}

/// A message table, as the message compiler would write it, with messages 1 to 3 (error, warning,
/// information) that are each just the entry's own text, `%1`. Each entry is its length, a flag for
/// UTF-16, and the text with a terminating zero, padded to four bytes.
#[cfg(windows)]
fn message_table() -> Vec<u8> {
    const FIRST: u32 = 1;
    const LAST: u32 = 3;
    let text: Vec<u8> = "%1\r\n\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let length = (4 + text.len()).next_multiple_of(4);
    let mut table = Vec::new();
    table.extend_from_slice(&1u32.to_le_bytes());
    table.extend_from_slice(&FIRST.to_le_bytes());
    table.extend_from_slice(&LAST.to_le_bytes());
    table.extend_from_slice(&16u32.to_le_bytes());
    for _ in FIRST..=LAST {
        table.extend_from_slice(&(length as u16).to_le_bytes());
        table.extend_from_slice(&1u16.to_le_bytes());
        table.extend_from_slice(&text);
        table.resize(table.len() + length - 4 - text.len(), 0);
    }
    table
}

/// The short commit the build is of.
fn git_commit() -> String {
    if let Ok(commit) = std::env::var("KOPPEL_GIT_COMMIT") {
        let commit = commit.trim();
        if !commit.is_empty() {
            return commit.chars().take(9).collect();
        }
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .filter(|text| !text.is_empty())
    };
    // The reflog changes on every commit and checkout, so a new commit rebuilds this.
    if let Some(log) = git(&["rev-parse", "--git-path", "logs/HEAD"]) {
        println!("cargo:rerun-if-changed={log}");
    }
    git(&["rev-parse", "--short=9", "HEAD"]).unwrap_or_else(|| "unknown".to_owned())
}

/// Today's date in UTC, `2026-11-12`.
fn build_date() -> String {
    let seconds = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_secs() as i64)
        });
    // Days since 1970-01-01 to a calendar date, as Howard Hinnant's civil_from_days.
    let days = seconds.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}
