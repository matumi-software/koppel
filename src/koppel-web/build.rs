//! Gzips the built UI into OUT_DIR, where `ui.rs` embeds it.
//!
//! Each file lands in `ui/gzip` when gzip makes it at least 5% smaller, otherwise as it is in `ui/raw`
//! (the fonts, which are compressed already). Only one copy of each file goes into the program.
//!
//! Without a built UI, a debug build (cargo build, test, clippy) gets a placeholder page and a warning,
//! so working on the Rust side needs no Node. A release build stops with an error instead, so a
//! program without its UI is never shipped by accident.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::exit;

use flate2::Compression;
use flate2::write::GzEncoder;

/// Where Vite writes the UI, from `src/`.
const UI_DIST: &str = "Koppel/src/Koppel.UI/dist";

const PLACEHOLDER: &str = "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Koppel</title></head>\n<body>\n<p>This Koppel was built without its UI. Build the UI and then Koppel again, from <code>src/</code>:</p>\n<pre>pnpm install\npnpm --filter koppel.ui build\ncargo build</pre>\n</body>\n</html>\n";

fn main() {
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let src = manifest.parent().expect("the crate sits in src/");
    let dist = UI_DIST
        .split('/')
        .fold(src.to_path_buf(), |path, part| path.join(part));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("ui");
    println!("cargo::rerun-if-changed={}", dist.display());

    let raw = out.join("raw");
    let gzip = out.join("gzip");
    if out.exists() {
        fs::remove_dir_all(&out)
            .unwrap_or_else(|e| fail(&format!("could not clear {}: {e}", out.display())));
    }
    for folder in [&raw, &gzip] {
        fs::create_dir_all(folder)
            .unwrap_or_else(|e| fail(&format!("could not create {}: {e}", folder.display())));
    }

    if dist.join("index.html").is_file() {
        add_folder(&dist, &dist, &raw, &gzip);
    } else if env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some() {
        println!(
            "cargo::warning=The UI is not built (src/{UI_DIST} has no index.html), so this build serves a placeholder page. \
             Run `pnpm install` and `pnpm --filter koppel.ui build` in src/ for the real one."
        );
        add_file(Path::new("index.html"), PLACEHOLDER.as_bytes(), &raw, &gzip);
    } else {
        fail(&format!(
            "the UI is not built (src/{UI_DIST} has no index.html). A release build embeds it, so build it first: \
             run `pnpm install` and `pnpm --filter koppel.ui build` in src/"
        ));
    }
}

fn add_folder(root: &Path, folder: &Path, raw: &Path, gzip: &Path) {
    let entries = fs::read_dir(folder)
        .unwrap_or_else(|e| fail(&format!("could not read {}: {e}", folder.display())));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| fail(&format!("could not read {}: {e}", folder.display())))
            .path();
        if path.is_dir() {
            add_folder(root, &path, raw, gzip);
        } else {
            let content = fs::read(&path)
                .unwrap_or_else(|e| fail(&format!("could not read {}: {e}", path.display())));
            let relative = path
                .strip_prefix(root)
                .expect("the walk stays under the root");
            add_file(relative, &content, raw, gzip);
        }
    }
}

fn add_file(relative: &Path, content: &[u8], raw: &Path, gzip: &Path) {
    let compressed = gzipped(content);
    let (folder, bytes) = if compressed.len() + content.len() / 20 < content.len() {
        (gzip, compressed.as_slice())
    } else {
        (raw, content)
    };
    let target = folder.join(relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|e| fail(&format!("could not create {}: {e}", parent.display())));
    }
    fs::write(&target, bytes)
        .unwrap_or_else(|e| fail(&format!("could not write {}: {e}", target.display())));
}

/// Gzip at the highest level. flate2 writes no file name and a zero time into the header, so the same
/// UI gives the same bytes on every build.
fn gzipped(content: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(content)
        .expect("writing to memory cannot fail");
    encoder.finish().expect("writing to memory cannot fail")
}

fn fail(message: &str) -> ! {
    eprintln!("error: {message}");
    exit(1);
}
