use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::host::UsbIpDriver;
use crate::{BoxFuture, BusId, UsbError};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// `CREATE_NO_WINDOW`: the command gets no console window of its own.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The usbip-win2 driver (BSD-2-Clause, <https://github.com/vadimgrn/usbip-win2>), driven through
/// its documented `usbip.exe` command line. Its kernel driver connects to the USB/IP server itself.
#[derive(Debug, Clone)]
pub struct UsbipWin2Driver {
    usbip_exe: PathBuf,
}

impl UsbipWin2Driver {
    /// Koppel's bundled copy, or a standalone install's `C:\Program Files\USBip\usbip.exe`.
    pub fn new(usbip_exe: impl Into<PathBuf>) -> Self {
        Self {
            usbip_exe: usbip_exe.into(),
        }
    }

    /// Asks the driver to stop its attach attempts for the device. Its help does not say what
    /// `--stop` exits with when there are none, and for a device that was never attached that is
    /// the usual case, so a failed stop never fails the attach or the detach: the command after
    /// it reports what really went wrong.
    async fn stop_attempts(&self, port: u16, host: &str, bus_id: &BusId) {
        let port = port.to_string();
        let _ = self
            .run(&[
                "--tcp-port",
                &port,
                "attach",
                "--remote",
                host,
                &bus_id_option(bus_id),
                "--stop",
            ])
            .await;
    }

    async fn run(&self, arguments: &[&str]) -> Result<String, UsbError> {
        if let Some(reason) = self.unavailable() {
            return Err(UsbError::failed(reason));
        }
        let mut command = Command::new(&self.usbip_exe);
        command
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .kill_on_drop(true);
        let child = command.spawn().map_err(|e| {
            UsbError::failed(format!("Could not start {}: {e}", self.usbip_exe.display()))
        })?;
        let last = arguments.last().copied().unwrap_or_default();
        // Dropping the child on the timeout kills it.
        let output = tokio::time::timeout(COMMAND_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| {
                UsbError::failed(format!(
                    "usbip.exe {last} did not finish in {} seconds.",
                    COMMAND_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|e| UsbError::failed(format!("usbip.exe {last} failed: {e}")))?;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(UsbError::failed(
                error.strip_prefix("error: ").unwrap_or(&error).to_owned(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// One token, so a value can never be read as an option of its own (usbip.exe takes `--bus-id=`).
fn bus_id_option(bus_id: &BusId) -> String {
    format!("--bus-id={bus_id}")
}

impl UsbIpDriver for UsbipWin2Driver {
    fn unavailable(&self) -> Option<String> {
        if self.usbip_exe.is_file() {
            None
        } else {
            Some(format!(
                "The Windows USB driver is not installed ({} is missing). Reinstall Koppel to add it.",
                self.usbip_exe.display()
            ))
        }
    }

    fn attach<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        bus_id: &'a BusId,
    ) -> BoxFuture<'a, Result<u32, UsbError>> {
        Box::pin(async move {
            // After a dropped connection the driver keeps trying to attach the device by itself.
            // Stop that first, so the device is not attached twice.
            self.stop_attempts(port, host, bus_id).await;

            // --once: if the attach fails, fail now instead of retrying in the background.
            let port = port.to_string();
            let output = self
                .run(&[
                    "--tcp-port",
                    &port,
                    "attach",
                    "--remote",
                    host,
                    &bus_id_option(bus_id),
                    "--terse",
                    "--once",
                ])
                .await?;
            output.trim().parse::<u32>().map_err(|_| {
                UsbError::failed(format!(
                    "usbip.exe attached {bus_id} but did not say on which port: {}",
                    output.trim()
                ))
            })
        })
    }

    fn detach<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        bus_id: &'a BusId,
        driver_port: u32,
    ) -> BoxFuture<'a, Result<(), UsbError>> {
        Box::pin(async move {
            // The driver attaches again by itself after a dropped connection, so stop that first.
            self.stop_attempts(port, host, bus_id).await;
            self.run(&["detach", "-p", &driver_port.to_string()])
                .await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("koppel-usbip-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        /// A batch file that plays usbip.exe.
        fn fake_usbip(&self, body: &str) -> PathBuf {
            let path = self.0.join("usbip.cmd");
            let body = body.lines().collect::<Vec<_>>().join("\r\n");
            fs::write(&path, format!("@echo off\r\n{body}\r\n")).unwrap();
            path
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn attach_goes_on_when_there_is_nothing_to_stop() {
        let folder = Folder::new("attach");
        let driver = UsbipWin2Driver::new(folder.fake_usbip(
            "echo %* | findstr /C:\"--stop\" >nul && (echo error: no attach attempt to stop 1>&2 & exit /b 1)\necho 7",
        ));

        let port = driver
            .attach("127.0.0.1", 3242, &"1-1.4".parse().unwrap())
            .await
            .unwrap();

        assert_eq!(port, 7);
    }

    #[tokio::test]
    async fn detach_goes_on_when_there_is_nothing_to_stop() {
        let folder = Folder::new("detach");
        let log = folder.0.join("calls.txt");
        let driver = UsbipWin2Driver::new(folder.fake_usbip(&format!(
            "echo %* >> \"{}\"\necho %* | findstr /C:\"--stop\" >nul && (echo error: no attach attempt to stop 1>&2 & exit /b 1)\nexit /b 0",
            log.display()
        )));

        driver
            .detach("127.0.0.1", 3242, &"1-1.4".parse().unwrap(), 7)
            .await
            .unwrap();

        assert!(fs::read_to_string(&log).unwrap().contains("detach -p 7"));
    }

    #[tokio::test]
    async fn attach_still_fails_when_the_attach_itself_fails() {
        let folder = Folder::new("fails");
        let driver =
            UsbipWin2Driver::new(folder.fake_usbip("echo error: device busy 1>&2\nexit /b 1"));

        let attach = driver
            .attach("127.0.0.1", 3242, &"1-1.4".parse().unwrap())
            .await;

        assert_eq!(attach.unwrap_err().to_string(), "device busy");
    }

    #[test]
    fn a_missing_usbip_exe_says_to_reinstall() {
        let driver = UsbipWin2Driver::new(r"C:\nowhere\usbip.exe");

        assert!(driver.unavailable().unwrap().contains("Reinstall Koppel"));
    }
}
