//! Request bodies and answers in the shapes ASP.NET's minimal APIs gave 0.1.x: JSON read with the names the UI sends
//! (camelCase), a body that is missing or not JSON refused with 400, validation problems as
//! `application/problem+json`, and `{ error }` for the sign-in endpoints.

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::header::{CONTENT_TYPE, HeaderValue, LOCATION};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

/// A JSON request body. ASP.NET answered a body it could not read with a bare 400, a missing one
/// too, and one that was not sent as JSON with 415.
pub(crate) struct JsonBody<T>(pub T);

impl<S, T> FromRequest<S> for JsonBody<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let is_json = is_json(request.headers());
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|_| StatusCode::BAD_REQUEST.into_response())?;
        if bytes.is_empty() {
            return Err(StatusCode::BAD_REQUEST.into_response());
        }
        if !is_json {
            return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response());
        }
        // A byte order mark is skipped, as ASP.NET did.
        serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
            .map(JsonBody)
            .map_err(|_| StatusCode::BAD_REQUEST.into_response())
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

/// A JSON answer with this status.
pub(crate) fn json_answer<T: Serialize>(status: StatusCode, value: &T) -> Response {
    match serde_json::to_vec(value) {
        Ok(body) => (
            status,
            [(
                CONTENT_TYPE,
                HeaderValue::from_static("application/json; charset=utf-8"),
            )],
            body,
        )
            .into_response(),
        Err(e) => {
            tracing::error!("an answer could not be written as JSON: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// 200 with the value as JSON.
pub(crate) fn ok<T: Serialize>(value: &T) -> Response {
    json_answer(StatusCode::OK, value)
}

/// 201 with a `Location` and the value as JSON.
pub(crate) fn created<T: Serialize>(location: &str, value: &T) -> Response {
    let mut response = json_answer(StatusCode::CREATED, value);
    if let Ok(location) = HeaderValue::from_str(location) {
        response.headers_mut().insert(LOCATION, location);
    }
    response
}

/// 202 with no body: the confirmed state follows over the push.
pub(crate) fn accepted() -> Response {
    StatusCode::ACCEPTED.into_response()
}

/// `{ error }` with this status, the shape of the sign-in endpoints' refusals.
pub(crate) fn error(status: StatusCode, message: &str) -> Response {
    json_answer(status, &json!({ "error": message }))
}

/// A validation problem: 400, `application/problem+json`, each field with its messages, as
/// `TypedResults.ValidationProblem` wrote it.
pub(crate) fn problem(errors: &[(&str, &str)]) -> Response {
    let mut fields = Map::new();
    for (field, message) in errors {
        let entry = fields
            .entry(field.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(messages) = entry {
            messages.push(Value::String(message.to_string()));
        }
    }
    let body = json!({
        "type": "https://tools.ietf.org/html/rfc9110#section-15.5.1",
        "title": "One or more validation errors occurred.",
        "status": 400,
        "errors": fields,
    });
    match serde_json::to_vec(&body) {
        Ok(body) => (
            StatusCode::BAD_REQUEST,
            [(
                CONTENT_TYPE,
                HeaderValue::from_static("application/problem+json"),
            )],
            body,
        )
            .into_response(),
        Err(_) => StatusCode::BAD_REQUEST.into_response(),
    }
}

/// A change that could not be written to its file: logged, and 500 to the caller.
pub(crate) fn failed(what: &str, e: impl std::fmt::Display) -> Response {
    tracing::error!("{what} failed: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}
