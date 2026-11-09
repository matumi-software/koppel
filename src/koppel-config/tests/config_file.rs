//! The write and read rules of a config file, as docs/architecture.md's "Config on disk" lists them.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use koppel_config::files::{KoppelSettings, Role, SharedDevices};
use koppel_config::{ConfigFile, KoppelPaths, Options, SecretBox, json, open};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "koppel-config-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn paths(dir: &Path) -> KoppelPaths {
    let paths = KoppelPaths::new(dir.join("config"), dir.join("keys"));
    paths.prepare().unwrap();
    paths
}

/// Waits up to five seconds for `condition`.
async fn eventually(mut condition: impl FnMut() -> bool) {
    for _ in 0..100 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out");
}

fn client() -> impl Fn(&KoppelSettings) -> KoppelSettings {
    |s| KoppelSettings {
        role: Role::Client,
        ..s.clone()
    }
}

#[tokio::test]
async fn a_missing_file_is_seeded_with_the_defaults_as_0_1_writes_them() {
    let dir = temp_dir("seed");
    let paths = paths(&dir);
    let settings = open::settings(&paths);
    settings.start().await.unwrap();

    let written = fs::read(paths.settings_file()).unwrap();
    let expected = "{\n  \"role\": \"server\",\n  \"shareOnStartup\": true,\n  \"setupComplete\": false,\n  \"notifyPairing\": true,\n  \"notifyLink\": true,\n  \"notifyAttachFailures\": true\n}";
    let expected = expected.replace('\n', if cfg!(windows) { "\r\n" } else { "\n" });
    assert_eq!(String::from_utf8(written).unwrap(), expected);
    settings.stop();
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn start_up_never_replaces_a_file_it_cannot_use() {
    let dir = temp_dir("strict");
    let paths = paths(&dir);
    fs::write(paths.settings_file(), br#"{"role": 1}"#).unwrap();

    let error = open::settings(&paths).start().await.unwrap_err();

    assert_eq!(fs::read(paths.settings_file()).unwrap(), br#"{"role": 1}"#);
    let message = error.to_string();
    assert!(
        message.starts_with(&format!(
            "Koppel cannot use {}: it is not valid JSON for this file",
            paths.settings_file().display()
        )),
        "{message}"
    );
    assert!(message.contains("The file was not changed."));
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn json_null_and_a_failed_check_are_named() {
    let dir = temp_dir("null");
    let paths = paths(&dir);
    fs::write(paths.settings_file(), b"null").unwrap();
    assert_eq!(
        open::settings(&paths).start().await.unwrap_err().reason,
        "it holds no value"
    );

    let secrets = Arc::new(SecretBox::with_key_file(&paths.keys).unwrap());
    let bad_hash =
        br#"{"passwordHash":"pbkdf2-sha256$1000$c2FsdA==$aGFzaA==","securityStamp":"x"}"#;
    fs::write(paths.ui_auth_file(), secrets.protect(bad_hash).unwrap()).unwrap();
    let file = ConfigFile::new(
        paths.ui_auth_file(),
        koppel_config::files::UiCredentials::empty(),
        Options {
            codec: secrets.codec(),
            validate: Some(Arc::new(koppel_config::files::UiCredentials::invalid)),
            start_with_defaults_when_unusable: false,
        },
    );
    assert_eq!(
        file.start().await.unwrap_err().reason,
        "it is not valid (the password hash is not in the format this version writes)"
    );
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_lenient_file_starts_with_defaults_and_moves_the_bad_one_aside_on_the_first_change() {
    let dir = temp_dir("lenient");
    let paths = paths(&dir);
    fs::write(paths.devices_file(), b"not json").unwrap();
    let devices = open::devices(&paths);

    devices.start().await.unwrap();
    assert_eq!(*devices.current(), SharedDevices::empty());
    assert!(
        devices
            .problem()
            .unwrap()
            .starts_with("it is not valid JSON for this file")
    );
    assert_eq!(fs::read(paths.devices_file()).unwrap(), b"not json");

    devices
        .update(|_| SharedDevices {
            bus_ids: Some(vec!["1-1".into()]),
            vid_pids: None,
        })
        .await
        .unwrap();
    assert_eq!(devices.problem(), None);
    let aside: Vec<_> = fs::read_dir(&paths.config)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("devices.json.unreadable-")
        })
        .collect();
    assert_eq!(aside.len(), 1);
    assert_eq!(
        aside[0].file_name().len(),
        "devices.json.unreadable-".len() + 17
    );
    assert_eq!(fs::read(aside[0].path()).unwrap(), b"not json");
    let fresh: SharedDevices = json::from_slice(&fs::read(paths.devices_file()).unwrap()).unwrap();
    assert_eq!(fresh.bus_ids, Some(vec!["1-1".to_string()]));
    devices.stop();
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn stale_temp_files_are_swept_and_a_write_leaves_none() {
    let dir = temp_dir("temps");
    let paths = paths(&dir);
    let stale = paths
        .config
        .join("settings.json.0123456789abcdef0123456789abcdef.tmp");
    let other = paths.config.join("devices.json.0123.tmp");
    fs::write(&stale, b"x").unwrap();
    fs::write(&other, b"x").unwrap();

    let settings = open::settings(&paths);
    settings.start().await.unwrap();
    settings.update(client()).await.unwrap();

    assert!(!stale.exists());
    assert!(
        other.exists(),
        "another file's temp files are its own business"
    );
    let left: Vec<_> = fs::read_dir(&paths.config)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("settings.json.") && n.ends_with(".tmp"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    settings.stop();
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_change_that_changes_nothing_is_not_written_but_still_delivered() {
    let dir = temp_dir("unchanged");
    let paths = paths(&dir);
    let settings = open::settings(&paths);
    settings.start().await.unwrap();
    let delivered = Arc::new(Mutex::new(0));
    let count = delivered.clone();
    settings.subscribe(move |_| {
        *count.lock().unwrap() += 1;
        async {}
    });
    let before = fs::metadata(paths.settings_file())
        .unwrap()
        .modified()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;

    settings.update(|s| s.clone()).await.unwrap();

    assert_eq!(*delivered.lock().unwrap(), 1);
    assert_eq!(
        fs::metadata(paths.settings_file())
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
    settings.stop();
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn deliveries_keep_write_order_and_a_nested_change_queues_behind_the_running_one() {
    let dir = temp_dir("order");
    let paths = paths(&dir);
    let settings = open::settings(&paths);
    settings.start().await.unwrap();
    let log = Arc::new(Mutex::new(Vec::<String>::new()));

    for name in ["first", "second"] {
        let log = log.clone();
        let file = settings.clone();
        settings.subscribe(move |value: Arc<KoppelSettings>| {
            let log = log.clone();
            let file = file.clone();
            async move {
                log.lock()
                    .unwrap()
                    .push(format!("{name} saw {}", value.role.as_str()));
                if name == "first" && value.role == Role::Client {
                    // Returns at once although the delivery it queues runs after this one.
                    file.update(|s| KoppelSettings {
                        role: Role::Both,
                        ..s.clone()
                    })
                    .await
                    .unwrap();
                    log.lock().unwrap().push("nested change written".into());
                }
            }
        });
    }

    settings.update(client()).await.unwrap();
    eventually(|| log.lock().unwrap().len() == 5).await;

    assert_eq!(
        *log.lock().unwrap(),
        [
            "first saw client",
            "nested change written",
            "second saw client",
            "first saw both",
            "second saw both"
        ]
    );
    assert_eq!(settings.current().role, Role::Both);
    settings.stop();
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn concurrent_changes_are_delivered_in_the_order_they_were_written() {
    let dir = temp_dir("concurrent");
    let paths = paths(&dir);
    let devices = open::devices(&paths);
    devices.start().await.unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    devices.subscribe(move |value: Arc<SharedDevices>| {
        log.lock()
            .unwrap()
            .push(value.bus_ids.clone().unwrap_or_default().len());
        async { tokio::time::sleep(Duration::from_millis(2)).await }
    });

    let mut tasks = Vec::new();
    for _ in 0..20 {
        let file = devices.clone();
        tasks.push(tokio::spawn(async move {
            file.update(|d| {
                let mut ids = d.bus_ids.clone().unwrap_or_default();
                ids.push(format!("1-{}", ids.len()));
                SharedDevices {
                    bus_ids: Some(ids),
                    vid_pids: None,
                }
            })
            .await
            .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }

    assert_eq!(*seen.lock().unwrap(), (1..=20).collect::<Vec<_>>());
    devices.stop();
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_handler_that_panics_does_not_stop_the_others() {
    let dir = temp_dir("panic");
    let paths = paths(&dir);
    let settings = open::settings(&paths);
    settings.start().await.unwrap();
    let reached = Arc::new(Mutex::new(false));
    settings.subscribe(|_| async { panic!("a handler that fails") });
    let flag = reached.clone();
    settings.subscribe(move |_| {
        *flag.lock().unwrap() = true;
        async {}
    });

    settings.update(client()).await.unwrap();

    assert!(*reached.lock().unwrap());
    settings.stop();
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn hand_edits_are_picked_up_checked_and_a_bad_one_keeps_the_last_good_value() {
    let dir = temp_dir("edits");
    let paths = paths(&dir);
    let settings = open::settings(&paths);
    settings.start().await.unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    settings.subscribe(move |value: Arc<KoppelSettings>| {
        log.lock().unwrap().push(value.role);
        async {}
    });

    fs::write(
        paths.settings_file(),
        br#"{"Role":"Client","shareOnStartup":true}"#,
    )
    .unwrap();
    eventually(|| *seen.lock().unwrap() == [Role::Client]).await;

    fs::write(paths.settings_file(), br#"{"role":"sideways"}"#).unwrap();
    eventually(|| settings.problem().is_some()).await;
    assert_eq!(settings.current().role, Role::Client);
    assert!(settings.problem().unwrap().contains("sideways"));

    fs::write(paths.settings_file(), br#"{"role":"both"}"#).unwrap();
    eventually(|| settings.problem().is_none() && settings.current().role == Role::Both).await;
    assert_eq!(*seen.lock().unwrap(), [Role::Client, Role::Both]);
    settings.stop();
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_hand_edit_is_loaded_before_the_next_change_so_the_change_keeps_it() {
    let dir = temp_dir("keepedit");
    let paths = paths(&dir);
    let settings = open::settings(&paths);
    settings.start().await.unwrap();
    settings.stop();

    tokio::time::sleep(Duration::from_millis(20)).await;
    fs::write(
        paths.settings_file(),
        br#"{"role":"client","shareOnStartup":false,"notifyLink":false}"#,
    )
    .unwrap();
    let written = settings
        .update(|s| KoppelSettings {
            setup_complete: true,
            ..s.clone()
        })
        .await
        .unwrap();

    assert_eq!(
        (written.role, written.notify_link, written.setup_complete),
        (Role::Client, false, true)
    );
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_change_over_a_bad_hand_edit_replaces_it_with_the_last_good_value_and_the_change() {
    let dir = temp_dir("overbad");
    let paths = paths(&dir);
    let settings = open::settings(&paths);
    settings.start().await.unwrap();
    settings.stop();

    tokio::time::sleep(Duration::from_millis(20)).await;
    fs::write(paths.settings_file(), b"{ broken").unwrap();
    settings.update(client()).await.unwrap();

    let on_disk: KoppelSettings =
        json::from_slice(&fs::read(paths.settings_file()).unwrap()).unwrap();
    assert_eq!(
        on_disk,
        KoppelSettings {
            role: Role::Client,
            ..KoppelSettings::default()
        }
    );
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn an_encrypted_file_round_trips_and_is_not_readable_as_text() {
    let dir = temp_dir("encrypted");
    let paths = paths(&dir);
    let secrets = Arc::new(SecretBox::open(&paths.keys).unwrap());
    let connection = open::connection(&paths, &secrets);
    connection.start().await.unwrap();
    connection
        .update(|c| koppel_config::files::ClientConnection {
            client_secret: Some("very secret".into()),
            ..c.clone()
        })
        .await
        .unwrap();
    connection.stop();

    let bytes = fs::read(paths.connection_file()).unwrap();
    assert!(bytes.starts_with(b"KPSF\x01"));
    assert!(!String::from_utf8_lossy(&bytes).contains("very secret"));
    let again = open::connection(&paths, &secrets);
    again.start().await.unwrap();
    assert_eq!(
        again.current().client_secret.as_deref(),
        Some("very secret")
    );
    again.stop();
    let _ = fs::remove_dir_all(dir);
}
