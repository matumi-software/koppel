//! What Koppel keeps on disk, seen from outside: what survives a restart, a hand edit of
//! `settings.json`, the refusal to start without the keys, and who may read the folders.

use std::path::Path;
use std::time::Duration;

use crate::support::shape;
use crate::support::{Instance, Main, Options, Pair, activity_with, eventually, json, password};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(30);

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{} cannot be read: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

fn write_json(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_string_pretty(value).expect("JSON")).expect("a write");
}

/// Everything in a folder as (relative path, bytes), to see that nothing changed.
fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, into: &mut Vec<(String, Vec<u8>)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, into);
            } else {
                let name = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                into.push((name, std::fs::read(&path).unwrap_or_default()));
            }
        }
    }
    let mut files = Vec::new();
    walk(root, root, &mut files);
    files.sort();
    files
}

#[tokio::test]
async fn settings_clients_the_certificate_and_the_password_survive_a_restart_and_the_sessions_do_not()
 {
    let mut main = Main::get().await;
    main.admin
        .patch(
            "/api/v1/settings",
            json!({ "role": "both", "shareOnStartup": false, "notifyLink": false }),
        )
        .await
        .expect(202);
    let created = main
        .admin
        .post("/api/v1/server/clients", json!({ "name": "survivor" }))
        .await
        .expect(201)
        .json();
    let id = created["clientId"].as_str().expect("an id").to_owned();
    let secret = created["clientSecret"]
        .as_str()
        .expect("a secret")
        .to_owned();
    let settings = main.admin.get("/api/v1/settings").await.expect(200).json();
    let identity = main
        .admin
        .get("/api/v1/server/identity")
        .await
        .expect(200)
        .json();
    let clients = main
        .admin
        .get("/api/v1/server/clients")
        .await
        .expect(200)
        .json();

    main.instance.restart().await;

    // Sessions are in memory, so a restart signs everyone out; the password is on disk, so signing
    // in again works.
    main.admin.get("/api/v1/settings").await.expect(401);
    let signed_in = main.target.sign_in().await;
    let settings_after = signed_in.get("/api/v1/settings").await.expect(200).json();
    assert_eq!(settings_after, settings);
    // The pinned certificate is the one the restarted instance presents.
    assert_eq!(
        main.target.current_certificate().await,
        identity["fingerprint"].as_str().unwrap_or_default()
    );
    assert_eq!(
        signed_in
            .get("/api/v1/server/identity")
            .await
            .expect(200)
            .json(),
        identity
    );
    assert_eq!(
        signed_in
            .get("/api/v1/server/clients")
            .await
            .expect(200)
            .json(),
        clients
    );
    assert_eq!(
        signed_in
            .get(&format!("/api/v1/server/clients/{id}/secret"))
            .await
            .expect(200)
            .json()["clientSecret"],
        json!(secret)
    );
    let state = main
        .target
        .anonymous()
        .get("/api/v1/auth/state")
        .await
        .expect(200)
        .json();
    assert_eq!(state["passwordSet"], json!(true));
    assert_eq!(state["setupOpen"], json!(false));
    // The activity log is memory only; what the restart must not do is make a second certificate.
    assert!(
        activity_with(&signed_in, "certificateSwitched")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn a_clients_connection_survives_a_restart_without_showing_its_secret() {
    let mut pair = Pair::get().await;
    let (_, secret) = pair.connect().await;
    let before = pair
        .client_admin
        .get("/api/v1/client/connection")
        .await
        .expect(200)
        .json();
    assert_eq!(before["hasSecret"], json!(true));

    pair.client_instance.restart().await;

    let client = pair.client.sign_in().await;
    let after = client
        .get("/api/v1/client/connection")
        .await
        .expect(200)
        .json();
    assert_eq!(after, before);
    assert!(!after.to_string().contains(&secret), "never the secret");
    // The stored credentials still link the client to the server.
    let linked = eventually("the restarted client to link", WAIT, || async {
        let devices = client
            .get("/api/v1/client/devices")
            .await
            .expect(200)
            .json();
        (devices["link"] == json!("connected")).then_some(devices)
    })
    .await;
    shape::fields(&linked, shape::CLIENT_DEVICES, "GET /api/v1/client/devices");
}

#[tokio::test]
async fn a_hand_edit_of_settings_json_is_applied_pushed_and_a_bad_one_is_kept_out() {
    let main = Main::get().await;
    let path = main.instance.config_dir().join("settings.json");
    let mut events = main.admin.events().await;
    assert_eq!(events.status, 200);

    // A valid edit while Koppel runs.
    let mut edited = read_json(&path);
    edited["shareOnStartup"] = json!(false);
    edited["notifyLink"] = json!(false);
    write_json(&path, &edited);
    let applied = eventually("the hand edit to be applied", WAIT, || async {
        let settings = main.admin.get("/api/v1/settings").await.expect(200).json();
        (settings["shareOnStartup"] == json!(false)).then_some(settings)
    })
    .await;
    assert_eq!(applied["notifyLink"], json!(false));
    let pushed = events
        .wait_for("SettingsChanged", WAIT, |s| {
            s["shareOnStartup"] == json!(false)
        })
        .await;
    assert_eq!(pushed, applied);

    // A change from the UI does not undo it.
    main.admin
        .patch("/api/v1/settings", json!({ "notifyPairing": false }))
        .await
        .expect(202);
    let settings = main.admin.get("/api/v1/settings").await.expect(200).json();
    assert_eq!(settings["shareOnStartup"], json!(false));
    assert_eq!(settings["notifyPairing"], json!(false));

    // An edit that is not valid is ignored, and the checks say so.
    std::fs::write(&path, "{ not json").expect("a write");
    let problem = eventually("the bad edit to be reported", WAIT, || async {
        let checks = main
            .admin
            .get("/api/v1/system/checks")
            .await
            .expect(200)
            .json();
        checks
            .as_array()
            .and_then(|list| list.iter().find(|c| c["id"] == json!("settings")))
            .cloned()
    })
    .await;
    assert_eq!(problem["ok"], json!(false), "{problem}");
    assert!(
        !problem["detail"].as_str().unwrap_or_default().is_empty(),
        "a reason: {problem}"
    );
    let kept = main.admin.get("/api/v1/settings").await.expect(200).json();
    assert_eq!(kept, settings, "the last good value stays");

    // Fixing it clears the report.
    let mut fixed = settings.clone();
    fixed["role"] = json!("server");
    fixed["version"] = json!(1);
    write_json(&path, &fixed);
    eventually("the report to clear", WAIT, || async {
        let checks = main
            .admin
            .get("/api/v1/system/checks")
            .await
            .expect(200)
            .json();
        checks
            .as_array()
            .is_some_and(|list| !list.iter().any(|c| c["id"] == json!("settings")))
            .then_some(())
    })
    .await;
}

/// Windows seals with DPAPI, not a key file, so moving the keys folder aside loses nothing there.
#[cfg(unix)]
#[tokio::test]
async fn without_its_keys_koppel_says_which_file_it_cannot_read_and_changes_nothing() {
    let mut main = Main::get().await;
    main.admin
        .patch("/api/v1/settings", json!({ "shareOnStartup": false }))
        .await
        .expect(202);
    main.instance.stop();

    let keys = main.instance.keys_dir();
    let kept_keys = main.instance.root().join("keys-backup");
    std::fs::rename(&keys, &kept_keys).expect("the keys folder moves aside");
    std::fs::create_dir(&keys).expect("an empty keys folder");
    let config = main.instance.config_dir();
    let before = snapshot(&config);

    let failed = main
        .instance
        .spawn()
        .await
        .expect_err("Koppel starts without the key ring that sealed its files");
    assert_ne!(failed.code, Some(0));
    assert!(
        failed.log.contains("Koppel cannot use")
            && ["secrets.json", "connection.json", "ui-auth.json"]
                .iter()
                .any(|file| failed.log.contains(file)),
        "the log names the file: {}",
        failed.log
    );
    assert!(
        failed.log.contains("restore the keys directory"),
        "the log says what to do: {}",
        failed.log
    );
    assert_eq!(snapshot(&config), before, "no file was changed or replaced");

    // The keys back where they were, Koppel starts and knows its password and its settings again.
    std::fs::remove_dir_all(&keys).expect("the empty folder goes");
    std::fs::rename(&kept_keys, &keys).expect("the keys come back");
    main.instance
        .spawn()
        .await
        .unwrap_or_else(|f| panic!("Koppel did not start with its keys back:\n{}", f.log));
    let admin = main.target.sign_in().await;
    let settings = admin.get("/api/v1/settings").await.expect(200).json();
    assert_eq!(settings["shareOnStartup"], json!(false));
}

#[tokio::test]
async fn a_new_install_makes_its_files_and_a_new_password_is_set_within_the_window() {
    let instance = Instance::start(Options::default()).await;
    let state = instance
        .target
        .anonymous()
        .get("/api/v1/auth/state")
        .await
        .expect(200)
        .json();
    assert_eq!(state["passwordSet"], json!(false));
    for file in ["settings.json", "secrets.json"] {
        assert!(
            instance.config_dir().join(file).is_file(),
            "{file} is made at the first start"
        );
    }
    let settings = read_json(&instance.config_dir().join("settings.json"));
    assert!(settings["role"].is_string(), "{settings}");
    // The password goes to ui-auth.json, sealed: the password is not in any file.
    instance.target.sign_in().await;
    let auth = instance.config_dir().join("ui-auth.json");
    assert!(auth.is_file());
    for (name, bytes) in snapshot(instance.root()) {
        assert!(
            !String::from_utf8_lossy(&bytes).contains(&password()),
            "the password is in {name}"
        );
    }
}

#[cfg(unix)]
mod permissions {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::snapshot;
    use crate::support::{Instance, Main, Options, json};

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .permissions()
            .mode()
            & 0o777
    }

    #[tokio::test]
    async fn the_folders_are_for_the_owner_only_and_so_are_the_files_in_them() {
        let main = Main::get().await;
        // Change something so every file has been written at least once while running.
        main.admin
            .patch("/api/v1/settings", json!({ "shareOnStartup": false }))
            .await
            .expect(202);
        let config = main.instance.config_dir();
        let keys = main.instance.keys_dir();
        assert_eq!(mode(&config), 0o700, "the config folder");
        assert_eq!(mode(&keys), 0o700, "the keys folder");
        for folder in [&config, &keys] {
            for (name, _) in snapshot(folder) {
                let file = folder.join(&name);
                assert_eq!(
                    mode(&file) & 0o077,
                    0,
                    "{} is open to others: {:o}",
                    file.display(),
                    mode(&file)
                );
            }
        }
        for secret in ["secrets.json", "ui-auth.json"] {
            let file = config.join(secret);
            let bytes = std::fs::read(&file).expect("a secret file");
            assert!(bytes.starts_with(b"KPSF"), "{secret} is sealed");
        }
    }

    #[tokio::test]
    async fn folders_that_were_open_to_others_are_closed_at_the_start() {
        let mut instance = Instance::unstarted(Options::default());
        for folder in [instance.config_dir(), instance.keys_dir()] {
            std::fs::create_dir_all(&folder).expect("a folder");
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o777))
                .expect("a mode");
        }
        instance
            .spawn()
            .await
            .unwrap_or_else(|f| panic!("{}", f.log));
        assert_eq!(mode(&instance.config_dir()), 0o700);
        assert_eq!(mode(&instance.keys_dir()), 0o700);
    }

    #[tokio::test]
    async fn a_config_folder_that_is_a_link_is_refused() {
        let mut instance = Instance::unstarted(Options::default());
        let real = instance.root().join("real-config");
        std::fs::create_dir(&real).expect("a folder");
        std::os::unix::fs::symlink(&real, instance.config_dir()).expect("a link");
        let failed = instance
            .spawn()
            .await
            .expect_err("Koppel refuses a config folder that is a link");
        assert_ne!(failed.code, Some(0));
        assert!(
            failed.log.contains("link"),
            "the log says why: {}",
            failed.log
        );
        assert!(
            snapshot(&real).is_empty(),
            "nothing was written through the link"
        );
    }
}

/// 0.2.0 signed its session cookies with `keys/ui-session.key`. Sessions are in memory now, so the
/// file is deleted at the start, and nothing writes it again.
#[tokio::test]
async fn the_session_key_an_older_version_wrote_is_deleted_at_start() {
    let mut main = Main::get().await;
    let key = main.instance.keys_dir().join("ui-session.key");
    assert!(
        !key.exists(),
        "this version never writes it, and signing in has just happened"
    );

    std::fs::write(&key, b"an old key").expect("a write");
    main.instance.restart().await;

    assert!(!key.exists(), "deleted at the first start");
    assert!(
        main.instance.log().contains("ui-session.key"),
        "the log says so"
    );
    // The password is kept, and signing in works without the key.
    main.target
        .sign_in()
        .await
        .get("/api/v1/settings")
        .await
        .expect(200);
    assert!(!key.exists());
}
