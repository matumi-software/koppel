//! Sharing the devices of a plain USB/IP server next to Koppel instead of this kernel's: usually
//! usbipd-win, on the Windows host that runs Koppel's container or on the same Windows machine.

use std::collections::BTreeMap;
use std::io;
use std::net::Ipv6Addr;

use http::uri::Authority;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use crate::host::{Capabilities, Export, UsbHost};
use crate::tasks::lock;
use crate::{BoxFuture, BusId, Clock, DeviceStream, ExportStatus, UsbDevice, UsbError, protocol};

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
    in_use: Mutex<BTreeMap<BusId, UsbDevice>>,
    snapshot: Mutex<Option<Arc<Snapshot>>>,
    /// Cancelled once the refresh it belongs to has stored its snapshot.
    refresh: Mutex<Option<CancellationToken>>,
}

struct Snapshot {
    devices: Vec<UsbDevice>,
    unavailable: Option<String>,
    next_refresh: tokio::time::Instant,
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
                if self.inner.clock.monotonic() >= snapshot.next_refresh {
                    self.start_refresh();
                }
                snapshot
            }
            None => {
                self.start_refresh().cancelled().await;
                // A refresh that panicked leaves nothing; the host then reads as not reachable.
                lock(&self.inner.snapshot)
                    .clone()
                    .unwrap_or_else(|| Arc::new(self.inner.failed("the check did not finish")))
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
            // Whatever happens below, even a panic, whoever waits for this refresh is let go.
            let _done = Finished(finished);
            let snapshot = inner.list_upstream().await;
            *lock(&inner.snapshot) = Some(Arc::new(snapshot));
        });
        done
    }
}

/// Cancels its token when dropped.
struct Finished(CancellationToken);

