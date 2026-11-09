//! Which connections the gateway serves. On native Windows only the kernel may connect: the
//! usbip-win2 driver connects from the kernel, so Windows lists the connection under the System
//! process (PID 4), and anything a user-mode process opens is listed under that process's own
//! PID. The gateway gives the client's access to the server to whoever reaches it, so on a
//! machine with several users or services it only accepts the kernel.

use std::net::{SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use crate::{BoxFuture, UsbError};

/// One IPv4 TCP connection as Windows lists it, with the process that owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpRow {
    pub local: SocketAddrV4,
    pub remote: SocketAddrV4,
    pub owning_pid: u32,
}

/// The machine's TCP connections. A fake stands in for it in tests.
pub trait TcpTable: Send + Sync {
    fn rows(&self) -> Result<Vec<TcpRow>, UsbError>;
}

/// Decides whether a connection to the loopback gateway came from the kernel.
pub struct KernelPeerCheck {
    table: Arc<dyn TcpTable>,
    attempts: u32,
    retry_delay: Duration,
}

impl KernelPeerCheck {
    /// The System process, the owner of connections the kernel itself makes.
    pub const KERNEL_PID: u32 = 4;

    /// Three looks, 50 ms apart.
    pub fn new(table: Arc<dyn TcpTable>) -> Self {
        Self::with_retries(table, 3, Duration::from_millis(50))
    }

    pub fn with_retries(table: Arc<dyn TcpTable>, attempts: u32, retry_delay: Duration) -> Self {
        Self {
            table,
            attempts,
            retry_delay,
        }
    }

    /// True when `peer` (the accepted connection's remote end) is a connection to `gateway` that
    /// the kernel owns. The row is looked up from the client side: its local end is the peer and
    /// its remote end is the gateway. A table that cannot be read counts as not the kernel.
    pub async fn is_kernel(&self, peer: SocketAddrV4, gateway: SocketAddrV4) -> bool {
        for _ in 0..self.attempts {
            match self.table.rows() {
                Ok(rows) => {
                    if let Some(pid) = owner(&rows, peer, gateway) {
                        return pid == Self::KERNEL_PID;
                    }
                }
                Err(e) => {
                    tracing::warn!("Could not check who opened a gateway connection: {e}");
                    return false;
                }
            }
            // The table can trail the accept by a moment.
            tokio::time::sleep(self.retry_delay).await;
        }
        false
    }
}

/// The owner of the connection from `peer` to `gateway`, or `None` when it is not listed.
pub fn owner(rows: &[TcpRow], peer: SocketAddrV4, gateway: SocketAddrV4) -> Option<u32> {
    rows.iter()
        .find(|row| row.local == peer && row.remote == gateway)
        .map(|row| row.owning_pid)
}

/// Decides which connections the gateway serves.
pub trait GatewayPeerCheck: Send + Sync {
    /// Whether the connection from `peer`, which the gateway just accepted on `gateway`, may be
    /// served.
    fn allows(&self, peer: SocketAddr, gateway: SocketAddr) -> BoxFuture<'_, bool>;
}

/// Serves everyone who reaches the gateway.
pub struct AnyPeer;

impl GatewayPeerCheck for AnyPeer {
    fn allows(&self, _: SocketAddr, _: SocketAddr) -> BoxFuture<'_, bool> {
        Box::pin(async { true })
    }
}

/// Serves only connections the Windows kernel owns. The kernel also connects for another user's
/// `usbip attach`, so the client still decides which imports it serves. An IPv6 connection cannot
/// be looked up and is refused.
pub struct KernelPeer(pub KernelPeerCheck);

impl GatewayPeerCheck for KernelPeer {
    fn allows(&self, peer: SocketAddr, gateway: SocketAddr) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            match (peer, gateway) {
                (SocketAddr::V4(peer), SocketAddr::V4(gateway)) => {
                    self.0.is_kernel(peer, gateway).await
                }
                _ => false,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::sync::Mutex;

    use super::*;

    const GATEWAY: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 3242);
    const PEER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 50000);

    fn row(local_port: u16, remote_port: u16, pid: u32) -> TcpRow {
        TcpRow {
            local: SocketAddrV4::new(Ipv4Addr::LOCALHOST, local_port),
            remote: SocketAddrV4::new(Ipv4Addr::LOCALHOST, remote_port),
            owning_pid: pid,
        }
    }

    struct FakeTable {
        snapshots: Vec<Vec<TcpRow>>,
        reads: Mutex<usize>,
    }

    impl FakeTable {
        fn new(snapshots: Vec<Vec<TcpRow>>) -> Arc<Self> {
            Arc::new(Self {
                snapshots,
                reads: Mutex::new(0),
            })
        }

        fn reads(&self) -> usize {
            *self.reads.lock().unwrap()
        }
    }

    impl TcpTable for FakeTable {
        fn rows(&self) -> Result<Vec<TcpRow>, UsbError> {
            let mut reads = self.reads.lock().unwrap();
            let rows = self.snapshots[(*reads).min(self.snapshots.len() - 1)].clone();
            *reads += 1;
            Ok(rows)
        }
    }

    fn check(table: Arc<FakeTable>) -> KernelPeerCheck {
        KernelPeerCheck::with_retries(table, 3, Duration::ZERO)
    }

    #[tokio::test]
    async fn connection_owned_by_the_kernel_is_allowed() {
        let table = FakeTable::new(vec![vec![row(50000, 3242, KernelPeerCheck::KERNEL_PID)]]);

        assert!(check(table).is_kernel(PEER, GATEWAY).await);
    }

    #[tokio::test]
    async fn connection_owned_by_a_user_process_is_refused() {
        let table = FakeTable::new(vec![vec![row(50000, 3242, 4321)]]);

        assert!(!check(table).is_kernel(PEER, GATEWAY).await);
    }

    #[tokio::test]
    async fn only_the_row_for_this_connection_counts() {
        // The kernel owns another connection to the gateway; this one is a user process's.
        let table = FakeTable::new(vec![vec![
            row(50001, 3242, KernelPeerCheck::KERNEL_PID),
            row(50000, 3242, 4321),
            row(50000, 9999, KernelPeerCheck::KERNEL_PID),
        ]]);

        assert!(!check(table).is_kernel(PEER, GATEWAY).await);
    }

    #[tokio::test]
    async fn connection_missing_from_the_table_is_refused_after_a_few_tries() {
        let table = FakeTable::new(vec![vec![]]);

        assert!(!check(table.clone()).is_kernel(PEER, GATEWAY).await);
        assert_eq!(table.reads(), 3);
    }

    #[tokio::test]
    async fn connection_that_appears_a_moment_late_is_found() {
        let table = FakeTable::new(vec![
            vec![],
            vec![row(50000, 3242, KernelPeerCheck::KERNEL_PID)],
        ]);

        assert!(check(table).is_kernel(PEER, GATEWAY).await);
    }

    #[tokio::test]
    async fn an_ipv6_connection_is_refused() {
        let table = FakeTable::new(vec![vec![row(50000, 3242, KernelPeerCheck::KERNEL_PID)]]);
        let peer = KernelPeer(check(table));

        assert!(
            !peer
                .allows(
                    "[::1]:50000".parse().unwrap(),
                    "[::1]:3242".parse().unwrap()
                )
                .await
        );
        assert!(peer.allows(PEER.into(), GATEWAY.into()).await);
    }
}
