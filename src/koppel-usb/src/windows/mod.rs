//! Windows: usbip-win2's driver attaches through its `usbip.exe`, and the gateway checks which
//! process owns a connection in Windows' TCP table.

mod tcp_table;
mod usbip_win2;

pub use tcp_table::WindowsTcpTable;
pub use usbip_win2::UsbipWin2Driver;
