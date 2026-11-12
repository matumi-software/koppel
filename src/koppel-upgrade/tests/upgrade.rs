//! Koppel 0.1.x and 0.2.0 data folders, upgraded.
//!
//! The fixtures were written by Koppel's own code, never by this crate:
//!
//! - `linux-0.1.2`: the volume of the published `matumisoftware/koppel:0.1.2` image after setting
//!   the password, the settings, two clients, a signing key rotation, a prepared next certificate
//!   and a client connection through its REST API. `plaintext/` is what the C# `SecretBox` decrypts
//!   its secret files to.
//! - `csharp-linux/plain`: Koppel's own stores (`SettingsStore`, `ServerSecretsStore`,
//!   `ConnectionStore`, the `JsonConfigFile`s and `SecretBox`), run on Linux by a throwaway program,
//!   with strings that need escaping, a 0.1.0 shaped `secrets.json` and an RSA certificate in
//!   `extra/`, and their plaintexts.
//! - `csharp-linux/key-certificate`: the same with the key ring protected by a key certificate.
//! - `linux-0.2.0`: what Koppel 0.2.0's code left on disk after it started on the 0.1.2 volume above:
//!   the files converted to its own sealed format (`koppel-secrets.key` in `keys/`, beside the 0.1.x
//!   key ring), the signing key dropped, one certificate written again by 0.2.0's PKCS#12 writer and
//!   the other as .NET made it, and the names and `null` keys 0.2.0 wrote.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use koppel_config::files::{
    AttachedDevices, ClientConnection, KoppelSettings, Role, ServerSecrets, SharedDevices,
    UiCredentials,
};
use koppel_config::{KoppelPaths, SecretBox, ServerCertificate, open};
use koppel_upgrade::{
    Conversion, KOPPEL_PURPOSES, KeyRing, Reshaped, convert_0_1_secrets, pkcs12, reshape_secrets,
    upgrade,
};

const LINUX_012_FINGERPRINT: &str = "21:91:2B:41:C0:2D:38:79:4B:F9:D6:EA:17:EC:B9:33:20:1E:11:19:DD:62:08:E7:C1:62:F7:3E:14:19:BD:AA";
const LINUX_012_NEXT_FINGERPRINT: &str = "71:20:A7:88:2D:EB:1A:65:7E:D0:0A:92:3A:43:65:8B:B1:4E:4E:12:AB:E0:D6:42:5D:54:07:26:22:86:AD:D8";
const BENCH_SECRET: &str = "tBQSIdMdQONeoLmAIJ507c791ZpSVOl5YkJtXkIwL9A";

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(path)
}

