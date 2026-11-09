//! On Windows, gives Koppel.exe Koppel's icon, its version, and the message table the event log
//! reads its entries with (see koppel-windows' `event_log`): the installer names Koppel.exe as the
//! event source's message file.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let icon = "../../resources/design-system/assets/Logos/koppel-app-icon.ico";
        println!("cargo:rerun-if-changed={icon}");
        let out =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
        let table = out.join("messages.bin");
        std::fs::write(&table, message_table()).expect("the message table is written");
        winresource::WindowsResource::new()
            .set_icon(icon)
            .set("FileDescription", "Koppel")
            .set("ProductName", "Koppel")
            .append_rc_content(&format!(
                "1 MESSAGETABLE \"{}\"",
                table.display().to_string().replace('\\', "/")
            ))
            .compile()
            .expect("the Windows resource compiler embeds the icon and the message table");
    }
}

/// A message table, as the message compiler would write it, with messages 1 to 3 (error, warning,
/// information) that are each just the entry's own text, `%1`. Each entry is its length, a flag for
/// UTF-16, and the text with a terminating zero, padded to four bytes.
#[cfg(windows)]
fn message_table() -> Vec<u8> {
    const FIRST: u32 = 1;
    const LAST: u32 = 3;
    let text: Vec<u8> = "%1\r\n\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let length = (4 + text.len()).next_multiple_of(4);
    let mut table = Vec::new();
    table.extend_from_slice(&1u32.to_le_bytes());
    table.extend_from_slice(&FIRST.to_le_bytes());
    table.extend_from_slice(&LAST.to_le_bytes());
    table.extend_from_slice(&16u32.to_le_bytes());
    for _ in FIRST..=LAST {
        table.extend_from_slice(&(length as u16).to_le_bytes());
        table.extend_from_slice(&1u16.to_le_bytes());
        table.extend_from_slice(&text);
        table.resize(table.len() + length - 4 - text.len(), 0);
    }
    table
}
