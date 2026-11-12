//! The server and client roles on Skakel 0.3.0 (protocol version 3, gRPC).
//!
//! | Module | What |
//! |---|---|
//! | `store` | `secrets.json` as skakel-server's store, and the certificate the web ports present |
//! | `server` | [`SkakelServerRole`]: skakel-server mounted on Koppel's web ports, its clients, keys and pairing |
//! | `announcer` | Discovery answers while the role is server or both |
//! | `remote` | Koppel's own gRPC service, the remote device list, and `/api/remote/devices` |
//! | `streams` | The `usb` stream type as koppel-usb's `DeviceStream`, both ends |
//! | `client` | [`SkakelClientRole`]: the connection test, discovery and pairing |
//! | `links` | The link to the saved server as koppel-usb's `Links`, and what it saves |

pub(crate) mod announcer;
mod client;
mod links;
pub(crate) mod remote;
mod server;
mod store;
mod streams;

pub use client::SkakelClientRole;
pub(crate) use links::SkakelLinks;
pub use server::SkakelServerRole;
pub(crate) use server::{APP_ID, ServerParts, ServerStartError};
pub(crate) use store::{SecretsError, SecretsStore};
