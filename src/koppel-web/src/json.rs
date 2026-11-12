//! The JSON request body: read with the names the UI sends (camelCase). A body that is missing or
//! is not JSON is refused with 400, and one not sent as JSON with 415, both as problem JSON.

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use serde::de::DeserializeOwned;

use crate::api::ApiError;

/// A JSON request body. ASP.NET answered a body it could not read with a bare 400, a missing one
/// too, and one that was not sent as JSON with 415; the statuses are kept.
pub(crate) struct JsonBody<T>(pub T);

fn unreadable() -> ApiError {
    ApiError::refused(
        StatusCode::BAD_REQUEST,
        "bad_request",
        "The request body is missing or is not valid JSON.",
    )
}

impl<S, T> FromRequest<S> for JsonBody<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let is_json = is_json(request.headers());
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|_| unreadable())?;
        if bytes.is_empty() {
            return Err(unreadable());
        }
        if !is_json {
            return Err(ApiError::refused(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                "Send the body as application/json.",
            ));
        }
        // A byte order mark is skipped, as ASP.NET did.
        serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
            .map(JsonBody)
            .map_err(|_| unreadable())
    }
}

/// `application/json` or a `+json` type, as ASP.NET's `HasJsonContentType` takes it.
fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
        .is_some_and(|media| media == "application/json" || media.ends_with("+json"))
}
