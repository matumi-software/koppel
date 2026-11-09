//! What is served where: the one table of path prefixes, the port each is on and the role that has
//! it, as `Surface.Listed` in 0.1.x's `SurfaceGuard.cs`. A path not listed is the UI's, so a new
//! route is never exposed to remote clients by accident.

use koppel_config::files::Role;

/// Which port a surface is served on. Both are the same port unless `Ports:Network` is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Listener {
    /// The UI port: the people who run this machine.
    Ui,
    /// The network port: other Koppel machines.
    Network,
}

/// Which role a surface belongs to. A machine that is both serves both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoleScope {
    Any,
    Server,
    Client,
}

/// A path prefix, the port that serves it and the role that needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Surface {
    pub prefix: &'static str,
    pub listener: Listener,
    pub role: RoleScope,
}

/// Everything not listed below, the UI's files and its own API included.
const UI: Surface = Surface {
    prefix: "/",
    listener: Listener::Ui,
    role: RoleScope::Any,
};

const LISTED: &[Surface] = &[
    // Skakel's HTTPS calls: info, pairing and tokens. Its gRPC calls are `Surface::GRPC`.
    Surface {
        prefix: "/skakel",
        listener: Listener::Network,
        role: RoleScope::Server,
    },
    Surface {
        prefix: "/api/remote",
        listener: Listener::Network,
        role: RoleScope::Server,
    },
    Surface {
        prefix: "/api/server",
        listener: Listener::Ui,
        role: RoleScope::Server,
    },
    Surface {
        prefix: "/api/client",
        listener: Listener::Ui,
        role: RoleScope::Client,
    },
];

/// The UI paths a person may reach without signing in. Everything else of the UI's under `/api`, a
/// route added tomorrow included, needs the session cookie. The UI's files and `/healthz` are not
/// under `/api`, so they stay open.
const ANONYMOUS: &[&str] = &["/api/auth/state", "/api/auth/setup", "/api/auth/login"];

impl Surface {
    /// A gRPC call, told by its content type: Skakel's control and stream services and Koppel's own
    /// remote device list, for other machines.
    pub(crate) const GRPC: Surface = Surface {
        prefix: "/",
        listener: Listener::Network,
        role: RoleScope::Server,
    };

    pub(crate) fn of(path: &str) -> Surface {
        LISTED
            .iter()
            .copied()
            .find(|surface| starts_with_segments(path, surface.prefix))
            .unwrap_or(UI)
    }

    pub(crate) fn served_to(self, role: Role) -> bool {
        match self.role {
            RoleScope::Server => role != Role::Client,
            RoleScope::Client => role != Role::Server,
            RoleScope::Any => true,
        }
    }
}

/// True when the path is the UI's API, the live push included, and not one of the anonymous paths.
pub(crate) fn requires_sign_in(path: &str) -> bool {
    Surface::of(path).listener == Listener::Ui
        && starts_with_segments(path, "/api")
        && !ANONYMOUS
            .iter()
            .any(|anonymous| path.eq_ignore_ascii_case(anonymous))
}

/// ASP.NET's `PathString.StartsWithSegments`: `prefix` is the whole path or is followed by `/`,
/// without regard to case. `/` starts every path.
pub(crate) fn starts_with_segments(path: &str, prefix: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    let Some(head) = path.get(..prefix.len()) else {
        return false;
    };
    head.eq_ignore_ascii_case(prefix)
        && matches!(path.as_bytes().get(prefix.len()), None | Some(b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_match_whole_segments_in_any_case() {
        assert!(starts_with_segments("/api/server", "/api/server"));
        assert!(starts_with_segments("/API/Server/clients", "/api/server"));
        assert!(!starts_with_segments("/api/servers", "/api/server"));
        assert!(!starts_with_segments("/ap", "/api"));
    }

    #[test]
    fn each_path_has_its_surface_and_sign_in_rule() {
        assert_eq!(Surface::of("/skakel/info").listener, Listener::Network);
        assert_eq!(Surface::of("/api/client/devices").role, RoleScope::Client);
        assert_eq!(Surface::of("/api/settings"), UI);
        assert!(requires_sign_in("/api/settings"));
        assert!(requires_sign_in("/api/not-a-route-yet"));
        assert!(requires_sign_in("/api/events"));
        assert!(!requires_sign_in("/api/auth/state"));
        assert!(!requires_sign_in("/API/AUTH/LOGIN"));
        assert!(!requires_sign_in("/api/remote/devices"));
        assert!(!requires_sign_in("/healthz"));
        assert!(!requires_sign_in("/"));
        assert!(!Surface::of("/api/server/clients").served_to(Role::Client));
        assert!(Surface::of("/api/server/clients").served_to(Role::Both));
    }
}
