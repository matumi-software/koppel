use std::fmt;

/// A USB operation that did not work. The message is written for the UI, as 0.1.x's
/// `UsbHostException` was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbError(String);

impl UsbError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UsbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsbError {}
