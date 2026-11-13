//! Generates the code for Koppel's own gRPC service from `proto/`.
//!
//! The `.proto` files compile with protox, in Rust, so no `protoc` is needed, as in Skakel.

use std::env;
use std::path::{Path, PathBuf};
use std::process::exit;

/// Koppel's own `.proto` files, under `proto/` in this crate.
const PROTO_FILES: &[&str] = &["koppel/remote/v1/devices.proto"];

fn main() {
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    generate_grpc(&manifest.join("proto"));
}

/// The messages, the tonic server for the server role and the tonic client for the client role. No
/// tonic transport: Skakel brings the connections, pinned to the server's certificate.
fn generate_grpc(root: &Path) {
    println!("cargo::rerun-if-changed={}", root.display());
    for file in PROTO_FILES {
        println!("cargo::rerun-if-changed={}", root.join(file).display());
    }
    let descriptors = protox::compile(PROTO_FILES, [root])
        .unwrap_or_else(|e| fail(&format!("the .proto files do not compile: {e:?}")));
    tonic_prost_build::configure()
        .build_transport(false)
        .compile_fds(descriptors)
        .unwrap_or_else(|e| fail(&format!("the gRPC code could not be generated: {e}")));
}

fn fail(message: &str) -> ! {
    eprintln!("error: {message}");
    exit(1);
}
