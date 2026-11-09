//! The kernel's usbfs device nodes (`/dev/bus/usb/BBB/DDD`), for what sysfs cannot do.

use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

use crate::UsbError;

/// `USBDEVFS_RESET`, `_IO('U', 20)` in `linux/usbdevice_fs.h`.
const USBDEVFS_RESET: libc::Ioctl = 0x5514;

/// A USB port reset, as the usbreset tool does it. The device keeps its driver and address.
pub fn reset(node: &Path) -> Result<(), UsbError> {
    let file = OpenOptions::new().write(true).open(node).map_err(|e| {
        UsbError::new(format!(
            "Could not open {} to reset it: {e}",
            node.display()
        ))
    })?;
    // SAFETY: the descriptor belongs to `file`, which outlives the call, and USBDEVFS_RESET
    // takes no argument.
    let result = unsafe { libc::ioctl(file.as_raw_fd(), USBDEVFS_RESET, 0) };
    if result != 0 {
        let errno = io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or_default();
        return Err(UsbError::new(format!(
            "The kernel refused to reset {} (errno {errno}).",
            node.display()
        )));
    }
    Ok(())
}
