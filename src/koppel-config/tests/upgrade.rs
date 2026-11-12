//! Koppel 0.1.x data folders, carried over.
//!
//! The fixtures were written by Koppel's C# code, never by this crate:
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

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use koppel_config::files::{
    AttachedDevices, ClientConnection, KoppelSettings, Role, ServerSecrets, SharedDevices,
    UiCredentials,
};
use koppel_config::{
    Conversion, KOPPEL_PURPOSES, KeyRing, KoppelPaths, SecretBox, ServerCertificate,
    convert_0_1_secrets, json, open,
};
use time::OffsetDateTime;

const LINUX_012_FINGERPRINT: &str = "21:91:2B:41:C0:2D:38:79:4B:F9:D6:EA:17:EC:B9:33:20:1E:11:19:DD:62:08:E7:C1:62:F7:3E:14:19:BD:AA";
const LINUX_012_NEXT_FINGERPRINT: &str = "71:20:A7:88:2D:EB:1A:65:7E:D0:0A:92:3A:43:65:8B:B1:4E:4E:12:AB:E0:D6:42:5D:54:07:26:22:86:AD:D8";

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(path)
}

fn read(path: &str) -> Vec<u8> {
    fs::read(fixture(path)).unwrap()
}

/// The fixture's text with this platform's line ending, as System.Text.Json would write it here.
fn here(bytes: &[u8]) -> String {
    let text = String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace("\r\n", "\n");
    if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text
    }
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

fn round_trip<T: serde::Serialize + serde::de::DeserializeOwned>(bytes: &[u8]) -> String {
    String::from_utf8(json::to_vec(&json::from_slice::<T>(bytes).unwrap()).unwrap()).unwrap()
}

