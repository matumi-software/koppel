//! The certificate the server presents, which can be replaced while it runs.

use std::fmt;
use std::sync::{Arc, RwLock};

use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;

use crate::sync::UnpoisonedRw;

/// The certificate the server presents. Every handshake reads the current one, so a replaced
/// certificate is what the next connection sees. Connections already open keep theirs.
pub struct CertificateSlot {
    current: RwLock<Arc<CertifiedKey>>,
}

impl CertificateSlot {
    pub fn new(certified: CertifiedKey) -> Arc<Self> {
        Arc::new(Self {
            current: RwLock::new(Arc::new(certified)),
        })
    }

    /// The certificate new connections get.
    pub fn current(&self) -> Arc<CertifiedKey> {
        self.current.read_locked().clone()
    }

    /// Presents `certified` from the next handshake on.
    pub fn replace(&self, certified: CertifiedKey) {
        *self.current.write_locked() = Arc::new(certified);
    }
}

impl fmt::Debug for CertificateSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CertificateSlot")
    }
}

impl ResolvesServerCert for CertificateSlot {
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.current())
    }
}