fn read(path: &str) -> Vec<u8> {
    fs::read(fixture(path)).unwrap()
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A copy of a fixture data folder to work on.
fn install(name: &str, from: &str) -> (PathBuf, KoppelPaths) {
    let dir = std::env::temp_dir().join(format!("koppel-upgrade-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    copy_dir(&fixture(&format!("{from}/config")), &dir.join("config"));
    copy_dir(&fixture(&format!("{from}/keys")), &dir.join("keys"));
    let paths = KoppelPaths::new(dir.join("config"), dir.join("keys"));
    paths.prepare().unwrap();
    (dir, paths)
}

fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap().flatten() {
        if entry.path().is_dir() {
            files.extend(snapshot(&entry.path()));
        } else {
            files.push((entry.path(), fs::read(entry.path()).unwrap()));
        }
    }
    files.sort();
    files
}

/// What a secret file holds, as text.
fn opened(secrets: &SecretBox, file: &Path) -> String {
    String::from_utf8(secrets.unprotect(&fs::read(file).unwrap()).unwrap()).unwrap()
}

/// The certificate of a started `secrets.json`, by its fingerprint.
fn fingerprints(state: &skakel_server::SkakelState) -> (String, Option<String>) {
    let current = ServerCertificate::from_record(&state.certificate).unwrap();
    let next = state.next_certificate.as_ref().map(|next| {
        ServerCertificate::from_record(&next.certificate)
            .unwrap()
            .fingerprint()
    });
    (current.fingerprint(), next)
}

#[test]
fn every_plain_file_of_each_install_reads_and_is_written_back_the_same() {
    for set in [
        "linux-0.1.2",
        "csharp-linux/plain",
        "csharp-linux/key-certificate",
        "linux-0.2.0",
    ] {
        let settings: KoppelSettings =
            serde_json::from_slice(&read(&format!("{set}/config/settings.json"))).unwrap();
        let devices: SharedDevices =
            serde_json::from_slice(&read(&format!("{set}/config/devices.json"))).unwrap();
        let attached: AttachedDevices =
            serde_json::from_slice(&read(&format!("{set}/config/attached.json"))).unwrap();

        let again: KoppelSettings =
            serde_json::from_slice(&serde_json::to_vec_pretty(&settings).unwrap()).unwrap();
        assert_eq!(again, settings, "{set} settings.json");
        let again: SharedDevices =
            serde_json::from_slice(&serde_json::to_vec_pretty(&devices).unwrap()).unwrap();
        assert_eq!(again, devices, "{set} devices.json");
        let again: AttachedDevices =
            serde_json::from_slice(&serde_json::to_vec_pretty(&attached).unwrap()).unwrap();
        assert_eq!(again, attached, "{set} attached.json");
    }
    let settings: KoppelSettings =
        serde_json::from_slice(&read("linux-0.1.2/config/settings.json")).unwrap();
    assert_eq!(
        settings,
        KoppelSettings {
            role: Role::Both,
            share_on_startup: false,
            setup_complete: true,
            notify_pairing: true,
            notify_link: false,
            notify_attach_failures: true
        }
    );
    let devices: SharedDevices =
        serde_json::from_slice(&read("csharp-linux/plain/config/devices.json")).unwrap();
    assert_eq!(
        devices.vid_pids.get("3-4.1").map(String::as_str),
        Some("0403:6001")
    );
    // 0.1.2 wrote `"vidPids": null` here, and a missing list as null.
    let devices: SharedDevices =
        serde_json::from_slice(&read("linux-0.1.2/config/devices.json")).unwrap();
    assert_eq!(devices, SharedDevices::default());
}

#[test]
fn the_secret_files_decrypt_to_what_csharp_decrypts_them_to() {
    for (set, files) in [
        (
            "linux-0.1.2",
            &[
                "config/secrets.json",
                "config/connection.json",
                "config/ui-auth.json",
            ][..],
        ),
        (
            "csharp-linux/plain",
            &[
                "config/secrets.json",
                "config/connection.json",
                "config/ui-auth.json",
                "extra/secrets-0.1.0.json",
                "extra/hello.bin",
            ][..],
        ),
    ] {
        let ring = KeyRing::load(&fixture(&format!("{set}/keys"))).unwrap();
        for file in files {
            let name = Path::new(file).file_name().unwrap().to_str().unwrap();
            let plain = ring
                .unprotect(&KOPPEL_PURPOSES, &read(&format!("{set}/{file}")))
                .unwrap();
            assert_eq!(
                plain,
                read(&format!("{set}/plaintext/{name}")),
                "{set} {file}"
            );
        }
    }
}

#[test]
fn the_decrypted_0_1_json_reads_into_the_types_except_the_old_secrets_shape() {
    for set in ["linux-0.1.2", "csharp-linux/plain"] {
        let connection: ClientConnection =
            serde_json::from_slice(&read(&format!("{set}/plaintext/connection.json"))).unwrap();
        let again: ClientConnection =
            serde_json::from_slice(&serde_json::to_vec_pretty(&connection).unwrap()).unwrap();
        assert_eq!(again, connection, "{set} connection.json");
        let auth: UiCredentials =
            serde_json::from_slice(&read(&format!("{set}/plaintext/ui-auth.json"))).unwrap();
        let again: UiCredentials =
            serde_json::from_slice(&serde_json::to_vec_pretty(&auth).unwrap()).unwrap();
        assert_eq!(again, auth, "{set} ui-auth.json");

        // The daemon reads the current shape only: PKCS#12 certificates are the upgrade's to read.
        let secrets = serde_json::from_slice::<ServerSecrets>(&read(&format!(
            "{set}/plaintext/secrets.json"
        )));
        assert!(secrets.is_err(), "{set} secrets.json is in the old shape");
    }
}

#[tokio::test]
async fn a_0_1_2_install_is_upgraded_and_keeps_its_certificate_clients_connection_and_password() {
    let (dir, paths) = install("convert", "linux-0.1.2");
    let keys_before = snapshot(&paths.keys);
    let settings_before = fs::read(paths.settings_file()).unwrap();
    let secrets = Arc::new(SecretBox::open(&paths.keys).unwrap());

    let done = upgrade(&paths, &secrets).unwrap();

    assert_eq!(
        done.conversion,
        Conversion::Converted(paths.secret_files().to_vec())
    );
    assert_eq!(
        done.secrets,
        Reshaped::Rewritten {
            dropped: Vec::new()
        }
    );
    for file in paths.secret_files() {
        assert!(
            fs::read(&file).unwrap().starts_with(b"KPSF\x01"),
            "{}",
            file.display()
        );
    }
    // connection.json and ui-auth.json hold the same JSON inside, byte for byte.
    for (file, plain) in [
        (paths.connection_file(), "connection.json"),
        (paths.ui_auth_file(), "ui-auth.json"),
    ] {
        assert_eq!(
            secrets.unprotect(&fs::read(file).unwrap()).unwrap(),
            read(&format!("linux-0.1.2/plaintext/{plain}"))
        );
    }
    // secrets.json holds Skakel's state: the certificates as DER and a key, nothing of PKCS#12 or
    // the token signing keys, and no null keys.
    let text = opened(&secrets, &paths.server_secrets_file());
    assert!(
        text.contains("\"certificate\"") && text.contains("\"privateKey\""),
        "{text}"
    );
    for gone in [
        "certificatePfx",
        "pfx",
        "signingKey",
        "retiredSigningKeys",
        "null",
    ] {
        assert!(!text.contains(gone), "{gone} is still in {text}");
    }
    assert_eq!(fs::read(paths.settings_file()).unwrap(), settings_before);
    let keys_after: Vec<_> = snapshot(&paths.keys)
        .into_iter()
        .filter(|(p, _)| !p.ends_with("koppel-secrets.key"))
        .collect();
    assert_eq!(
        keys_after, keys_before,
        "the 0.1.x key ring is not changed or deleted"
    );

    let server = open::server_secrets(&paths, &secrets);
    server.start().await.unwrap();
    let state = server.current().skakel.clone().unwrap();
    assert_eq!(
        fingerprints(&state),
        (
            LINUX_012_FINGERPRINT.to_owned(),
            Some(LINUX_012_NEXT_FINGERPRINT.to_owned())
        )
    );
    let bench = state
        .clients
        .iter()
        .find(|c| c.id == "client-9419f1711c")
        .unwrap();
    assert_eq!(
        (bench.name.as_str(), bench.secret.as_deref()),
        ("bench", Some(BENCH_SECRET))
    );
    let lab = state
        .clients
        .iter()
        .find(|c| c.id == "client-6730156f0e")
        .unwrap();
    assert_eq!(lab.name, "lab <pc> & 'x' + é 😀");

    let connection = open::connection(&paths, &secrets);
    connection.start().await.unwrap();
    let link = connection.current();
    assert_eq!(
        link.server_fingerprint.as_deref(),
        Some(LINUX_012_FINGERPRINT)
    );
    assert_eq!(
        link.client_secret.as_deref().map(String::as_str),
        Some(BENCH_SECRET)
    );
    assert!(link.is_complete());

    let auth = open::ui_auth(&paths, &secrets);
    auth.start().await.unwrap();
    assert!(auth.current().has_password());
    server.stop();
    connection.stop();
    auth.stop();

    let again = upgrade(&paths, &secrets).unwrap();
    assert_eq!(again.conversion, Conversion::NothingToDo);
    assert_eq!(again.secrets, Reshaped::Nothing);
    let _ = fs::remove_dir_all(dir);
}

/// What Koppel 0.2.0 left on disk: the files are sealed already, so only `secrets.json` has to be
/// rewritten, and the daemon cannot read it before that.
#[tokio::test]
async fn a_0_2_0_install_is_read_and_rewritten_with_everything_it_had() {
    let (dir, paths) = install("from-0-2-0", "linux-0.2.0");
    let secrets = Arc::new(SecretBox::open(&paths.keys).unwrap());
    let before = fs::read(paths.server_secrets_file()).unwrap();
    let old_text = opened(&secrets, &paths.server_secrets_file());
    assert!(
        old_text.contains("certificatePfx"),
        "the fixture is in 0.2.0's shape"
    );
    assert!(old_text.contains("\"version\""), "0.2.0 wrote the version");
    assert!(
        serde_json::from_str::<ServerSecrets>(&old_text).is_err(),
        "the current type does not read 0.2.0's shape"
    );
    let connection_before = fs::read(paths.connection_file()).unwrap();
    let ui_auth_before = fs::read(paths.ui_auth_file()).unwrap();
    let settings_before = fs::read(paths.settings_file()).unwrap();

    let done = upgrade(&paths, &secrets).unwrap();

    assert_eq!(done.conversion, Conversion::NothingToDo);
    assert_eq!(
        done.secrets,
        Reshaped::Rewritten {
            dropped: Vec::new()
        }
    );
    assert_ne!(
        fs::read(paths.server_secrets_file()).unwrap(),
        before,
        "rewritten"
    );
    let text = opened(&secrets, &paths.server_secrets_file());
    for gone in ["certificatePfx", "pfx", "signingKey", "null"] {
        assert!(!text.contains(gone), "{gone} is still in {text}");
    }
    // The other files are read as they are.
    assert_eq!(
        fs::read(paths.connection_file()).unwrap(),
        connection_before
    );
    assert_eq!(fs::read(paths.ui_auth_file()).unwrap(), ui_auth_before);
    assert_eq!(fs::read(paths.settings_file()).unwrap(), settings_before);

    let server = open::server_secrets(&paths, &secrets);
    server.start().await.unwrap();
    let state = server.current().skakel.clone().unwrap();
    assert_eq!(
        fingerprints(&state),
        (
            LINUX_012_FINGERPRINT.to_owned(),
            Some(LINUX_012_NEXT_FINGERPRINT.to_owned())
        ),
        "the certificates keep their fingerprints"
    );
    let certificate = ServerCertificate::from_record(&state.certificate).unwrap();
    assert_eq!(
        certificate.certified_key().unwrap().key.algorithm(),
        rustls::SignatureAlgorithm::ECDSA
    );
    let ids: Vec<&str> = state.clients.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["client-9419f1711c", "client-6730156f0e"]);
    let bench = &state.clients[0];
    assert_eq!(bench.secret.as_deref(), Some(BENCH_SECRET));
    assert!(bench.secret_hash.starts_with("sha256$"));
    assert_eq!(state.clients[1].name, "lab <pc> & 'x' + é 😀");
    let next = state.next_certificate.as_ref().unwrap();
    let old: serde_json::Value = serde_json::from_str(&old_text).unwrap();
    assert_eq!(
        serde_json::json!(next.received_by),
        old["skakel"]["nextCertificate"]["receivedBy"],
        "the clients that stored the next certificate are kept"
    );

    let connection = open::connection(&paths, &secrets);
    connection.start().await.unwrap();
    assert!(connection.current().is_complete());
    assert_eq!(
        connection
            .current()
            .client_secret
            .as_deref()
            .map(String::as_str),
        Some(BENCH_SECRET)
    );
    let auth = open::ui_auth(&paths, &secrets);
    auth.start().await.unwrap();
    assert!(auth.current().has_password());
    let settings = open::settings(&paths);
    settings.start().await.unwrap();
    assert_eq!(settings.current().role, Role::Both);
    server.stop();
    connection.stop();
    auth.stop();
    settings.stop();

    // A second start finds nothing left to do, and the key ring is where it was.
    let again = upgrade(&paths, &secrets).unwrap();
    assert_eq!(again.secrets, Reshaped::Nothing);
    assert!(
        fs::read_dir(&paths.keys)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("key-"))
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_failed_conversion_leaves_the_0_1_files_as_they_were() {
    // A key ring whose master key is not the one that encrypted the files.
    let (dir, paths) = install("wrongkey", "linux-0.1.2");
    let key = fs::read_dir(&paths.keys)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("key-"))
        .unwrap()
        .path();
    let text = fs::read_to_string(&key).unwrap();
    let start = text.find("<value>").unwrap() + 7;
    let mut wrong = text.clone();
    wrong.replace_range(
        start..start + 4,
        if &text[start..start + 4] == "AAAA" {
            "BBBB"
        } else {
            "AAAA"
        },
    );
    fs::write(&key, wrong).unwrap();
    let before = snapshot(&dir);
    let secrets = SecretBox::with_key_file(&paths.keys).unwrap();

    let error = upgrade(&paths, &secrets).unwrap_err();

    assert!(error.to_string().contains("MAC does not match"), "{error}");
    let after: Vec<_> = snapshot(&dir)
        .into_iter()
        .filter(|(p, _)| !p.ends_with("koppel-secrets.key"))
        .collect();
    assert_eq!(after, before);

    // Start-up then refuses the file and names it, as 0.1.x does for a lost key ring.
    let refused = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(open::server_secrets(&paths, &Arc::new(secrets)).start())
        .unwrap_err();
    assert!(refused.reason.contains("0.1.x's format"), "{refused}");
    let _ = fs::remove_dir_all(dir);

    // No key ring at all.
    let (dir, paths) = install("nokeys", "linux-0.1.2");
    for entry in fs::read_dir(&paths.keys).unwrap().flatten() {
        fs::remove_file(entry.path()).unwrap();
    }
    let before = snapshot(&paths.config);
    let error = upgrade(&paths, &SecretBox::with_key_file(&paths.keys).unwrap()).unwrap_err();
    assert!(error.to_string().contains("no key-*.xml"), "{error}");
    assert_eq!(snapshot(&paths.config), before);
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_key_ring_protected_with_a_key_certificate_takes_the_pair_again_path() {
    let (dir, paths) = install("keycert", "csharp-linux/key-certificate");
    let originals: Vec<_> = paths
        .secret_files()
        .iter()
        .map(|f| fs::read(f).unwrap())
        .collect();
    let secrets = Arc::new(SecretBox::open(&paths.keys).unwrap());

    let done = upgrade(&paths, &secrets).unwrap();

    let Conversion::PairAgain(moved) = done.conversion else {
        panic!("expected the pair-again path");
    };
    assert_eq!(done.secrets, Reshaped::Nothing);
    assert_eq!(moved.len(), 3);
    for ((from, to), original) in moved.iter().zip(&originals) {
        assert!(!from.exists());
        assert!(to.to_string_lossy().contains(".0.1-key-certificate-"));
        assert_eq!(&fs::read(to).unwrap(), original, "kept unchanged");
    }
    // Koppel then starts as a new install.
    let server = open::server_secrets(&paths, &secrets);
    server.start().await.unwrap();
    assert_eq!(server.current().skakel, None);
    let auth = open::ui_auth(&paths, &secrets);
    auth.start().await.unwrap();
    assert!(!auth.current().has_password());
    server.stop();
    auth.stop();
    let settings: KoppelSettings =
        serde_json::from_slice(&fs::read(paths.settings_file()).unwrap()).unwrap();
    assert_eq!(settings.role, Role::Both, "plain files stay");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_pkcs12_files_csharp_made_are_read() {
    let secrets: serde_json::Value =
        serde_json::from_slice(&read("linux-0.1.2/plaintext/secrets.json")).unwrap();
    let pfx = secrets["skakel"]["certificatePfx"].as_str().unwrap();
    let fingerprints_file =
        String::from_utf8(read("csharp-linux/plain/extra/fingerprints.txt")).unwrap();
    assert!(!fingerprints_file.is_empty());
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(pfx)
        .unwrap();
    let contents = pkcs12::parse(&bytes).unwrap();
    let certificate = ServerCertificate {
        certificate: contents.certificates[0].der.clone(),
        private_key: contents.keys[0].der.clone(),
    };
    assert_eq!(certificate.fingerprint(), LINUX_012_FINGERPRINT);
    let certified = certificate.certified_key().unwrap();
    assert_eq!(certified.key.algorithm(), rustls::SignatureAlgorithm::ECDSA);

    // No 0.1.x made an RSA certificate, but one reads all the same.
    let rsa = String::from_utf8(read("csharp-linux/plain/extra/rsa.pfx.b64")).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(rsa.trim())
        .unwrap();
    let contents = pkcs12::parse(&bytes).unwrap();
    let rsa = ServerCertificate {
        certificate: contents.certificates[0].der.clone(),
        private_key: contents.keys[0].der.clone(),
    };
    assert_eq!(
        rsa.certified_key().unwrap().key.algorithm(),
        rustls::SignatureAlgorithm::RSA
    );
}

/// A data folder holding only the plaintext secrets.json of another fixture, sealed with a key file.
fn sealed_secrets_install(name: &str, plaintext: &[u8]) -> (PathBuf, KoppelPaths, SecretBox) {
    let dir = std::env::temp_dir().join(format!("koppel-upgrade-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let paths = KoppelPaths::new(dir.join("config"), dir.join("keys"));
    paths.prepare().unwrap();
    let secrets = SecretBox::with_key_file(&paths.keys).unwrap();
    fs::write(
        paths.server_secrets_file(),
        secrets.protect(plaintext).unwrap(),
    )
    .unwrap();
    (dir, paths, secrets)
}

#[tokio::test]
async fn a_0_1_0_secrets_file_carries_over_as_0_1_1_carried_it() {
    let (dir, paths, secrets) = sealed_secrets_install(
        "carry",
        &read("csharp-linux/plain/plaintext/secrets-0.1.0.json"),
    );

    let done = reshape_secrets(&paths, &secrets).unwrap();

    assert_eq!(
        done,
        Reshaped::Rewritten {
            dropped: vec![(
                client_name("client-1111111111"),
                "client-1111111111".to_owned()
            )]
        }
    );
    let secrets = Arc::new(secrets);
    let file = open::server_secrets(&paths, &secrets);
    file.start().await.unwrap();
    let state = file.current().skakel.clone().unwrap();
    file.stop();
    assert_eq!(
        state
            .clients
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["client-2222222222"]
    );
    assert_eq!(state.clients[0].generation, 3);
    let fingerprints_file =
        String::from_utf8(read("csharp-linux/plain/extra/fingerprints.txt")).unwrap();
    let (current, next) = fingerprints(&state);
    assert_eq!(current, fingerprints_file.lines().next().unwrap());
    assert_eq!(next.as_deref(), fingerprints_file.lines().nth(1));
    assert_eq!(
        state.next_certificate.unwrap().received_by,
        ["client-2222222222"]
    );
    let _ = fs::remove_dir_all(dir);
}

/// The name 0.1.0's file gave the client with this id.
fn client_name(id: &str) -> String {
    let legacy: serde_json::Value =
        serde_json::from_slice(&read("csharp-linux/plain/plaintext/secrets-0.1.0.json")).unwrap();
    legacy["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap()["name"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn a_0_1_0_secrets_file_that_cannot_be_carried_over_is_kept_aside_and_the_start_is_fresh() {
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&read("csharp-linux/plain/plaintext/secrets-0.1.0.json")).unwrap();
    legacy["certificatePfx"] = "bm90IGEgY2VydGlmaWNhdGU=".into();
    let (dir, paths, secrets) =
        sealed_secrets_install("aside", &serde_json::to_vec(&legacy).unwrap());
    let before = fs::read(paths.server_secrets_file()).unwrap();

    let Reshaped::StartedFresh(aside) = reshape_secrets(&paths, &secrets).unwrap() else {
        panic!("expected a fresh start");
    };

    assert!(!paths.server_secrets_file().exists());
    assert!(aside.to_string_lossy().contains("secrets.json.0.1.0-"));
    assert_eq!(fs::read(aside).unwrap(), before, "kept unchanged");
    let _ = fs::remove_dir_all(dir);
}

/// A data folder the C# code wrote on this Windows machine, with its key ring protected by machine
/// DPAPI. Such blobs only open on the machine that made them, so they are not committed: point
/// `KOPPEL_WINDOWS_0_1_FIXTURE` at one (the folder holding `config`, `keys` and `plaintext`) to run it.
#[cfg(windows)]
#[tokio::test]
async fn a_windows_0_1_install_with_a_dpapi_key_ring_is_converted() {
    let Some(source) = std::env::var_os("KOPPEL_WINDOWS_0_1_FIXTURE").map(PathBuf::from) else {
        eprintln!("skipped: KOPPEL_WINDOWS_0_1_FIXTURE is not set");
        return;
    };
    let dir = std::env::temp_dir().join(format!("koppel-upgrade-windows-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    copy_dir(&source.join("config"), &dir.join("config"));
    copy_dir(&source.join("keys"), &dir.join("keys"));
    let paths = KoppelPaths::new(dir.join("config"), dir.join("keys"));
    paths.prepare().unwrap();
    let key = fs::read_dir(&paths.keys)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("key-"))
        .unwrap();
    assert!(
        fs::read_to_string(key.path())
            .unwrap()
            .contains("DpapiXmlDecryptor"),
        "the fixture's key ring uses DPAPI"
    );
    let secrets = Arc::new(SecretBox::open(&paths.keys).unwrap());

    assert_eq!(
        convert_0_1_secrets(&paths, &secrets).unwrap(),
        Conversion::Converted(paths.secret_files().to_vec())
    );

    for (file, plain) in
        paths
            .secret_files()
            .iter()
            .zip(["secrets.json", "connection.json", "ui-auth.json"])
    {
        let bytes = fs::read(file).unwrap();
        assert!(bytes.starts_with(b"KPSF\x01\x01"), "machine DPAPI");
        assert_eq!(
            secrets.unprotect(&bytes).unwrap(),
            fs::read(source.join("plaintext").join(plain)).unwrap()
        );
    }
    reshape_secrets(&paths, &secrets).unwrap();
    let server = open::server_secrets(&paths, &secrets);
    server.start().await.unwrap();
    let expected = fs::read_to_string(source.join("extra/fingerprints.txt")).unwrap();
    assert_eq!(
        fingerprints(&server.current().skakel.clone().unwrap()).0,
        expected.lines().next().unwrap()
    );
    server.stop();
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_payload_key_is_found_by_its_id_among_other_and_unreadable_key_files() {
    let (dir, paths) = install("morekeys", "linux-0.1.2");
    let original = fs::read_dir(&paths.keys)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("key-"))
        .unwrap()
        .path();
    let text = fs::read_to_string(&original).unwrap();
    let other = text
        .replace(
            "9130dec0-c26c-4c1a-9248-6f9905a5d9d7",
            "00000000-1111-2222-3333-444444444444",
        )
        .replace("<value>Z0MX", "<value>AAAA");
    fs::write(
        paths
            .keys
            .join("key-00000000-1111-2222-3333-444444444444.xml"),
        other,
    )
    .unwrap();
    fs::write(paths.keys.join("key-broken.xml"), "<key").unwrap();

    let ring = KeyRing::load(&paths.keys).unwrap();
    let plain = ring
        .unprotect(
            &KOPPEL_PURPOSES,
            &fs::read(paths.connection_file()).unwrap(),
        )
        .unwrap();
    assert_eq!(plain, read("linux-0.1.2/plaintext/connection.json"));

    fs::remove_file(original).unwrap();
    let missing = KeyRing::load(&paths.keys)
        .unwrap()
        .unprotect(
            &KOPPEL_PURPOSES,
            &fs::read(paths.connection_file()).unwrap(),
        )
        .unwrap_err();
    let message = missing.to_string();
    assert!(
        message.contains("9130dec0-c26c-4c1a-9248-6f9905a5d9d7")
            && message.contains("key-broken.xml"),
        "{message}"
    );
    let _ = fs::remove_dir_all(dir);
}
