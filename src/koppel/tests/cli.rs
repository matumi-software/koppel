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
