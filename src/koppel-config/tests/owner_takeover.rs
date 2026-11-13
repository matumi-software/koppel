//! A volume made by the 0.1.x Docker image has its folders owned by 1654, and Koppel runs as root.
//! The test makes such folders for real, so it needs root (the Linux test container is); anywhere
//! else it does nothing.
#![cfg(unix)]

use std::os::unix::fs::{MetadataExt, chown};

use koppel_config::KoppelPaths;

fn root() -> bool {
    // SAFETY: geteuid takes no arguments and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("koppel-owner-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn start_up_takes_over_folders_of_the_old_image_user() {
    if !root() {
        return;
    }
    let base = scratch("takeover");
    let paths = KoppelPaths::new(base.join("config"), base.join("keys"));
    for folder in [&paths.config, &paths.keys] {
        std::fs::create_dir_all(folder).unwrap();
        std::fs::write(folder.join("secrets.json"), b"{}").unwrap();
        chown(folder, Some(1654), Some(1654)).unwrap();
    }
    paths.prepare().unwrap();
    for folder in [&paths.config, &paths.keys] {
        let metadata = std::fs::metadata(folder).unwrap();
        assert_eq!((metadata.uid(), metadata.gid()), (0, 0));
        assert_eq!(metadata.mode() & 0o777, 0o700);
        assert_eq!(
            std::fs::metadata(folder.join("secrets.json"))
                .unwrap()
                .uid(),
            0
        );
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn start_up_refuses_folders_of_any_other_user_and_says_how_to_fix_it() {
    if !root() {
        return;
    }
    let base = scratch("refuse");
    let paths = KoppelPaths::new(base.join("config"), base.join("keys"));
    std::fs::create_dir_all(&paths.config).unwrap();
    chown(&paths.config, Some(4242), Some(4242)).unwrap();
    let refused = paths.prepare().unwrap_err().to_string();
    assert!(
        refused.contains("belongs to user 4242, not to user 0"),
        "{refused}"
    );
    assert!(refused.contains("docker compose stop"), "{refused}");
    assert!(refused.contains("alpine chown -R 0:0 /data"), "{refused}");
    assert!(refused.contains("sudo chown -R 0 "), "{refused}");
    assert_eq!(std::fs::metadata(&paths.config).unwrap().uid(), 4242);
    let _ = std::fs::remove_dir_all(base);
}
