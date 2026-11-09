//! Facts about this machine and process that never change while it runs.

use time::OffsetDateTime;

/// What the side panel and the server's identity show, fixed at start. The UI and network ports are
/// the same port unless `Ports:Network` is set.
#[derive(Debug, Clone)]
pub struct Host {
    /// The machine's name, as .NET's `Environment.MachineName` gives it.
    pub name: String,
    pub version: String,
    pub started_at: OffsetDateTime,
    pub ui_port: u16,
    pub network_port: u16,
    /// True when Koppel runs as the installed Windows service, which keeps a tray icon (#88).
    pub tray: bool,
    /// The operating system, for the side panel.
    pub os: String,
}

/// The machine's name: the NetBIOS name on Windows, the host name up to its first dot elsewhere, as
/// .NET's `Environment.MachineName` gives it.
pub fn machine_name() -> String {
    #[cfg(windows)]
    let name = std::env::var("COMPUTERNAME").ok();
    #[cfg(not(windows))]
    let name = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .or_else(|| std::env::var("HOSTNAME").ok());
    name.map(|n| n.trim().split('.').next().unwrap_or_default().to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "localhost".to_owned())
}

/// The operating system as the side panel shows it. On Linux the kernel matters most, since it
/// carries the USB/IP drivers, and in Docker it is the host's.
pub fn os_label() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(kernel) = std::fs::read_to_string("/proc/sys/kernel/osrelease") {
            let kernel = kernel.trim();
            let in_container = std::path::Path::new("/.dockerenv").exists()
                || std::env::var("KOPPEL_RUNNING_IN_CONTAINER").as_deref() == Ok("true");
            return if in_container {
                format!("Linux kernel {kernel}, in Docker")
            } else {
                format!("Linux kernel {kernel}")
            };
        }
        "Linux".to_owned()
    }
    #[cfg(windows)]
    {
        windows_version()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        std::env::consts::OS.to_owned()
    }
}

/// "Microsoft Windows 10.0.26300", as .NET's `RuntimeInformation.OSDescription` says it. Read with
/// `RtlGetVersion`, which, unlike `GetVersionEx`, does not pretend to be Windows 8 to a program
/// without a manifest.
#[cfg(windows)]
fn windows_version() -> String {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: the structure is ours, sized as the call expects.
    let status = unsafe { RtlGetVersion(&mut info) };
    if status.is_ok() {
        format!(
            "Microsoft Windows {}.{}.{}",
            info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber
        )
    } else {
        "Microsoft Windows".to_owned()
    }
}
