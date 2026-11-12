//! The USB/IP operation messages (device list and import), version 1.1.1, as documented in the
//! Linux kernel's `Documentation/usb/usbip_protocol.rst`. All fields are big-endian. After a
//! successful import the same connection carries USB traffic, which the kernel or the driver
//! handles itself; Koppel only relays it.

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt};

use crate::bus_id::InvalidBusId;
use crate::{BusId, UsbDevice, UsbInterface, UsbSpeed};

pub const VERSION: u16 = 0x0111;
pub const OP_REQ_DEVLIST: u16 = 0x8005;
pub const OP_REP_DEVLIST: u16 = 0x0005;
pub const OP_REQ_IMPORT: u16 = 0x8003;
pub const OP_REP_IMPORT: u16 = 0x0003;

pub const HEADER_SIZE: usize = 8;
pub const BUS_ID_SIZE: usize = 32;
pub const DEVICE_SIZE: usize = 312;
pub const INTERFACE_SIZE: usize = 4;

/// Status values in a reply header. 0 is success.
pub const STATUS_OK: u32 = 0;
pub const STATUS_ERROR: u32 = 1;

/// The common header of every operation message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpHeader {
    pub version: u16,
    pub code: u16,
    pub status: u32,
}

pub fn parse_header(source: &[u8; HEADER_SIZE]) -> OpHeader {
    OpHeader {
        version: u16::from_be_bytes([source[0], source[1]]),
        code: u16::from_be_bytes([source[2], source[3]]),
        status: u32::from_be_bytes([source[4], source[5], source[6], source[7]]),
    }
}

pub fn header(code: u16, status: u32) -> [u8; HEADER_SIZE] {
    let mut header = [0u8; HEADER_SIZE];
    header[0..2].copy_from_slice(&VERSION.to_be_bytes());
    header[2..4].copy_from_slice(&code.to_be_bytes());
    header[4..8].copy_from_slice(&status.to_be_bytes());
    header
}

pub fn dev_list_request() -> [u8; HEADER_SIZE] {
    header(OP_REQ_DEVLIST, 0)
}

/// OP_REQ_IMPORT: the header, then the bus ID in 32 bytes.
pub fn import_request(bus_id: &BusId) -> [u8; HEADER_SIZE + BUS_ID_SIZE] {
    let mut message = [0u8; HEADER_SIZE + BUS_ID_SIZE];
    message[..HEADER_SIZE].copy_from_slice(&header(OP_REQ_IMPORT, 0));
    write_string(&mut message[HEADER_SIZE..], bus_id.as_str());
    message
}

/// How many interfaces go on the wire for `device`: the record has one byte for the count.
fn interface_count(device: &UsbDevice) -> u8 {
    u8::try_from(device.interfaces.len()).unwrap_or(u8::MAX)
}

/// OP_REP_DEVLIST: the exportable devices, each followed by its interfaces. A device with more
/// interfaces than the record can count (255) is sent with the first 255.
pub fn dev_list_reply(devices: &[UsbDevice]) -> Vec<u8> {
    let count = u32::try_from(devices.len()).unwrap_or(u32::MAX);
    let devices = &devices[..usize::try_from(count).unwrap_or(devices.len())];
    let size: usize = devices
        .iter()
        .map(|d| DEVICE_SIZE + usize::from(interface_count(d)) * INTERFACE_SIZE)
        .sum();
    let mut message = Vec::with_capacity(HEADER_SIZE + 4 + size);
    message.extend_from_slice(&header(OP_REP_DEVLIST, STATUS_OK));
    message.extend_from_slice(&count.to_be_bytes());
    for device in devices {
        message.extend_from_slice(&device_record(device));
        for interface in device
            .interfaces
            .iter()
            .take(usize::from(interface_count(device)))
        {
            message.extend_from_slice(&[
                interface.class,
                interface.sub_class,
                interface.protocol,
                0,
            ]);
        }
    }
    message
}

/// OP_REP_IMPORT. On success the device follows; on failure only the header.
pub fn import_reply(device: Option<&UsbDevice>) -> Vec<u8> {
    let Some(device) = device else {
        return header(OP_REP_IMPORT, STATUS_ERROR).to_vec();
    };
    let mut message = Vec::with_capacity(HEADER_SIZE + DEVICE_SIZE);
    message.extend_from_slice(&header(OP_REP_IMPORT, STATUS_OK));
    message.extend_from_slice(&device_record(device));
    message
}

