//! What is served where. There are two surfaces, each with its own router (see `routes.rs`): the UI
//! (the people who run this machine: the UI's files, `/healthz` and `/api`) and the network (other
//! Koppel machines: `/skakel`, `/api/remote` and the gRPC services). A router answers on the port its
//! listener is on, so which surface a request reaches follows from the port by construction. When the
//! two ports are the same port, [`is_network_path`] says which router a path goes to; it is the only
//! place that names the network surface's paths, and it reads the path alone, never a header.
//!
//! The role a surface needs is a layer on its router (`guard::role_gate`).

use koppel_config::files::Role;

use crate::roles::skakel::remote;

/// Which role a surface or a group of routes belongs to. A machine that is both serves both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoleScope {
    Server,
    Client,
}

impl RoleScope {
    pub(crate) fn served_to(self, role: Role) -> bool {
        match self {
            Self::Server => role != Role::Client,
            Self::Client => role != Role::Server,
        }
    }
}

/// True for a path the network surface serves: Skakel's HTTPS calls under `/skakel`, Koppel's own
/// `/api/remote`, and the gRPC services. Exact and case sensitive, as the routers are.
pub(crate) fn is_network_path(path: &str) -> bool {
    starts_with_segments(path, "/skakel")
        || starts_with_segments(path, "/api/remote")
        || is_grpc_path(path)
}

/// True for the path of a call to a gRPC service this server runs: `/<package>.<Service>/<Method>`,
/// with a package under `skakel` (Skakel's own services) or Koppel's `koppel.remote.v1.RemoteDevices`.
/// A request that only says it is gRPC, by a content type, is not one. No UI path has a dot in its
/// first segment, so a UI path is never taken for one.
pub(crate) fn is_grpc_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    let (service, method) = rest.split_once('/').unwrap_or((rest, ""));
    !method.is_empty() && (service.starts_with("skakel.") || service == remote::SERVICE_NAME)
}

/// `prefix` is the whole path or is followed by `/`. Case sensitive: a router that is case
/// sensitive and a guard that is not disagree about what a path is.
pub(crate) fn starts_with_segments(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_match_whole_segments_and_the_case_they_are_written_in() {
        assert!(starts_with_segments("/api/server", "/api/server"));
        assert!(starts_with_segments("/api/server/clients", "/api/server"));
        assert!(!starts_with_segments("/API/Server/clients", "/api/server"));
        assert!(!starts_with_segments("/api/servers", "/api/server"));
        assert!(!starts_with_segments("/ap", "/api"));
    }

    #[test]
    fn a_grpc_call_is_told_by_its_path() {
        for path in [
            "/skakel.v3.ControlService/Connect",
            "/skakel.v3.StreamService/Open",
            "/koppel.remote.v1.RemoteDevices/Watch",
        ] {
            assert!(is_grpc_path(path), "{path}");
            assert!(is_network_path(path), "{path}");
        }
        for path in [
            "/",
            "/api/v1/auth/login",
            "/skakel/info",
            "/skakel.v3.ControlService",
            "/Skakel.v3.ControlService/Connect",
            "/koppel.remote.v1.RemoteDevices",
            "/koppel.remote.v1.Other/Watch",
            "/koppel.remote.v2.RemoteDevices/Watch",
        ] {
            assert!(!is_grpc_path(path), "{path}");
        }
    }

    #[test]
    fn each_path_belongs_to_one_surface() {
        for path in [
            "/skakel/info",
            "/skakel/v3/token",
            "/api/remote/devices",
            "/skakel.v3.ControlService/Hello",
        ] {
            assert!(is_network_path(path), "{path}");
        }
        for path in [
            "/",
            "/healthz",
            "/api/v1/settings",
            "/api/v1/server/clients",
            "/api/remotes",
            "/skakels",
            "/SKAKEL/info",
        ] {
            assert!(!is_network_path(path), "{path}");
        }
    }

    #[test]
    fn a_role_serves_its_own_scope() {
        assert!(RoleScope::Server.served_to(Role::Server));
        assert!(RoleScope::Server.served_to(Role::Both));
        assert!(!RoleScope::Server.served_to(Role::Client));
        assert!(RoleScope::Client.served_to(Role::Client));
        assert!(RoleScope::Client.served_to(Role::Both));
        assert!(!RoleScope::Client.served_to(Role::Server));
    }
}
