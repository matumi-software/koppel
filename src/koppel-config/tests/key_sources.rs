//! Where the key that seals the secret files comes from: the keys directory, `KOPPEL_KEY_FILE`, a
//! systemd credential or a Docker secret, and what happens to files when it changes.

use std::fs;
use std::path::{Path, PathBuf};

use koppel_config::{KeySource, KeySources, SecretBox};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "koppel-keys-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn environment(path: &Path) -> KeySources {
    KeySources {
        environment: Some(path.to_path_buf()),
        ..KeySources::none()
    }
}

const KEY: &str = "Zm9yIHRlc3RzIG9ubHksIGEga2V5IG9mIGF0IGxlYXN0IDMyIGJ5dGVz";

#[test]
fn a_key_from_outside_the_volume_seals_and_nothing_is_made_beside_the_files() {
    let dir = temp_dir("outside");
    let keys = dir.join("keys");
    fs::create_dir(&keys).unwrap();
    let key_file = dir.join("koppel-key");
    fs::write(&key_file, format!("{KEY}\n")).unwrap();

    let secrets = SecretBox::with_sources(&keys, &environment(&key_file)).unwrap();

    assert_eq!(*secrets.source(), KeySource::Environment(key_file.clone()));
    assert!(secrets.source().is_outside_the_volume());
    assert!(names(&keys).is_empty(), "no key file is made in the volume");
    let sealed = secrets.protect(b"{\"a\":1}").unwrap();

    // The same file, with or without its line break, is the same key.
    fs::write(&key_file, KEY).unwrap();
    let again = SecretBox::with_sources(&keys, &environment(&key_file)).unwrap();
    assert_eq!(again.unprotect(&sealed).unwrap(), b"{\"a\":1}");

    // Another key does not open it, and the volume's own key does not either.
    fs::write(&key_file, "a different key of at least 32 bytes, for sure").unwrap();
    let other = SecretBox::with_sources(&keys, &environment(&key_file)).unwrap();
    let refused = other.unprotect(&sealed).unwrap_err().to_string();
    assert!(refused.contains("another key"), "{refused}");
    let on_volume = SecretBox::with_key_file(&keys).unwrap();
    assert!(on_volume.unprotect(&sealed).is_err());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_key_that_is_too_short_or_missing_is_refused_and_named() {
    let dir = temp_dir("short");
    let keys = dir.join("keys");
    fs::create_dir(&keys).unwrap();
    let key_file = dir.join("koppel-key");
    fs::write(&key_file, "tiny\n").unwrap();

    let error = SecretBox::with_sources(&keys, &environment(&key_file)).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains(&key_file.display().to_string()),
        "{message}"
    );
    assert!(message.contains("needs at least 32"), "{message}");

    // KOPPEL_KEY_FILE that names nothing is an error, not a quiet fall back to the volume's key.
    let error = SecretBox::with_sources(&keys, &environment(&dir.join("missing"))).unwrap_err();
    assert!(error.to_string().contains("not a file"), "{error}");
    assert!(names(&keys).is_empty());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_sources_are_tried_in_order() {
    let dir = temp_dir("order");
    let keys = dir.join("keys");
    fs::create_dir(&keys).unwrap();
    let make = |name: &str, text: &str| {
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        path
    };
    let env = make("env", "the environment key, 32 bytes or more of it");
    let credential = make("credential", "the credential key, 32 bytes or more of it");
    let secret = make("secret", "the Docker secret key, 32 bytes or more of it");
    let all = KeySources {
        environment: Some(env.clone()),
        credential: Some(credential.clone()),
        docker_secret: Some(secret.clone()),
    };

    let source = |sources: &KeySources| {
        SecretBox::with_sources(&keys, sources)
            .unwrap()
            .source()
            .clone()
    };
    assert_eq!(source(&all), KeySource::Environment(env));
    let no_env = KeySources {
        environment: None,
        ..all.clone()
    };
    assert_eq!(source(&no_env), KeySource::Credential(credential));
    let only_secret = KeySources {
        credential: Some(dir.join("absent")),
        ..no_env
    };
    assert_eq!(source(&only_secret), KeySource::DockerSecret(secret));
    let nothing = KeySources {
        docker_secret: Some(dir.join("absent")),
        ..only_secret
    };
    assert_eq!(source(&nothing), KeySource::KeysDirectory);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn files_sealed_beside_them_move_to_a_key_outside_and_reseal() {
    let dir = temp_dir("move");
    let keys = dir.join("keys");
    fs::create_dir(&keys).unwrap();
    let file = dir.join("secrets.json");
    let before = SecretBox::with_key_file(&keys).unwrap();
    fs::write(&file, before.protect(b"{\"k\":\"v\"}").unwrap()).unwrap();
    let key_file = dir.join("koppel-key");
    fs::write(&key_file, KEY).unwrap();

    // With the new key the old file still opens, through the key file kept in the volume.
    let moved = SecretBox::with_sources(&keys, &environment(&key_file)).unwrap();
    assert_eq!(moved.keys_read(), 2);
    assert_eq!(
        moved.unprotect(&fs::read(&file).unwrap()).unwrap(),
        b"{\"k\":\"v\"}"
    );

    // Sealing again moves it to the new key; running it twice changes nothing more.
    assert_eq!(
        moved.reseal(std::slice::from_ref(&file)),
        std::slice::from_ref(&file)
    );
    let resealed = fs::read(&file).unwrap();
    assert!(moved.reseal(std::slice::from_ref(&file)).is_empty());
    assert_eq!(fs::read(&file).unwrap(), resealed);

    // Now the key file in the volume can go, and the file still opens.
    fs::remove_file(keys.join("koppel-secrets.key")).unwrap();
    let alone = SecretBox::with_sources(&keys, &environment(&key_file)).unwrap();
    assert_eq!(alone.keys_read(), 1);
    assert_eq!(alone.unprotect(&resealed).unwrap(), b"{\"k\":\"v\"}");
    assert!(names(&keys).is_empty(), "nothing was made in the volume");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_rotated_key_file_is_read_and_everything_is_sealed_again_with_the_new_one() {
    let dir = temp_dir("rotate");
    let keys = dir.join("keys");
    fs::create_dir(&keys).unwrap();
    let file = dir.join("ui-auth.json");
    let old = SecretBox::with_key_file(&keys).unwrap();
    fs::write(&file, old.protect(b"{\"stamp\":1}").unwrap()).unwrap();

    // The rotation: put the key aside under a name of its own and start.
    fs::rename(
        keys.join("koppel-secrets.key"),
        keys.join("koppel-secrets.2026-11-12.key"),
    )
    .unwrap();
    let rotated = SecretBox::with_key_file(&keys).unwrap();
    assert_eq!(rotated.keys_read(), 2);
    assert_eq!(
        rotated.unprotect(&fs::read(&file).unwrap()).unwrap(),
        b"{\"stamp\":1}"
    );
    assert_eq!(rotated.reseal(std::slice::from_ref(&file)).len(), 1);

    // The new key alone opens it now, so the old file can be deleted.
    fs::remove_file(keys.join("koppel-secrets.2026-11-12.key")).unwrap();
    let new_only = SecretBox::with_key_file(&keys).unwrap();
    assert_eq!(new_only.keys_read(), 1);
    assert_eq!(
        new_only.unprotect(&fs::read(&file).unwrap()).unwrap(),
        b"{\"stamp\":1}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_file_sealed_with_a_key_that_is_not_in_the_ring_is_left_alone() {
    let dir = temp_dir("foreign");
    let (one, two) = (dir.join("one"), dir.join("two"));
    fs::create_dir_all(&one).unwrap();
    fs::create_dir_all(&two).unwrap();
    let file = dir.join("connection.json");
    let sealed = SecretBox::with_key_file(&one)
        .unwrap()
        .protect(b"{}")
        .unwrap();
    fs::write(&file, &sealed).unwrap();

    let ours = SecretBox::with_key_file(&two).unwrap();
    assert!(
        ours.reseal(&[file.clone(), dir.join("missing.json")])
            .is_empty()
    );
    assert_eq!(fs::read(&file).unwrap(), sealed);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn an_outside_key_is_rotated_by_keeping_the_old_one_as_a_retired_key() {
    let dir = temp_dir("rotate-outside");
    let keys = dir.join("keys");
    fs::create_dir(&keys).unwrap();
    let old_key = dir.join("old-key");
    fs::write(&old_key, KEY).unwrap();
    let file = dir.join("secrets.json");
    let before = SecretBox::with_sources(&keys, &environment(&old_key)).unwrap();
    fs::write(&file, before.protect(b"{\"k\":1}").unwrap()).unwrap();

    let new_key = dir.join("new-key");
    fs::write(&new_key, "the new key, which is also at least 32 bytes").unwrap();
    fs::copy(&old_key, keys.join("koppel-secrets.2026-11-12.key")).unwrap();
    let rotated = SecretBox::with_sources(&keys, &environment(&new_key)).unwrap();
    assert_eq!(rotated.keys_read(), 2);
    assert_eq!(rotated.reseal(std::slice::from_ref(&file)).len(), 1);

    fs::remove_file(keys.join("koppel-secrets.2026-11-12.key")).unwrap();
    let alone = SecretBox::with_sources(&keys, &environment(&new_key)).unwrap();
    assert_eq!(
        alone.unprotect(&fs::read(&file).unwrap()).unwrap(),
        b"{\"k\":1}"
    );
    let _ = fs::remove_dir_all(dir);
}
