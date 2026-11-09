//! Sharing the devices of a plain USB/IP server next to Koppel instead of this kernel's: usually
//! usbipd-win, on the Windows host that runs Koppel's container or on the same Windows machine.

use std::collections::BTreeMap;
use std::io;
use std::net::Ipv6Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use time::OffsetDateTime;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use crate::host::{Export, HostKind, UsbHost};
use crate::tasks::lock;
use crate::{
    BoxFuture, Clock, DeviceStream, ExportStatus, UsbDevice, UsbError, bus_id, protocol, relay,
};

/// The most devices Koppel accepts in one list.
pub const MAX_DEVICES: usize = 256;

/// The most bytes Koppel reads for one list.
pub const MAX_LIST_BYTES: usize = 512 * 1024;

const LIST_FOR: Duration = Duration::from_secs(1);
const RETRY_AFTER: Duration = Duration::from_secs(5);
const DEFAULT_DEADLINE: Duration = Duration::from_secs(5);

/// Shares what a plain USB/IP server exports. Koppel stays the only door from the network: it
/// lists what that server exports and, on export, relays the client's session straight to it, so
/// the traffic on the network is still inside the stream to the client. Which devices are shared
/// is decided on that server (`usbipd bind`), not here.
///
/// The device list is a cached snapshot. A background refresh with one deadline keeps it fresh,
/// so no caller waits on the network, and nothing here holds a lock while it does I/O. What the
/// server says is not trusted: the device count and the bytes read for the list are capped.
#[derive(Clone)]
pub struct UsbIpUpstreamHost {
    inner: Arc<Inner>,
}

struct Inner {
    host: String,
    port: u16,
    clock: Arc<dyn Clock>,
    deadline: Duration,
    in_use: Mutex<BTreeMap<String, UsbDevice>>,
    snapshot: Mutex<Option<Arc<Snapshot>>>,
    /// Cancelled once the refresh it belongs to has stored its snapshot.
    refresh: Mutex<Option<CancellationToken>>,
}

struct Snapshot {
    devices: Vec<UsbDevice>,
    unavailable: Option<String>,
    next_refresh: OffsetDateTime,
}

impl UsbIpUpstreamHost {
    /// The USB/IP server at `host`, such as `host.docker.internal`, and `port`.
    pub fn new(host: impl Into<String>, port: u16, clock: Arc<dyn Clock>) -> Self {
        Self::with_deadline(host, port, clock, DEFAULT_DEADLINE)
    }

    /// As [`Self::new`], with another deadline for each look at the server and each import.
    pub fn with_deadline(
        host: impl Into<String>,
        port: u16,
        clock: Arc<dyn Clock>,
        deadline: Duration,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                host: host.into(),
                port,
                clock,
                deadline,
                in_use: Mutex::new(BTreeMap::new()),
                snapshot: Mutex::new(None),
                refresh: Mutex::new(None),
            }),
        }
    }

    /// What the server exports, plus the devices Koppel is relaying, which it may no longer
    /// list. Returns the cached snapshot and refreshes it in the background; only the very first
    /// call waits for the server.
    async fn devices(&self) -> Vec<UsbDevice> {
        let current = lock(&self.inner.snapshot).clone();
        let snapshot = match current {
            Some(snapshot) => {
                if self.inner.clock.now() >= snapshot.next_refresh {
                    self.start_refresh();
                }
                snapshot
            }
            None => {
                self.start_refresh().cancelled().await;
                lock(&self.inner.snapshot)
                    .clone()
                    .expect("a finished refresh leaves a snapshot")
            }
        };

        let in_use = lock(&self.inner.in_use).clone();
        let mut devices: Vec<UsbDevice> = snapshot
            .devices
            .iter()
            .filter(|d| !in_use.contains_key(&d.bus_id))
            .cloned()
            .collect();
        devices.extend(in_use.into_values());
        devices.sort_by(|a, b| a.bus_id.cmp(&b.bus_id));
        devices
    }

    /// Starts a refresh unless one is running, and returns the one that is.
    fn start_refresh(&self) -> CancellationToken {
        let mut refresh = lock(&self.inner.refresh);
        if let Some(running) = refresh.as_ref()
            && !running.is_cancelled()
        {
            return running.clone();
        }
        let done = CancellationToken::new();
        *refresh = Some(done.clone());
        drop(refresh);

        let inner = self.inner.clone();
        let finished = done.clone();
        tokio::spawn(async move {
            let snapshot = inner.list_upstream().await;
            *lock(&inner.snapshot) = Some(Arc::new(snapshot));
            finished.cancel();
        });
        done
    }
}