impl Drop for Finished {
    fn drop(&mut self) {
        self.0.cancel();
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
                // The reader already left out any device whose bus ID is not one.
                return Snapshot {
                    devices: devices.into_iter().filter(|d| !d.is_hub()).collect(),
                    unavailable: None,
                    next_refresh: self.clock.monotonic() + LIST_FOR,
                };
            }
            Ok(Err(e)) => e.to_string(),
            Err(_) => "it did not answer in time".to_owned(),
        };
        self.failed(&why)
    }

    /// A snapshot that says the server cannot be reached, and when to look again.
    fn failed(&self, why: &str) -> Snapshot {
        Snapshot {
            devices: Vec::new(),
            unavailable: Some(format!(
                "Cannot reach the USB/IP server on {}:{} ({why}). On Windows, install usbipd-win with 'winget install usbipd' and share devices with 'usbipd bind --busid <id>'.",
                self.host, self.port
            )),
            next_refresh: self.clock.monotonic() + RETRY_AFTER,
        }
    }

    async fn connect(&self) -> io::Result<TcpStream> {
        let stream = TcpStream::connect((self.host.as_str(), self.port)).await?;
        stream.set_nodelay(true)?;
        Ok(stream)
    }

    /// Connects and imports `bus_id` under the deadline. Returns the connection, ready for traffic.
    async fn import(&self, bus_id: &BusId) -> Result<TcpStream, UsbError> {
        let handshake = async {
            let mut upstream = self.connect().await?;
            upstream
                .write_all(&protocol::import_request(bus_id))
                .await?;
            let reply = protocol::read_header(&mut upstream).await?;
            if reply.code != protocol::OP_REP_IMPORT || reply.status != protocol::STATUS_OK {
                return Ok(Err(UsbError::failed(format!(
                    "{} would not export {bus_id}. Another client may be using it.",
                    self.host
                ))));
            }
            let record = protocol::read_imported_device(&mut upstream).await?;
            // The reply must describe the device that was asked for.
            if record.bus_id != *bus_id {
                return Ok(Err(UsbError::failed(format!(
                    "{} answered the import of {bus_id} with a different device ({}).",
                    self.host, record.bus_id
                ))));
            }
            Ok::<_, io::Error>(Ok(upstream))
        };
        let why = match tokio::time::timeout(self.deadline, handshake).await {
            Ok(Ok(result)) => return result,
            Ok(Err(e)) => e.to_string(),
            Err(_) => "it did not answer in time".to_owned(),
        };
        Err(UsbError::failed(format!(
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
    bus_id: BusId,
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
                usbip::relay(network, upstream, stop.cancelled()).await;
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

    fn capabilities(&self) -> Capabilities {
        Capabilities::UPSTREAM
    }

    fn list_devices(&self) -> BoxFuture<'_, Result<Vec<UsbDevice>, UsbError>> {
        Box::pin(async move { Ok(self.devices().await) })
    }

    /// The server only lists devices it exports, so everything listed is shared.
    fn is_shared(&self, _: &UsbDevice) -> bool {
        true
    }

    fn sharing_advice(&self, bus_id: &BusId, share: bool) -> String {
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

    fn share<'a>(&'a self, bus_id: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>> {
        let advice = self.sharing_advice(bus_id, true);
        Box::pin(async move { Err(UsbError::failed(advice)) })
    }

    fn unshare<'a>(&'a self, bus_id: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>> {
        let advice = self.sharing_advice(bus_id, false);
        Box::pin(async move { Err(UsbError::failed(advice)) })
    }

    fn status<'a>(&'a self, bus_id: &'a BusId) -> BoxFuture<'a, Result<ExportStatus, UsbError>> {
        Box::pin(async move {
            if lock(&self.inner.in_use).contains_key(bus_id) {
                return Ok(ExportStatus::Used);
            }
            Ok(
                if self.devices().await.iter().any(|d| d.bus_id == *bus_id) {
                    ExportStatus::Available
                } else {
                    ExportStatus::NotExported
                },
            )
        })
    }

    fn reset<'a>(&'a self, _: &'a BusId) -> BoxFuture<'a, Result<(), UsbError>> {
        Box::pin(async { Ok(()) })
    }

    /// Claims the device, then imports it from the server. The claim comes first, so two imports
    /// of one device cannot both connect.
    fn export<'a>(
        &'a self,
        device: &'a UsbDevice,
    ) -> BoxFuture<'a, Result<Box<dyn Export>, UsbError>> {
        Box::pin(async move {
            let bus_id = device.bus_id.clone();
            {
                let mut in_use = lock(&self.inner.in_use);
                if in_use.contains_key(&bus_id) {
                    return Err(UsbError::Claimed(bus_id));
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

        // A bare IPv6 address has no room for a port, and `Authority` wants it in brackets.
        if text.parse::<Ipv6Addr>().is_ok() {
            return Ok(new(text, Self::DEFAULT_PORT));
        }
        if text.contains('@') {
            return Err(bad("there is no room for a user name"));
        }
        let authority: Authority = text.parse().map_err(|_| {
            if text.matches(':').count() > 1 && !text.starts_with('[') {
                bad("put an IPv6 address in [ ] when it has a port")
            } else if text.starts_with('[') {
                bad("the text in [ ] is not an IPv6 address")
            } else {
                bad("it is not a host with a port")
            }
        })?;
        let host = authority.host();
        let bare = host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .map(|inner| {
                inner
                    .parse::<Ipv6Addr>()
                    .map(|_| inner)
                    .map_err(|_| bad("the text in [ ] is not an IPv6 address"))
            })
            .transpose()?
            .unwrap_or(host);
        if bare.is_empty() || bare.chars().any(char::is_whitespace) {
            return Err(bad("the host is missing"));
        }
        let after_host = &text[host.len()..];
        if after_host.is_empty() {
            return Ok(new(bare, Self::DEFAULT_PORT));
        }
        let port = port_after_colon(after_host).ok_or_else(|| bad(PORT_RULE))?;
        Ok(new(bare, port))
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

    #[tokio::test]
    async fn a_refresh_that_panics_still_lets_the_ones_waiting_for_it_go() {
        let done = CancellationToken::new();
        let finished = done.clone();

        let refresh = tokio::spawn(async move {
            let _done = Finished(finished);
            panic!("on purpose");
        });

        assert!(refresh.await.unwrap_err().is_panic());
        assert!(done.is_cancelled());
    }

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
