//! The server's shared devices for its clients: Koppel's own gRPC service `koppel.remote.v1`
//! (`proto/koppel/remote/v1/devices.proto`), which takes the place of 0.1.x's hub message
//! `DevicesChanged`, and `GET /api/remote/devices`, which 0.1.x served behind a client's token too.
//! Both show the remote view: the shared devices without who holds them.

use std::pin::Pin;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::http::StatusCode;
use axum::http::header::{HeaderValue, WWW_AUTHENTICATE};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::Stream;
use koppel_usb::{DeviceState, DeviceView, RemoteDevices};
use skakel_server::ClientPrincipal;
use tokio::sync::watch;
use tonic::{Request, Status};

/// The code generated from `devices.proto`.
#[allow(clippy::all, clippy::pedantic)]
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/koppel.remote.v1.rs"));
}

use proto::remote_devices_server::{RemoteDevices as RemoteDevicesService, RemoteDevicesServer};
use proto::{DeviceList, InterfaceList, RemoteDevice, WatchDevicesRequest};

/// The service's name as a gRPC path starts with it, `/koppel.remote.v1.RemoteDevices/Watch`: how the
/// web ports tell a call to it from a request for the UI (see `surface.rs`).
pub const SERVICE_NAME: &str = <RemoteDevicesServer<Service> as tonic::server::NamedService>::NAME;

/// The remote view as it changes, shared by every call and request.
pub type Shared = watch::Receiver<Arc<RemoteDevices>>;

/// The service Skakel's server runs for its clients.
#[derive(Clone)]
pub struct Service {
    devices: Shared,
}

impl Service {
    pub fn server(devices: Shared) -> RemoteDevicesServer<Self> {
        RemoteDevicesServer::new(Self { devices })
    }
}

type Lists = Pin<Box<dyn Stream<Item = Result<DeviceList, Status>> + Send>>;

#[tonic::async_trait]
impl RemoteDevicesService for Service {
    type WatchStream = Lists;

    /// The list now, then each new one. Skakel ends the call when the client is revoked, its secret
    /// rotated or the server stops.
    async fn watch(
        &self,
        _request: Request<WatchDevicesRequest>,
    ) -> Result<tonic::Response<Self::WatchStream>, Status> {
        let mut devices = self.devices.clone();
        devices.mark_changed();
        let lists = futures_util::stream::unfold(devices, |mut devices| async move {
            devices.changed().await.ok()?;
            let list = to_proto(&devices.borrow_and_update());
            Some((Ok(list), devices))
        });
        Ok(tonic::Response::new(Box::pin(lists)))
    }
}

/// `GET /api/remote/devices`, for other Koppel machines with a token from Skakel's token endpoint,
/// answered as 0.1.x answered it: 401 with `WWW-Authenticate: Bearer` without a current token.
pub fn routes(devices: Shared) -> Router {
    Router::new().route(
        "/api/remote/devices",
        get(move |caller: Result<ClientPrincipal, Response>| {
            let devices = devices.clone();
            async move {
                if caller.is_err() {
                    return (
                        StatusCode::UNAUTHORIZED,
                        [(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"))],
                    )
                        .into_response();
                }
                let view = devices.borrow().clone();
                Json(&*view).into_response()
            }
        }),
    )
}

/// The remote view as the service sends it.
pub fn to_proto(devices: &RemoteDevices) -> DeviceList {
    DeviceList {
        devices: devices
            .devices
            .iter()
            .map(|d| RemoteDevice {
                bus_id: d.bus_id.to_string(),
                vid_pid: d.vid_pid.clone(),
                name: d.name.clone(),
                manufacturer: d.manufacturer.clone(),
                speed: d.speed.clone(),
                state: d.state.as_str().to_owned(),
                interfaces: d.interfaces.as_ref().map(|names| InterfaceList {
                    names: names.clone(),
                }),
            })
            .collect(),
    }
}

/// A list the service sent, as the client's USB layer takes it.
pub fn from_proto(list: DeviceList) -> RemoteDevices {
    RemoteDevices {
        // A row whose bus ID is not one is left out.
        devices: list
            .devices
            .into_iter()
            .filter_map(|d| {
                Some(DeviceView {
                    vid_pid: d.vid_pid,
                    name: d.name,
                    manufacturer: d.manufacturer,
                    speed: d.speed,
                    state: DeviceState::parse(&d.state),
                    interfaces: d.interfaces.map(|i| i.names),
                    ..DeviceView::new(d.bus_id.parse().ok()?)
                })
            })
            .collect(),
    }
}

/// True for the path of a call to a gRPC service this server runs: `/<package>.<Service>/<Method>`,
/// with a package under `skakel` (Skakel's own services) or Koppel's `koppel.remote.v1.RemoteDevices`.
/// A request that only says it is gRPC, by a content type, is not one. No UI path has a dot in its
/// first segment, so a UI path is never taken for one.
pub fn is_grpc_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    let (service, method) = rest.split_once('/').unwrap_or((rest, ""));
    !method.is_empty() && (service.starts_with("skakel.") || service == SERVICE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_crosses_the_wire_unchanged() {
        let devices = RemoteDevices {
            devices: vec![
                DeviceView {
                    vid_pid: "064f:2af9".into(),
                    name: "CodeMeter-Stick".into(),
                    manufacturer: Some("WIBU-SYSTEMS AG".into()),
                    speed: "USB 1.1 (12 Mbit/s)".into(),
                    state: DeviceState::Shared,
                    interfaces: Some(vec!["Vendor specific".into()]),
                    ..DeviceView::new("1-1.4".parse().unwrap())
                },
                DeviceView {
                    state: DeviceState::Attached,
                    interfaces: Some(Vec::new()),
                    ..DeviceView::new("2-1".parse().unwrap())
                },
            ],
        };

        assert_eq!(from_proto(to_proto(&devices)), devices);
    }
}
