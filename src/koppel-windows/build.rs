//! Gives KoppelDesktop.exe Koppel's icon, which the tray shows, its version, and a manifest that
//! makes it DPI aware and gives its message boxes the current look.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let icon = "../../resources/design-system/assets/Logos/koppel-app-icon.ico";
        println!("cargo:rerun-if-changed={icon}");
        println!("cargo:rerun-if-changed=desktop.manifest");
        winresource::WindowsResource::new()
            .set_icon(icon)
            .set("FileDescription", "Koppel")
            .set("ProductName", "Koppel")
            .set("CompanyName", "Matumi (Pty) Ltd")
            .set("LegalCopyright", "Copyright (C) 2026 Matumi (Pty) Ltd")
            .set_manifest_file("desktop.manifest")
            .compile()
            .expect("the Windows resource compiler embeds the icon and the manifest");
    }
}
