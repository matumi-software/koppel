//! `GET /readyz`: whether Koppel can do what it is set up to do, for a person or a monitor to ask.
//! `/healthz` stays liveness for the container's health check and systemd's watchdog: it answers
//! `ok` as long as the process serves requests. This one looks at the parts that can fail while the
//! process is up: the USB gateway (a port that is taken), the certificate the web ports present, and
//! the link to the server.
//!
//! It follows the rules of `/healthz`: on the UI port behind `Ui:AllowFrom`, anonymous, the same in
//! any role, no header needed. Because anyone it lets through can read it, it names states and
//! never messages, addresses, fingerprints or file names: the reason for a fault is in the UI (the
//! devices page) and in the log.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use koppel_usb::{ClientDevices, LinkState};
use serde::Serialize;

use crate::state::ReadyApi;

/// The plain USB/IP gateway, which only a container on Windows turns on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum GatewayState {
    /// Not in use here.
    Off,
    Listening,
    /// It should be listening and is not, as when its port is taken.
    Faulted,
}

/// The certificate the web ports present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CertificateState {
    Loaded,
    Missing,
}

/// What `/readyz` answers with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Readiness {
    pub ready: bool,
    pub gateway: GatewayState,
    pub certificate: CertificateState,
    /// The link to the saved server. It is reported but does not decide `ready`: a client whose
    /// server is away is working, and a monitor can read this field when it cares.
    pub link: LinkState,
}

impl Readiness {
    pub(crate) fn of(devices: &ClientDevices, certificate_loaded: bool) -> Self {
        let gateway = if devices.gateway_fault.is_some() {
            GatewayState::Faulted
        } else if devices.gateway {
            GatewayState::Listening
        } else {
            GatewayState::Off
        };
        let certificate = if certificate_loaded {
            CertificateState::Loaded
        } else {
            CertificateState::Missing
        };
        Self {
            ready: gateway != GatewayState::Faulted && certificate == CertificateState::Loaded,
            gateway,
            certificate,
            link: devices.link,
        }
    }
}

/// 200 when Koppel is ready, 503 when it is not, with the states either way.
pub(crate) async fn readyz(State(app): State<ReadyApi>) -> Response {
    let loaded = !app.certificate.current().cert.is_empty();
    let readiness = Readiness::of(&app.usb_client.current(), loaded);
    let status = if readiness.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(readiness)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices(link: LinkState, gateway: bool, fault: Option<&str>) -> ClientDevices {
        ClientDevices {
            link,
            link_message: Some("a message that must not leave".into()),
            devices: Vec::new(),
            unavailable: None,
            gateway,
            gateway_fault: fault.map(str::to_owned),
        }
    }

    #[test]
    fn ready_when_the_gateway_is_not_faulted_and_a_certificate_is_loaded() {
        for (devices, gateway) in [
            (
                devices(LinkState::Unconfigured, false, None),
                GatewayState::Off,
            ),
            (
                devices(LinkState::Connected, true, None),
                GatewayState::Listening,
            ),
        ] {
            let readiness = Readiness::of(&devices, true);
            assert!(readiness.ready);
            assert_eq!(readiness.gateway, gateway);
        }
    }

    #[test]
    fn a_gateway_fault_or_a_missing_certificate_is_not_ready() {
        let faulted = Readiness::of(
            &devices(LinkState::Connected, false, Some("port taken")),
            true,
        );
        assert!(!faulted.ready);
        assert_eq!(faulted.gateway, GatewayState::Faulted);
        let no_certificate = Readiness::of(&devices(LinkState::Connected, false, None), false);
        assert!(!no_certificate.ready);
        assert_eq!(no_certificate.certificate, CertificateState::Missing);
    }

    #[test]
    fn a_link_that_is_down_is_reported_and_does_not_decide() {
        let readiness = Readiness::of(&devices(LinkState::Error, false, None), true);
        assert!(readiness.ready);
        assert_eq!(readiness.link, LinkState::Error);
    }

    #[test]
    fn the_answer_has_states_only() {
        let json = serde_json::to_value(Readiness::of(
            &devices(LinkState::Error, true, Some("secret reason")),
            true,
        ))
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "ready": false,
                "gateway": "faulted",
                "certificate": "loaded",
                "link": "error",
            })
        );
    }
}
