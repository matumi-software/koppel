//! The program's command line and environment, through the built binary. None of these starts the
//! server: they print and exit.

use std::net::TcpListener;
use std::process::{Command, Output};

fn koppel(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_koppel"));
    command.args(args);
    // A clean slate: what the developer or the container sets must not leak into the run.
    for (name, _) in std::env::vars_os().filter_map(|(k, v)| Some((k.to_str()?.to_owned(), v))) {
        let upper = name.to_ascii_uppercase();
        if upper.starts_with("KOPPEL_") || name.contains("__") || upper.ends_with("_ENVIRONMENT") {
            command.env_remove(&name);
        }
    }
    command.envs(env.iter().copied());
    command.output().expect("the binary runs")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A loopback port nothing listens on.
fn closed_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn version_names_the_commit_and_the_build_date() {
    let out = koppel(&["--version"], &[]);
    assert!(out.status.success());
    let line = text(&out.stdout);
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        line.starts_with(&format!("koppel {version} (commit ")),
        "{line}"
    );
    assert!(line.contains(", built 20"), "{line}");
}

#[test]
fn help_lists_the_commands_and_the_environment_names() {
    let out = koppel(&["--help"], &[]);
    assert!(out.status.success());
    let help = text(&out.stdout);
    for word in [
        "serve",
        "reset-password",
        "healthcheck",
        "KOPPEL_UI_PORT",
        "--config-dir",
    ] {
        assert!(help.contains(word), "{word} is not in:\n{help}");
    }
}

#[test]
fn a_bad_value_stops_before_anything_starts_and_names_the_option() {
    for (env, name) in [
        (("KOPPEL_UI_PORT", "x"), "--ui-port"),
        (("Ports__Ui", "x"), "--ui-port"),
        (("KOPPEL_UI_ALLOW_FROM", "everyone"), "--ui-allow-from"),
    ] {
        let out = koppel(&["serve"], &[env]);
        assert_eq!(out.status.code(), Some(2), "{env:?}");
        assert!(text(&out.stderr).contains(name), "{}", text(&out.stderr));
    }
}

#[test]
fn an_unknown_option_does_not_start_the_server() {
    let out = koppel(&["--no-such-option"], &[]);
    assert_eq!(out.status.code(), Some(2));
}

/// Arguments and environment variables for one run.
type Run<'a> = (&'a [&'a str], &'a [(&'a str, &'a str)]);

#[test]
fn healthcheck_reads_the_new_and_the_old_names() {
    let port = closed_port().to_string();
    let cases: [Run; 4] = [
        (&["healthcheck"], &[("KOPPEL_UI_PORT", port.as_str())]),
        (&["healthcheck", "--ui-port", port.as_str()], &[]),
        (&["--healthcheck"], &[("Ports__Ui", port.as_str())]),
        (&["--healthcheck", "--Ports:Ui", port.as_str()], &[]),
    ];
    for (args, env) in cases {
        let out = koppel(args, env);
        assert_eq!(out.status.code(), Some(1), "{args:?} {env:?}");
        assert!(
            text(&out.stderr).contains(&format!("127.0.0.1:{port}/healthz")),
            "{args:?} {env:?}: {}",
            text(&out.stderr)
        );
    }
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("koppel-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// What the installer relies on: `prepare-data-folder --check` exits 1 and says why on standard
/// error, whatever is where the folder should be, and changes nothing. (Other systems answer that
/// the command is for Windows, with the same exit code.)
#[test]
fn prepare_data_folder_check_fails_with_a_reason_and_changes_nothing() {
    let base = scratch("prepare-check");
    let missing = base.join("Koppel");
    let out = koppel(
        &["prepare-data-folder", "--check", missing.to_str().unwrap()],
        &[],
    );
    assert_eq!(out.status.code(), Some(1));
    let said = text(&out.stderr);
    if cfg!(windows) {
        assert!(said.contains("does not exist."), "{said}");
    } else {
        assert!(said.contains("is for Windows"), "{said}");
    }
    assert!(!missing.exists(), "--check made the folder");

    let file = base.join("file");
    std::fs::write(&file, b"x").unwrap();
    let out = koppel(
        &["prepare-data-folder", "--check", file.to_str().unwrap()],
        &[],
    );
    assert_eq!(out.status.code(), Some(1));
    if cfg!(windows) {
        assert!(text(&out.stderr).contains("is a file. Koppel needs a folder there."));
    }
    assert_eq!(std::fs::read(&file).unwrap(), b"x");
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(windows)]
#[test]
fn prepare_data_folder_refuses_a_junction_and_leaves_its_target_alone() {
    let base = scratch("prepare-junction");
    let target = base.join("target");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep.txt"), b"keep").unwrap();
    let link = base.join("Koppel");
    let made = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .output()
        .unwrap();
    assert!(made.status.success());
    for args in [
        vec!["prepare-data-folder", "--check", link.to_str().unwrap()],
        vec!["prepare-data-folder", link.to_str().unwrap()],
    ] {
        let out = koppel(&args, &[]);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let said = text(&out.stderr);
        // Without elevation the second call stops earlier, which is also a refusal.
        assert!(
            said.contains("junction or symbolic link")
                || said.contains("run this as an administrator"),
            "{said}"
        );
    }
    assert_eq!(std::fs::read(target.join("keep.txt")).unwrap(), b"keep");
    let _ = std::fs::remove_dir(&link);
    let _ = std::fs::remove_dir_all(base);
}

/// Not elevated, the preparing call stops before it touches anything.
#[cfg(windows)]
#[test]
fn prepare_data_folder_without_elevation_changes_nothing() {
    let elevated = Command::new("net")
        .arg("session")
        .output()
        .is_ok_and(|out| out.status.success());
    if elevated {
        return;
    }
    let base = scratch("prepare-unelevated");
    let folder = base.join("Koppel");
    let out = koppel(&["prepare-data-folder", folder.to_str().unwrap()], &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("run this as an administrator"));
    assert!(!folder.exists());
    let _ = std::fs::remove_dir_all(base);
}