impl Inner {
    /// One look at the server, under one deadline for the connect, the request and the whole reply.
    async fn list_upstream(&self) -> Snapshot {
        let look = async {
            let mut stream = self.connect().await?;
            stream.write_all(&protocol::dev_list_request()).await?;
            let reply = protocol::read_header(&mut stream).await?;
            if reply.code != protocol::OP_REP_DEVLIST || reply.status != protocol::STATUS_OK {
                return Err(io::Error::other(
                    "it answered the device list request with an error",
                ));
            }
            protocol::read_dev_list(&mut stream, MAX_DEVICES, MAX_LIST_BYTES).await
        };
        let why = match tokio::time::timeout(self.deadline, look).await {
            Ok(Ok(devices)) => {
                // Bus IDs come from another machine and end up in the UI and the logs.
                return Snapshot {
                    devices: devices
                        .into_iter()
                        .filter(|d| !d.is_hub() && bus_id::is_valid(&d.bus_id))
                        .collect(),
                    unavailable: None,
                    next_refresh: self.clock.now() + LIST_FOR,
                };
            }
            Ok(Err(e)) => e.to_string(),
            Err(_) => "it did not answer in time".to_owned(),
        };
        Snapshot {
            devices: Vec::new(),
            unavailable: Some(format!(
                "Cannot reach the USB/IP server on {}:{} ({why}). On Windows, install usbipd-win with 'winget install usbipd' and share devices with 'usbipd bind --busid <id>'.",
                self.host, self.port
            )),
            next_refresh: self.clock.now() + RETRY_AFTER,
        }
    }

    async fn connect(&self) -> io::Result<TcpStream> {
        let stream = TcpStream::connect((self.host.as_str(), self.port)).await?;
        stream.set_nodelay(true)?;
        Ok(stream)
    }

    /// Connects and imports `bus_id` under the deadline. Returns the connection, ready for traffic.
    async fn import(&self, bus_id: &str) -> Result<TcpStream, UsbError> {
        let handshake = async {
            let mut upstream = self.connect().await?;
            upstream
                .write_all(&protocol::import_request(bus_id))
                .await?;
            let reply = protocol::read_header(&mut upstream).await?;
            if reply.code != protocol::OP_REP_IMPORT || reply.status != protocol::STATUS_OK {
                return Ok(Err(UsbError::new(format!(
                    "{} would not export {bus_id}. Another client may be using it.",
                    self.host
                ))));
            }
            protocol::read_imported_device(&mut upstream).await?;
            Ok::<_, io::Error>(Ok(upstream))
        };
        let why = match tokio::time::timeout(self.deadline, handshake).await {
            Ok(Ok(result)) => return result,
            Ok(Err(e)) => e.to_string(),
            Err(_) => "it did not answer in time".to_owned(),
        };
        Err(UsbError::new(format!(
            "Could not import {bus_id} from {}:{}: {why}",
            self.host, self.port
        )))
    }
}

/// The import of one device, holding its claim until the connection to the server is closed.
struct UpstreamExport {
    upstream: Option<TcpStream>,
    claim: Claim,
}

/// Takes the bus ID out of the in-use list when dropped.
struct Claim {
    inner: Arc<Inner>,
    bus_id: String,
}

impl Drop for Claim {
    fn drop(&mut self) {
        lock(&self.inner.in_use).remove(&self.bus_id);
    }
}

impl Export for UpstreamExport {
    fn relay<'a>(
        mut self: Box<Self>,
        network: &'a mut dyn DeviceStream,
        stop: CancellationToken,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            // USB traffic can be quiet for a long time; only the handshake has a deadline.
            if let Some(upstream) = self.upstream.take() {
                relay(network, upstream, &stop).await;
            }
            // The connection is closed above, before the claim goes, so a second import never
            // meets a half-closed first one.
            drop(self.claim);
        })
    }
}

impl UsbHost for UsbIpUpstreamHost {
    fn unavailable(&self) -> Option<String> {
        lock(&self.inner.snapshot)
            .as_ref()
            .and_then(|s| s.unavailable.clone())
    }

    fn can_share(&self) -> bool {
        false
    }

    fn can_unshare(&self) -> bool {
        false
    }

    fn can_reset(&self) -> bool {
        false
    }

    fn kind(&self) -> HostKind {
        HostKind::Upstream
    }

