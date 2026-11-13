//! One error type for every handler. Responses use RFC 9457 problem details.
//! Internal causes are logged, never returned to the client.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("authentication required")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    #[error("too many requests")]
    RateLimited,
    #[error("upstream service error: {0}")]
    Upstream(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type AppResult<T> = Result<T, AppError>;

pub fn bad(msg: impl Into<String>) -> AppError {
    AppError::BadRequest(msg.into())
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            AppError::Upstream(_) => StatusCode::BAD_GATEWAY,
            AppError::Db(e) => match e {
                sqlx::Error::RowNotFound => StatusCode::NOT_FOUND,
                sqlx::Error::Database(d) if d.is_unique_violation() => StatusCode::CONFLICT,
                sqlx::Error::Database(d) if d.is_check_violation() || d.is_foreign_key_violation() => StatusCode::BAD_REQUEST,
                // SQLSTATE class 22 (data exception): the value itself is unacceptable, e.g. a NUL byte in text.
                sqlx::Error::Database(d) if d.code().map(|c| c.starts_with("22")).unwrap_or(false) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            AppError::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self.status() {
            StatusCode::UNAUTHORIZED => "unauthorized",
            StatusCode::FORBIDDEN => "forbidden",
            StatusCode::NOT_FOUND => "not_found",
            StatusCode::BAD_REQUEST => "invalid_request",
            StatusCode::CONFLICT => "conflict",
            StatusCode::TOO_MANY_REQUESTS => "rate_limited",
            StatusCode::BAD_GATEWAY => "upstream_error",
            _ => "internal_error",
        }
    }

    /// Message that is safe to show to a client.
    pub fn public_message(&self) -> String {
        match self {
            AppError::Db(e) => match self.status() {
                StatusCode::NOT_FOUND => "not found".into(),
                StatusCode::CONFLICT => "a record with the same unique value already exists".into(),
                StatusCode::BAD_REQUEST => match e {
                    sqlx::Error::Database(d) => match d.constraint() {
                        Some(c) => format!("value rejected by constraint {c}"),
                        None => "a value contains characters or a format that cannot be stored".into(),
                    },
                    _ => "invalid value".into(),
                },
                _ => "internal error".into(),
            },
            AppError::Other(_) => "internal error".into(),
            other => reword(&other.to_string()),
        }
    }
}

/// A parser's complaint, without the names of the program's own types: "expected struct
/// NewItem" says nothing to a caller, "expected an object" does.
pub fn reword(text: &str) -> String {
    let mut out = text.to_string();
    for kind in ["struct ", "enum ", "variant "] {
        while let Some(at) = out.find(&format!("expected {kind}")) {
            let start = at + "expected ".len();
            let end = out[start + kind.len()..].find(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':')).map(|n| start + kind.len() + n).unwrap_or(out.len());
            out.replace_range(start..end, if kind == "struct " { "an object" } else { "one of the allowed values" });
        }
    }
    // The id parser describes the character it stumbled on; "not a valid id" is enough.
    while let Some(at) = out.find("UUID parsing failed") {
        let end = out[at..].find(" at line ").map(|n| at + n).unwrap_or(out.len());
        out.replace_range(at..end, "not a valid id");
    }
    out
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        if status.is_server_error() {
            tracing::error!(error = ?self, "request failed");
        }
        let body = json!({
            "type": "about:blank",
            "title": status.canonical_reason().unwrap_or("Error"),
            "status": status.as_u16(),
            "code": self.code(),
            "detail": self.public_message(),
        });
        let mut resp = (status, axum::Json(body)).into_response();
        resp.headers_mut().insert(header::CONTENT_TYPE, header::HeaderValue::from_static("application/problem+json"));
        if status == StatusCode::UNAUTHORIZED {
            resp.headers_mut().insert(header::WWW_AUTHENTICATE, header::HeaderValue::from_static("Bearer realm=\"gantry\""));
        }
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_errors_do_not_leak_details() {
        let e = AppError::Other(anyhow::anyhow!("password=hunter2 at 10.0.0.5"));
        assert_eq!(e.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(e.public_message(), "internal error");
    }

    #[test]
    fn parser_complaints_are_reworded() {
        assert_eq!(reword("q is not a valid search: invalid type: integer `1`, expected struct ItemQuery at line 1 column 1"), "q is not a valid search: invalid type: integer `1`, expected an object at line 1 column 1");
        assert_eq!(reword("level_id: UUID parsing failed: invalid character: found `O` at 0 at line 1 column 400"), "level_id: not a valid id at line 1 column 400");
        assert_eq!(reword("container: UUID parsing failed: invalid length: found 0"), "container: not a valid id");
        assert_eq!(reword("unknown variant `x`, expected enum planning::Kind"), "unknown variant `x`, expected one of the allowed values");
        assert_eq!(reword("name must be 1 to 200 characters"), "name must be 1 to 200 characters");
        assert_eq!(bad("expected struct Thing").public_message(), "expected an object");
    }

    #[test]
    fn row_not_found_maps_to_404() {
        let e = AppError::Db(sqlx::Error::RowNotFound);
        assert_eq!(e.status(), StatusCode::NOT_FOUND);
        assert_eq!(e.code(), "not_found");
    }

    #[test]
    fn status_codes() {
        assert_eq!(AppError::Unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(AppError::Forbidden("x".into()).status(), StatusCode::FORBIDDEN);
        assert_eq!(bad("x").status(), StatusCode::BAD_REQUEST);
        assert_eq!(AppError::RateLimited.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
