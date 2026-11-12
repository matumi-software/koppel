//! Several Koppel processes starting at once on a new install make one key file, and all use it.
//! The test starts itself again as the workers, so the processes are real.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use koppel_config::SecretBox;

const DIRECTORY: &str = "KOPPEL_RACE_DIRECTORY";
const GO: &str = "KOPPEL_RACE_GO";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

/// The worker. Without the environment it is not a test, so it passes.
#[test]
fn key_file_race_worker() {
    let (Ok(directory), Ok(go)) = (std::env::var(DIRECTORY), std::env::var(GO)) else {
        return;
    };
    // Every worker waits for the same signal, so they open the key file together.
    let waited = Instant::now();
    while !Path::new(&go).exists() {
        assert!(waited.elapsed() < Duration::from_secs(20), "no signal");
        std::hint::spin_loop();
    }
    let secrets = SecretBox::with_key_file(&PathBuf::from(directory)).unwrap();
    println!("SEALED {}", hex(&secrets.protect(b"{}").unwrap()));
}

#[test]
fn processes_that_start_together_end_up_with_one_key() {
    if std::env::var(DIRECTORY).is_ok() {
        return;
    }
    for round in 0..5 {
        let root = std::env::temp_dir().join(format!("koppel-race-{}-{round}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let keys = root.join("keys");
        std::fs::create_dir_all(&keys).unwrap();
        let go = root.join("go");

        let children: Vec<_> = (0..6)
            .map(|_| {
                Command::new(std::env::current_exe().unwrap())
                    .args(["key_file_race_worker", "--exact", "--nocapture"])
                    .env(DIRECTORY, &keys)
                    .env(GO, &go)
                    .stdout(Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(&go, b"go").unwrap();

        let mut sealed = Vec::new();
        for child in children {
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success(), "a worker failed");
            let text = String::from_utf8(output.stdout).unwrap();
            let line = text
                .lines()
                .find_map(|l| l.strip_prefix("SEALED "))
                .expect("a worker sealed nothing");
            sealed.push(unhex(line));
        }

        // One key: the file opens every worker's output, and nothing else is left in the directory.
        let secrets = SecretBox::with_key_file(&keys).unwrap();
        for blob in &sealed {
            assert_eq!(secrets.unprotect(blob).unwrap(), b"{}");
        }
        let names: Vec<_> = std::fs::read_dir(&keys)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["koppel-secrets.key"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