/// The 312 byte device record. The interface count is the number of interfaces the device
/// lists, at most 255.
pub fn device_record(device: &UsbDevice) -> [u8; DEVICE_SIZE] {
    let mut target = [0u8; DEVICE_SIZE];
    write_string(&mut target[..256], &device.path);
    write_string(&mut target[256..256 + BUS_ID_SIZE], device.bus_id.as_str());
    target[288..292].copy_from_slice(&device.bus_num.to_be_bytes());
    target[292..296].copy_from_slice(&device.dev_num.to_be_bytes());
    target[296..300].copy_from_slice(&device.speed.wire().to_be_bytes());
    target[300..302].copy_from_slice(&device.vendor_id.to_be_bytes());
    target[302..304].copy_from_slice(&device.product_id.to_be_bytes());
    target[304..306].copy_from_slice(&device.bcd_device.to_be_bytes());
    target[306] = device.class;
    target[307] = device.sub_class;
    target[308] = device.protocol;
    target[309] = device.configuration_value;
    target[310] = device.num_configurations;
    target[311] = interface_count(device);
    target
}

/// Reads a 312 byte device record. The interfaces are not part of it, so the device has none.
/// A record whose bus ID is not one is an error: it came from another machine.
pub fn read_device(source: &[u8; DEVICE_SIZE]) -> Result<UsbDevice, InvalidBusId> {
    let u32_at = |at: usize| {
        u32::from_be_bytes([source[at], source[at + 1], source[at + 2], source[at + 3]])
    };
    let u16_at = |at: usize| u16::from_be_bytes([source[at], source[at + 1]]);
    let bus_id: BusId = read_string(&source[256..256 + BUS_ID_SIZE]).parse()?;
    Ok(UsbDevice {
        path: read_string(&source[..256]),
        bus_num: u32_at(288),
        dev_num: u32_at(292),
        speed: UsbSpeed::from_wire(u32_at(296)),
        vendor_id: u16_at(300),
        product_id: u16_at(302),
        bcd_device: u16_at(304),
        class: source[306],
        sub_class: source[307],
        protocol: source[308],
        configuration_value: source[309],
        num_configurations: source[310],
        ..UsbDevice::new(bus_id)
    })
}

pub async fn read_header<R: AsyncRead + Unpin + ?Sized>(stream: &mut R) -> io::Result<OpHeader> {
    let mut buffer = [0u8; HEADER_SIZE];
    stream.read_exact(&mut buffer).await?;
    Ok(parse_header(&buffer))
}

/// The bus ID that follows an OP_REQ_IMPORT header, as text: the caller checks it.
pub async fn read_bus_id<R: AsyncRead + Unpin + ?Sized>(stream: &mut R) -> io::Result<String> {
    let mut buffer = [0u8; BUS_ID_SIZE];
    stream.read_exact(&mut buffer).await?;
    Ok(read_string(&buffer))
}

/// The device that follows a successful OP_REP_IMPORT header.
pub async fn read_imported_device<R: AsyncRead + Unpin + ?Sized>(
    stream: &mut R,
) -> io::Result<UsbDevice> {
    let mut buffer = [0u8; DEVICE_SIZE];
    stream.read_exact(&mut buffer).await?;
    read_device(&buffer).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Reads the body of an OP_REP_DEVLIST after its header, from a server that is not trusted to
/// keep it small: more than `max_devices` devices, or more than `max_bytes` bytes in all, is an
/// `InvalidData` error that says so. A record whose bus ID is not one is left out.
pub async fn read_dev_list<R: AsyncRead + Unpin + ?Sized>(
    stream: &mut R,
    max_devices: usize,
    max_bytes: usize,
) -> io::Result<Vec<UsbDevice>> {
    let mut count = [0u8; 4];
    stream.read_exact(&mut count).await?;
    let expected = usize::try_from(u32::from_be_bytes(count)).unwrap_or(usize::MAX);
    if expected > max_devices {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("The server listed {expected} devices; the limit is {max_devices}."),
        ));
    }

    let mut read = count.len();
    let mut devices = Vec::with_capacity(expected);
    let mut buffer = [0u8; DEVICE_SIZE];
    for _ in 0..expected {
        stream.read_exact(&mut buffer).await?;
        let device = read_device(&buffer);
        let mut interfaces = vec![0u8; usize::from(buffer[311]) * INTERFACE_SIZE];
        read += buffer.len() + interfaces.len();
        if read > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("The server's device list is longer than {max_bytes} bytes."),
            ));
        }
        stream.read_exact(&mut interfaces).await?;
        // The interfaces were read either way, so the next record starts where it should.
        if let Ok(mut device) = device {
            device.interfaces = interfaces
                .chunks(INTERFACE_SIZE)
                .map(|i| UsbInterface::new(i[0], i[1], i[2]))
                .collect();
            devices.push(device);
        }
    }
    Ok(devices)
}