    fn list_devices(&self) -> BoxFuture<'_, Result<Vec<UsbDevice>, UsbError>> {
        Box::pin(async move { Ok(self.devices().await) })
    }

    /// The server only lists devices it exports, so everything listed is shared.
    fn is_shared(&self, _: &UsbDevice) -> bool {
        true
    }

    fn sharing_advice(&self, bus_id: &str, share: bool) -> String {
        let host = &self.inner.host;
        if share {
            format!(
                "{bus_id} is not shared on {host}. On Windows, run 'usbipd bind --busid {bus_id}' as administrator."
            )
        } else {
            format!(
                "Stop sharing it on {host}. On Windows, run 'usbipd unbind --busid {bus_id}' as administrator."
            )
        }
    }

    fn share<'a>(&'a self, bus_id: &'a str) -> BoxFuture<'a, Result<(), UsbError>> {
        let advice = self.sharing_advice(bus_id, true);
        Box::pin(async move { Err(UsbError::new(advice)) })
    }

    fn unshare<'a>(&'a self, bus_id: &'a str) -> BoxFuture<'a, Result<(), UsbError>> {
        let advice = self.sharing_advice(bus_id, false);
        Box::pin(async move { Err(UsbError::new(advice)) })
    }

    fn status<'a>(&'a self, bus_id: &'a str) -> BoxFuture<'a, Result<ExportStatus, UsbError>> {
        Box::pin(async move {
            if lock(&self.inner.in_use).contains_key(bus_id) {
                return Ok(ExportStatus::Used);
            }
            Ok(if self.devices().await.iter().any(|d| d.bus_id == bus_id) {
                ExportStatus::Available
            } else {
                ExportStatus::NotExported
            })
        })
    }

    fn reset<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<(), UsbError>> {
        Box::pin(async { Ok(()) })
    }

    /// Claims the device, then imports it from the server. The claim comes first, so two imports
    /// of one device cannot both connect.
    fn export<'a>(
        &'a self,
        device: &'a UsbDevice,
    ) -> BoxFuture<'a, Result<Box<dyn Export>, UsbError>> {
        Box::pin(async move {
            let bus_id = bus_id::require(&device.bus_id)?.to_owned();
            {
                let mut in_use = lock(&self.inner.in_use);
                if in_use.contains_key(&bus_id) {
                    return Err(UsbError::new(format!(
                        "{bus_id} is already being used through this server."
                    )));
                }
                in_use.insert(bus_id.clone(), device.clone());
            }
            let claim = Claim {
                inner: self.inner.clone(),
                bus_id: bus_id.clone(),
            };
            let upstream = self.inner.import(&bus_id).await?;
            Ok(Box::new(UpstreamExport {
                upstream: Some(upstream),
                claim,
            }) as Box<dyn Export>)
        })
    }
}

/// The USB/IP server behind `Usb:Upstream`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamAddress {
    pub host: String,
    pub port: u16,
}

impl UpstreamAddress {
    pub const SETTING: &str = "Usb:Upstream";

    /// USB/IP's usual port.
    pub const DEFAULT_PORT: u16 = 3240;

    /// Accepts `host`, `host:port`, `[v6]:port`, `[v6]` and a bare IPv6 address; the port
    /// defaults to 3240. A value that is none of those is an error that names the setting.
    pub fn parse(value: &str) -> Result<Self, String> {
        let text = value.trim();
        let bad = |why: &str| {
            format!(
                "{} is \"{value}\": {why}. Use host, host:port or [v6]:port.",
                Self::SETTING
            )
        };
        let new = |host: &str, port: u16| Self {
            host: host.to_owned(),
            port,
        };

        if let Some(rest) = text.strip_prefix('[') {
            let Some(close) = rest.find(']') else {
                return Err(bad("the text in [ ] is not an IPv6 address"));
            };
            let address = &rest[..close];
            if address.parse::<Ipv6Addr>().is_err() {
                return Err(bad("the text in [ ] is not an IPv6 address"));
            }
            let after = &rest[close + 1..];
            return if after.is_empty() {
                Ok(new(address, Self::DEFAULT_PORT))
            } else {
                Ok(new(
                    address,
                    port_after_colon(after).ok_or_else(|| bad(PORT_RULE))?,
                ))
            };
        }

        let colons = text.matches(':').count();
        if colons > 1 {
            // Several colons without brackets can only be a bare IPv6 address, with no room for a port.
            return if text.parse::<Ipv6Addr>().is_ok() {
                Ok(new(text, Self::DEFAULT_PORT))
            } else {
                Err(bad("put an IPv6 address in [ ] when it has a port"))
            };
        }

        let host = text.split(':').next().unwrap_or_default();
        if host.is_empty() || host.chars().any(char::is_whitespace) {
            return Err(bad("the host is missing"));
        }
        if colons == 1 {
            let port = port_after_colon(&text[host.len()..]).ok_or_else(|| bad(PORT_RULE))?;
            Ok(new(host, port))
        } else {
            Ok(new(host, Self::DEFAULT_PORT))
        }
    }
}

const PORT_RULE: &str = "the port must be a number from 1 to 65535";

fn port_after_colon(colon_and_port: &str) -> Option<u16> {
    let digits = colon_and_port.strip_prefix(':')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u16>().ok().filter(|&port| port >= 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_host_and_port() {
        for (value, host, port) in [
            ("usbipd.lab", "usbipd.lab", 3240),
            ("usbipd.lab:3241", "usbipd.lab", 3241),
            ("192.168.1.20:4000", "192.168.1.20", 4000),
            ("[::1]:3241", "::1", 3241),
            ("[fe80::1]", "fe80::1", 3240),
            ("fe80::1", "fe80::1", 3240),
            ("  host:65535 ", "host", 65535),
        ] {
            assert_eq!(
                UpstreamAddress::parse(value),
                Ok(UpstreamAddress {
                    host: host.into(),
                    port
                }),
                "{value}"
            );
        }
    }

    #[test]
    fn parse_rejects_a_bad_value_and_names_the_setting() {
        for value in [
            "host:",
            "host:abc",
            "host:0",
            "host:65536",
            ":3240",
            "[::1",
            "[host]:3240",
            "[::1]3240",
            "::1:3240x",
            "",
        ] {
            let error = UpstreamAddress::parse(value).unwrap_err();
            assert!(error.contains("Usb:Upstream"), "{value}: {error}");
        }
    }
}
