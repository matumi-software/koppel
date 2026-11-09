//! Asks the UI's `/healthz` whether Koppel answers, as 0.1.x's health probe did, through WinHTTP so
//! KoppelDesktop.exe carries no TLS stack of its own. Koppel's certificate is its own, self-signed,
//! so the certificate is not checked: the answer only says whether Koppel is up.

use std::time::Duration;

use windows::Win32::Networking::WinHttp::{
    SECURITY_FLAG_IGNORE_CERT_CN_INVALID, SECURITY_FLAG_IGNORE_CERT_DATE_INVALID,
    SECURITY_FLAG_IGNORE_CERT_WRONG_USAGE, SECURITY_FLAG_IGNORE_UNKNOWN_CA,
    WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_OPTION_SECURITY_FLAGS,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
};
use windows::core::{HSTRING, PCWSTR, w};

/// One try: true when `GET /healthz` on `host`:`port` answers 200 within `timeout`.
pub fn answers(host: &str, port: u16, timeout: Duration) -> bool {
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: every handle is closed by `Handle` when it goes out of scope, children before their
    // parents (they are declared in that order), and every buffer lives through its call.
    unsafe {
        let Some(session) = Handle::new(WinHttpOpen(
            w!("Koppel"),
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        )) else {
            return false;
        };
        if WinHttpSetTimeouts(session.0, ms, ms, ms, ms).is_err() {
            return false;
        }
        let host = HSTRING::from(host.trim_start_matches('[').trim_end_matches(']'));
        let Some(connection) = Handle::new(WinHttpConnect(session.0, &host, port, 0)) else {
            return false;
        };
        let Some(request) = Handle::new(WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            w!("/healthz"),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        )) else {
            return false;
        };
        let flags = (SECURITY_FLAG_IGNORE_UNKNOWN_CA
            | SECURITY_FLAG_IGNORE_CERT_CN_INVALID
            | SECURITY_FLAG_IGNORE_CERT_DATE_INVALID
            | SECURITY_FLAG_IGNORE_CERT_WRONG_USAGE)
            .to_le_bytes();
        if WinHttpSetOption(Some(request.0), WINHTTP_OPTION_SECURITY_FLAGS, Some(&flags)).is_err() {
            return false;
        }
        if WinHttpSendRequest(request.0, None, None, 0, 0, 0).is_err()
            || WinHttpReceiveResponse(request.0, std::ptr::null_mut()).is_err()
        {
            return false;
        }
        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut status).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .is_ok()
            && status == 200
    }
}

/// A WinHTTP handle, closed when dropped.
struct Handle(*mut core::ffi::c_void);

impl Handle {
    fn new(raw: *mut core::ffi::c_void) -> Option<Self> {
        (!raw.is_null()).then_some(Self(raw))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: a handle WinHTTP gave us, closed once.
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}