/// ASCII, cut short so the terminating zero always fits. Anything outside ASCII becomes `?`.
fn write_string(target: &mut [u8], value: &str) {
    let room = target.len() - 1;
    for (slot, c) in target.iter_mut().take(room).zip(value.chars()) {
        *slot = if c.is_ascii() { c as u8 } else { b'?' };
    }
}

/// Up to the first zero, as ASCII. A byte outside ASCII reads as `?`.
fn read_string(source: &[u8]) -> String {
    let end = source.iter().position(|&b| b == 0).unwrap_or(source.len());
    source[..end]
        .iter()
        .map(|&b| if b.is_ascii() { b as char } else { '?' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ftdi() -> UsbDevice {
        UsbDevice {
            path: "/sys/devices/pci0000:00/0000:00:14.0/usb1/1-1/1-1.4".into(),
            bus_num: 1,
            dev_num: 5,
            speed: UsbSpeed::Full,
            vendor_id: 0x0403,
            product_id: 0x6001,
            bcd_device: 0x0600,
            configuration_value: 1,
            num_configurations: 1,
            interfaces: vec![UsbInterface::new(0xff, 0xff, 0xff)],
            manufacturer: Some("FTDI".into()),
            product: Some("FT232R USB UART".into()),
            serial: Some("A50285BI".into()),
            driver: Some("usbip-host".into()),
            ..UsbDevice::new("1-1.4".parse().unwrap())
        }
    }

    #[test]
    fn import_request_is_header_plus_a_32_byte_bus_id() {
        let request = import_request(&"1-1.4".parse().unwrap());

        assert_eq!(request.len(), 40);
        assert_eq!(u16::from_be_bytes([request[0], request[1]]), 0x0111);
        assert_eq!(u16::from_be_bytes([request[2], request[3]]), 0x8003);
        assert_eq!(&request[8..13], b"1-1.4");
        assert_eq!(request[13], 0);
    }

    #[tokio::test]
    async fn import_reply_round_trips_the_device_fields() {
        let reply = import_reply(Some(&ftdi()));
        assert_eq!(reply.len(), 8 + 312);
        let mut stream = reply.as_slice();

        let header = read_header(&mut stream).await.unwrap();
        let device = read_imported_device(&mut stream).await.unwrap();

        assert_eq!(
            header,
            OpHeader {
                version: 0x0111,
                code: 0x0003,
                status: 0
            }
        );
        assert_eq!(device.bus_id, "1-1.4");
        assert_eq!(device.path, ftdi().path);
        assert_eq!(device.dev_id(), (1 << 16) | 5);
        assert_eq!(device.speed, UsbSpeed::Full);
        assert_eq!(device.vid_pid(), "0403:6001");
        assert_eq!(device.configuration_value, 1);
    }

    #[test]
    fn devlist_reply_counts_devices_and_carries_their_interfaces() {
        let reply = dev_list_reply(&[ftdi()]);

        assert_eq!(reply.len(), 8 + 4 + 312 + 4);
        assert_eq!(
            u32::from_be_bytes([reply[8], reply[9], reply[10], reply[11]]),
            1
        );
        assert_eq!(reply[8 + 4 + 311], 1, "bNumInterfaces");
        assert_eq!(reply[8 + 4 + 312], 0xff, "bInterfaceClass");
    }

    #[tokio::test]
    async fn devlist_reply_reads_back_with_its_interfaces() {
        let reply = dev_list_reply(&[ftdi()]);
        let mut stream = &reply[HEADER_SIZE..];

        let devices = read_dev_list(&mut stream, 10, 4096).await.unwrap();

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].interfaces, ftdi().interfaces);
    }

    #[tokio::test]
    async fn a_devlist_over_its_limits_is_refused() {
        let reply = dev_list_reply(&[ftdi(), ftdi()]);

        let too_many = read_dev_list(&mut &reply[HEADER_SIZE..], 1, 4096).await;
        let too_long = read_dev_list(&mut &reply[HEADER_SIZE..], 10, 400).await;

        assert!(too_many.unwrap_err().to_string().contains("limit is 1"));
        assert!(
            too_long
                .unwrap_err()
                .to_string()
                .contains("longer than 400 bytes")
        );
    }

    #[test]
    fn failed_import_reply_is_only_a_header_with_an_error_status() {
        let reply = import_reply(None);

        assert_eq!(reply.len(), 8);
        assert_eq!(
            u32::from_be_bytes([reply[4], reply[5], reply[6], reply[7]]),
            1
        );
    }

    #[test]
    fn a_string_too_long_for_its_field_keeps_its_terminator() {
        let record = device_record(&UsbDevice {
            path: "1".repeat(400),
            ..ftdi()
        });

        assert_eq!(&record[..255], "1".repeat(255).as_bytes());
        assert_eq!(record[255], 0);
    }

    #[test]
    fn more_interfaces_than_the_record_can_count_are_cut_at_255() {
        let wide = UsbDevice {
            interfaces: vec![UsbInterface::new(1, 2, 3); 300],
            ..ftdi()
        };

        let reply = dev_list_reply(&[wide]);

        assert_eq!(reply[8 + 4 + 311], 255);
        assert_eq!(reply.len(), 8 + 4 + 312 + 255 * 4);
    }

    #[tokio::test]
    async fn a_record_with_a_bad_bus_id_is_left_out_and_the_next_one_still_reads() {
        let mut reply = dev_list_reply(&[ftdi(), ftdi()]);
        // Spoil the first record's bus ID.
        let at = HEADER_SIZE + 4 + 256;
        reply[at..at + 6].copy_from_slice(b"1-1 --");

        let devices = read_dev_list(&mut &reply[HEADER_SIZE..], 10, 4096)
            .await
            .unwrap();

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].bus_id, "1-1.4");
        assert_eq!(devices[0].interfaces, ftdi().interfaces);
    }

    mod properties {
        use std::future::Future;

        use proptest::prelude::*;

        use super::*;

        fn bus_id() -> impl Strategy<Value = BusId> {
            (1u32..100, prop::collection::vec(1u32..30, 1..4)).prop_map(|(bus, ports)| {
                let ports: Vec<String> = ports.iter().map(u32::to_string).collect();
                format!("{bus}-{}", ports.join(".")).parse().unwrap()
            })
        }

        fn device() -> impl Strategy<Value = UsbDevice> {
            (
                bus_id(),
                "[ -~]{0,120}",
                (any::<u32>(), any::<u32>(), 0u32..7),
                (any::<u16>(), any::<u16>(), any::<u16>()),
                (
                    any::<u8>(),
                    any::<u8>(),
                    any::<u8>(),
                    any::<u8>(),
                    any::<u8>(),
                ),
                prop::collection::vec((any::<u8>(), any::<u8>(), any::<u8>()), 0..6),
            )
                .prop_map(
                    |(
                        bus_id,
                        path,
                        (bus_num, dev_num, speed),
                        (vendor_id, product_id, bcd_device),
                        (class, sub_class, protocol, configuration_value, num_configurations),
                        interfaces,
                    )| UsbDevice {
                        path,
                        bus_num,
                        dev_num,
                        speed: UsbSpeed::from_wire(speed),
                        vendor_id,
                        product_id,
                        bcd_device,
                        class,
                        sub_class,
                        protocol,
                        configuration_value,
                        num_configurations,
                        interfaces: interfaces
                            .into_iter()
                            .map(|(c, s, p)| UsbInterface::new(c, s, p))
                            .collect(),
                        ..UsbDevice::new(bus_id)
                    },
                )
        }

        fn run<T>(work: impl Future<Output = T>) -> T {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(work)
        }

        proptest! {
            #[test]
            fn a_device_list_reads_back_as_it_was_written(
                devices in prop::collection::vec(device(), 0..5)
            ) {
                let reply = dev_list_reply(&devices);
                let read = run(read_dev_list(&mut &reply[HEADER_SIZE..], 16, 1 << 20)).unwrap();
                prop_assert_eq!(read, devices);
            }

            #[test]
            fn an_import_record_reads_back_without_its_interfaces(device in device()) {
                let read = read_device(&device_record(&device)).unwrap();
                prop_assert_eq!(&read, &UsbDevice { interfaces: Vec::new(), ..device });
            }

            #[test]
            fn any_record_bytes_read_as_a_device_or_an_error_and_never_panic(
                bytes in prop::collection::vec(any::<u8>(), DEVICE_SIZE)
            ) {
                let record: [u8; DEVICE_SIZE] = bytes.try_into().unwrap();
                let _ = read_device(&record);
            }

            #[test]
            fn any_bytes_read_as_a_device_list_or_an_error_within_the_caps(
                bytes in prop::collection::vec(any::<u8>(), 0..2000)
            ) {
                if let Ok(devices) = run(read_dev_list(&mut bytes.as_slice(), 4, 1500)) {
                    prop_assert!(devices.len() <= 4);
                }
            }
        }
    }
}