#[test]
fn every_plain_file_reads_and_writes_back_byte_for_byte() {
    for set in [
        "linux-0.1.2",
        "csharp-linux/plain",
        "csharp-linux/key-certificate",
    ] {
        let settings = read(&format!("{set}/config/settings.json"));
        assert_eq!(
            round_trip::<KoppelSettings>(&settings),
            here(&settings),
            "{set} settings.json"
        );
        let devices = read(&format!("{set}/config/devices.json"));
        assert_eq!(
            round_trip::<SharedDevices>(&devices),
            here(&devices),
            "{set} devices.json"
        );
        let attached = read(&format!("{set}/config/attached.json"));
        assert_eq!(
            round_trip::<AttachedDevices>(&attached),
            here(&attached),
            "{set} attached.json"
        );
    }
    let settings: KoppelSettings =
        json::from_slice(&read("linux-0.1.2/config/settings.json")).unwrap();
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
        json::from_slice(&read("csharp-linux/plain/config/devices.json")).unwrap();
    assert_eq!(
        devices.vid_pids.unwrap().get("3-4.1").map(String::as_str),
        Some("0403:6001")
    );
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
fn the_decrypted_json_reads_into_the_types_and_writes_back_byte_for_byte() {
    for set in ["linux-0.1.2", "csharp-linux/plain"] {
        let secrets = read(&format!("{set}/plaintext/secrets.json"));
        assert_eq!(
            round_trip::<ServerSecrets>(&secrets),
            here(&secrets),
            "{set} secrets.json"
        );
        let connection = read(&format!("{set}/plaintext/connection.json"));
        assert_eq!(
            round_trip::<ClientConnection>(&connection),
            here(&connection),
            "{set} connection.json"
        );
        let auth = read(&format!("{set}/plaintext/ui-auth.json"));
        assert_eq!(
            round_trip::<UiCredentials>(&auth),
            here(&auth),
            "{set} ui-auth.json"
        );
    }
    let legacy = read("csharp-linux/plain/plaintext/secrets-0.1.0.json");
    assert_eq!(round_trip::<ServerSecrets>(&legacy), here(&legacy));
}

#[tokio::test]
async fn a_0_1_2_install_is_converted_and_keeps_its_certificate_clients_connection_and_password() {
    let (dir, paths) = install("convert", "linux-0.1.2");
    let keys_before = snapshot(&paths.keys);
    let settings_before = fs::read(paths.settings_file()).unwrap();
    let secrets = Arc::new(SecretBox::open(&paths.keys).unwrap());

    let conversion = convert_0_1_secrets(&paths, &secrets).unwrap();

    assert_eq!(
        conversion,
        Conversion::Converted(paths.secret_files().to_vec())
    );
    for file in paths.secret_files() {
        assert!(
            fs::read(&file).unwrap().starts_with(b"KPSF\x01"),
            "{}",
            file.display()
        );
    }
    // The JSON inside is the same, byte for byte.
    for (file, plain) in
        paths
            .secret_files()
            .iter()
            .zip(["secrets.json", "connection.json", "ui-auth.json"])
    {
        assert_eq!(
            secrets.unprotect(&fs::read(file).unwrap()).unwrap(),
            read(&format!("linux-0.1.2/plaintext/{plain}"))
        );
    }
    assert_eq!(fs::read(paths.settings_file()).unwrap(), settings_before);
    let keys_after: Vec<_> = snapshot(&paths.keys)
        .into_iter()
        .filter(|(p, _)| !p.ends_with("koppel-secrets.key"))
        .collect();
    assert_eq!(keys_after, keys_before, "the 0.1.x key ring is not changed");

    let server = open::server_secrets(&paths, &secrets);
    server.start().await.unwrap();
    let state = server.current().skakel.clone().unwrap();
    assert_eq!(
        state.certificate().unwrap().fingerprint(),
        LINUX_012_FINGERPRINT
    );
    assert_eq!(
        state.next_certificate().unwrap().unwrap().fingerprint(),
        LINUX_012_NEXT_FINGERPRINT
    );
    assert_eq!(
        state.signing_key.as_ref().unwrap().key_id,
        "skakel-c160b88c37"
    );
    assert_eq!(state.retired_signing_keys.len(), 1);
    let bench = state
        .clients
        .iter()
        .find(|c| c.id == "client-9419f1711c")
        .unwrap();
    assert_eq!(
        (
            bench.name.as_str(),
            bench.secret.as_deref().map(String::as_str)
        ),
        ("bench", Some("tBQSIdMdQONeoLmAIJ507c791ZpSVOl5YkJtXkIwL9A"))
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
        Some("tBQSIdMdQONeoLmAIJ507c791ZpSVOl5YkJtXkIwL9A")
    );
    assert!(link.is_complete());

    let auth = open::ui_auth(&paths, &secrets);
    auth.start().await.unwrap();
    assert!(auth.current().has_password());
    assert_eq!(
        auth.current().security_stamp.as_deref(),
        Some("15369d15e204430db3e39bc88e2431ba")
    );

    for file in [
        server.path().to_path_buf(),
        connection.path().to_path_buf(),
        auth.path().to_path_buf(),
    ] {
        assert!(file.exists());
    }
    server.stop();
    connection.stop();
    auth.stop();

    assert_eq!(
        convert_0_1_secrets(&paths, &secrets).unwrap(),
        Conversion::NothingToDo
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

    let error = convert_0_1_secrets(&paths, &secrets).unwrap_err();

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
    let error =
        convert_0_1_secrets(&paths, &SecretBox::with_key_file(&paths.keys).unwrap()).unwrap_err();
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

    let Conversion::PairAgain(moved) = convert_0_1_secrets(&paths, &secrets).unwrap() else {
        panic!("expected the pair-again path");
    };

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
        json::from_slice(&fs::read(paths.settings_file()).unwrap()).unwrap();
    assert_eq!(settings.role, Role::Both, "plain files stay");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_server_certificate_loads_as_0_1_stores_it() {
    let secrets: ServerSecrets =
        json::from_slice(&read("linux-0.1.2/plaintext/secrets.json")).unwrap();
    let state = secrets.skakel.unwrap();
    let certificate = state.certificate().unwrap();
    assert_eq!(certificate.fingerprint(), LINUX_012_FINGERPRINT);
    let certified = certificate.certified_key().unwrap();
    assert_eq!(
        certified.cert[0].as_ref(),
        certificate.certificate.as_slice()
    );
    assert_eq!(certified.key.algorithm(), rustls::SignatureAlgorithm::ECDSA);

    let fingerprints =
        String::from_utf8(read("csharp-linux/plain/extra/fingerprints.txt")).unwrap();
    let plain: ServerSecrets =
        json::from_slice(&read("csharp-linux/plain/plaintext/secrets.json")).unwrap();
    let state = plain.skakel.unwrap();
    assert_eq!(
        state.certificate().unwrap().fingerprint(),
        fingerprints.lines().next().unwrap()
    );
    assert_eq!(
        state.next_certificate().unwrap().unwrap().fingerprint(),
        fingerprints.lines().nth(1).unwrap()
    );

    // No 0.1.x made an RSA certificate, but one loads all the same.
    let rsa = ServerCertificate::from_pfx_base64(
        &String::from_utf8(read("csharp-linux/plain/extra/rsa.pfx.b64")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        rsa.certified_key().unwrap().key.algorithm(),
        rustls::SignatureAlgorithm::RSA
    );
}

#[test]
fn a_0_1_0_secrets_file_carries_over_as_0_1_1_carried_it() {
    let legacy: ServerSecrets =
        json::from_slice(&read("csharp-linux/plain/plaintext/secrets-0.1.0.json")).unwrap();
    let now = OffsetDateTime::now_utc();

    let carried = legacy.skakel_state(now).unwrap().unwrap();

    assert_eq!(
        carried
            .state
            .clients
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["client-2222222222"]
    );
    assert_eq!(carried.state.clients[0].generation, 3);
    assert_eq!(
        carried
            .dropped
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["client-1111111111"]
    );
    assert_eq!(
        carried.state.signing_key.as_ref().unwrap().key_id,
        "koppel-1"
    );
    assert_eq!(carried.state.signing_key.as_ref().unwrap().created_at, now);
    assert_eq!(
        carried.state.next_certificate.unwrap().received_by,
        ["client-2222222222"]
    );
    let fingerprints =
        String::from_utf8(read("csharp-linux/plain/extra/fingerprints.txt")).unwrap();
    assert_eq!(
        ServerCertificate::from_pfx_base64(&carried.state.certificate_pfx)
            .unwrap()
            .fingerprint(),
        fingerprints.lines().next().unwrap()
    );
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
    let server = open::server_secrets(&paths, &secrets);
    server.start().await.unwrap();
    let fingerprints = fs::read_to_string(source.join("extra/fingerprints.txt")).unwrap();
    assert_eq!(
        server
            .current()
            .skakel
            .clone()
            .unwrap()
            .certificate()
            .unwrap()
            .fingerprint(),
        fingerprints.lines().next().unwrap()
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
