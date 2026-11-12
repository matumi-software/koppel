//! What an API handler answers when it cannot do what it was asked: one error type, written as
//! RFC 9457 problem JSON (`application/problem+json`).
//!
//! The body always has `type` (`about:blank`: the status says it all), `title` (the status's
//! reason phrase), `status`, `detail` (a sentence the UI can show as it is) and `code`, a stable
//! word such as `validation` or `wrong_password` that a client can branch on without reading the
//! sentence. A validation problem adds `errors`, each field with its messages. A handler returns
//! `Result<Json<T>, ApiError>` (or `Result<StatusCode, ApiError>` for an answer without a body), and
//! `?` does the rest.

use std::error::Error;

use axum::Json;
use axum::http::StatusCode;
use axum::http::header::{CONTENT_TYPE, HeaderValue, LOCATION, RETRY_AFTER};
use axum::response::{IntoResponse, Response};
use indexmap::IndexMap;
use serde::Serialize;
use ts_rs::TS;

/// What an API handler returns.
pub(crate) type ApiResult<T> = Result<T, ApiError>;

/// A request that was refused or failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ApiError {
    /// 400 with `code: validation` and a message for each field that is wrong.
    #[error("the request is not valid")]
    Validation { errors: Vec<(String, String)> },
    /// Any other refusal with its status, a stable code and a sentence for the person.
    #[error("{detail}")]
    Refused {
        status: StatusCode,
        code: &'static str,
        detail: String,
        /// Seconds, for `Retry-After`.
        retry_after: Option<u64>,
    },
    /// A change that could not be written, or anything else the person did nothing wrong in. The
    /// caller gets a 500 with no detail of the cause; the log gets the whole chain.
    #[error("{what} failed")]
    Internal {
        what: &'static str,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

impl ApiError {
    /// A validation problem: each field with its message.
    pub(crate) fn invalid(errors: &[(&str, &str)]) -> Self {
        Self::Validation {
            errors: errors
                .iter()
                .map(|(field, message)| ((*field).to_owned(), (*message).to_owned()))
                .collect(),
        }
    }

    pub(crate) fn refused(
        status: StatusCode,
        code: &'static str,
        detail: impl Into<String>,
    ) -> Self {
        Self::Refused {
            status,
            code,
            detail: detail.into(),
            retry_after: None,
        }
    }

    /// 429 with `Retry-After`.
    pub(crate) fn too_many(
        code: &'static str,
        detail: impl Into<String>,
        retry_after: u64,
    ) -> Self {
        Self::Refused {
            status: StatusCode::TOO_MANY_REQUESTS,
            code,
            detail: detail.into(),
            retry_after: Some(retry_after),
        }
    }

    pub(crate) fn not_found(detail: impl Into<String>) -> Self {
        Self::refused(StatusCode::NOT_FOUND, "not_found", detail)
    }

    /// A failure with its cause kept for the log.
    pub(crate) fn internal(
        what: &'static str,
        source: impl Into<Box<dyn Error + Send + Sync>>,
    ) -> Self {
        Self::Internal {
            what,
            source: source.into(),
        }
    }

    pub(crate) fn status(&self) -> StatusCode {
        match self {
            Self::Validation { .. } => StatusCode::BAD_REQUEST,
            Self::Refused { status, .. } => *status,
            Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Validation { .. } => "validation",
            Self::Refused { code, .. } => code,
            Self::Internal { .. } => "internal",
        }
    }

    fn problem(&self) -> Problem {
        let status = self.status();
        let (detail, errors) = match self {
            Self::Validation { errors } => {
                let mut fields: IndexMap<String, Vec<String>> = IndexMap::new();
                for (field, message) in errors {
                    fields
                        .entry(field.clone())
                        .or_default()
                        .push(message.clone());
                }
                ("One or more fields are not valid.".to_owned(), Some(fields))
            }
            Self::Refused { detail, .. } => (detail.clone(), None),
            Self::Internal { .. } => (
                "Koppel could not complete this. The log has the reason.".to_owned(),
                None,
            ),
        };
        Problem {
            kind: "about:blank".to_owned(),
            title: status.canonical_reason().unwrap_or("Error").to_owned(),
            status: status.as_u16(),
            code: self.code().to_owned(),
            detail,
            errors,
        }
    }
}

/// The body of every refusal: RFC 9457 problem JSON.
#[derive(Serialize, TS)]
#[cfg_attr(test, derive(serde::Deserialize))]
pub(crate) struct Problem {
    /// `about:blank`: the status says it all.
    #[serde(rename = "type")]
    kind: String,
    title: String,
    status: u16,
    /// A stable word to branch on, such as `validation` or `wrong_password`.
    code: String,
    /// A sentence the UI shows as it is.
    detail: String,
    /// A validation problem: each field with its messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(type = "Record<string, Array<string>>", optional)]
    errors: Option<IndexMap<String, Vec<String>>>,
}

/// The error and what is under it, as one line: `a: b: c`.
fn chain(error: &(dyn Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(next) = source {
        text.push_str(": ");
        text.push_str(&next.to_string());
        source = next.source();
    }
    text
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let Self::Internal { what, source } = &self {
            tracing::error!("{what} failed: {}", chain(source.as_ref()));
        }
        let mut response = (
            self.status(),
            [(
                CONTENT_TYPE,
                HeaderValue::from_static("application/problem+json"),
            )],
            serde_json::to_vec(&self.problem()).unwrap_or_default(),
        )
            .into_response();
        if let Self::Refused {
            retry_after: Some(seconds),
            ..
        } = &self
        {
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from(*seconds));
        }
        response
    }
}

/// 202 with no body: the confirmed state follows over the push.
pub(crate) fn accepted() -> ApiResult<StatusCode> {
    Ok(StatusCode::ACCEPTED)
}

/// 201 with a `Location` and the value as JSON.
pub(crate) struct Created<T>(pub String, pub T);

impl<T: serde::Serialize> IntoResponse for Created<T> {
    fn into_response(self) -> Response {
        let mut response = (StatusCode::CREATED, Json(self.1)).into_response();
        if let Ok(location) = HeaderValue::from_str(&self.0) {
            response.headers_mut().insert(LOCATION, location);
        }
        response
    }
}
