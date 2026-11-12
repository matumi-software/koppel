use std::net::{Ipv4Addr, SocketAddrV4};

use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, TCP_TABLE_OWNER_PID_ALL};

use crate::UsbError;
use crate::peer_check::{TcpRow, TcpTable};

const AF_INET: u32 = 2;

/// Windows' `GetExtendedTcpTable`, the same list `netstat -ano` prints.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsTcpTable;

impl TcpTable for WindowsTcpTable {
    fn rows(&self) -> Result<Vec<TcpRow>, UsbError> {
        let mut size = 0u32;
        let mut buffer: Vec<u32> = Vec::new();
        // The table can grow between the size query and the read, so ask again if it did.
        for _ in 0..5 {
            let table = if buffer.is_empty() {
                None
            } else {
                Some(buffer.as_mut_ptr().cast())
            };
            // SAFETY: `table` is either none, to ask for the size, or `buffer`, which holds at
            // least `size` bytes.
            let result = unsafe {
                GetExtendedTcpTable(table, &mut size, false, AF_INET, TCP_TABLE_OWNER_PID_ALL, 0)
            };
            if result == NO_ERROR.0 && !buffer.is_empty() {
                return Ok(parse(&buffer));
            }
            if result != ERROR_INSUFFICIENT_BUFFER.0 && result != NO_ERROR.0 {
                return Err(UsbError::failed(format!(
                    "Could not read the TCP table (error {result})."
                )));
            }
            buffer = vec![0u32; (size as usize).div_ceil(4).max(1)];
        }
        Err(UsbError::failed(
            "The TCP table kept changing while it was read.",
        ))
    }
}

/// `MIB_TCPTABLE_OWNER_PID`: a count, then rows of state, local address, local port, remote
/// address, remote port and owning PID, six `u32`s each.
fn parse(table: &[u32]) -> Vec<TcpRow> {
    let count = table[0] as usize;
    table[1..]
        .as_chunks::<6>()
        .0
        .iter()
        .take(count)
        .map(|row| TcpRow {
            local: SocketAddrV4::new(address(row[1]), port(row[2])),
            remote: SocketAddrV4::new(address(row[3]), port(row[4])),
            owning_pid: row[5],
        })
        .collect()
}

/// The address is in network byte order, so its bytes in memory are the dotted quad.
fn address(raw: u32) -> Ipv4Addr {
    Ipv4Addr::from(raw.to_ne_bytes())
}

/// The port sits in the low 16 bits in network byte order.
fn port(raw: u32) -> u16 {
    let bytes = raw.to_ne_bytes();
    u16::from_be_bytes([bytes[0], bytes[1]])
}

#[cfg(test)]
mod tests {
    use std::net::{SocketAddr, TcpListener, TcpStream};

    use super::*;
    use crate::peer_check::{KernelPeerCheck, owner};

    #[tokio::test]
    async fn windows_tcp_table_lists_a_loopback_connection_under_its_owner() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let SocketAddr::V4(gateway) = listener.local_addr().unwrap() else {
            unreachable!()
        };
        let _client = TcpStream::connect(gateway).unwrap();
        let (_accepted, peer) = listener.accept().unwrap();
        let SocketAddr::V4(peer) = peer else {
            unreachable!()
        };

        let rows = WindowsTcpTable.rows().unwrap();

        assert_eq!(owner(&rows, peer, gateway), Some(std::process::id()));
        let check = KernelPeerCheck::new(std::sync::Arc::new(WindowsTcpTable));
        assert!(!check.is_kernel(peer, gateway).await);
    }
}
