use {
    crate::{
        api::{content::ContentError, ApiError},
        bitcoin_rpc::{RpcClientError, RpcClientPoolError},
        index::{IndexError, StoreError},
    },
    axum::response::{IntoResponse, Response},
    http::{header, HeaderValue, StatusCode},
    std::fmt::Write,
    tracing::error,
};

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("content error: {0}")]
    ContentError(#[from] ContentError),

    #[error("rpc client error: {0}")]
    RpcClientError(#[from] RpcClientError),

    #[error("rpc client pool error: {0}")]
    RpcClientPoolError(#[from] RpcClientPoolError),

    #[error("api error: {0}")]
    ApiError(#[from] ApiError),

    #[error("not found: {0}")]
    NotFound(String),
}

pub type ServerResult<T = Response> = Result<T, ServerError>;

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message).into_response(),
            Self::ApiError(ApiError::IndexError(IndexError::StoreError(StoreError::NotFound(
                message,
            )))) => (StatusCode::NOT_FOUND, message).into_response(),
            Self::ApiError(ApiError::RpcError(error)) => {
                error!("rpc error: {error}");
                (StatusCode::BAD_REQUEST, error.to_string()).into_response()
            }
            Self::ApiError(error) => {
                error!("error serving request: {error}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    StatusCode::INTERNAL_SERVER_ERROR
                        .canonical_reason()
                        .unwrap_or_default(),
                )
                    .into_response()
            }
            Self::RpcClientError(error) => {
                error!("rpc client error: {error}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    StatusCode::INTERNAL_SERVER_ERROR
                        .canonical_reason()
                        .unwrap_or_default(),
                )
                    .into_response()
            }
            Self::RpcClientPoolError(error) => {
                error!("rpc client pool error: {error}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    StatusCode::INTERNAL_SERVER_ERROR
                        .canonical_reason()
                        .unwrap_or_default(),
                )
                    .into_response()
            }
            Self::ContentError(ContentError::NotAcceptable {
                accept_encoding,
                content_encoding,
            }) => {
                let mut message = format!(
                    "inscription content encoding `{}` is not acceptable.",
                    String::from_utf8_lossy(content_encoding.as_bytes())
                );

                if let Some(accept_encoding) = accept_encoding.0 {
                    write!(message, " `Accept-Encoding` header: `{accept_encoding}`").unwrap();
                } else {
                    write!(message, " `Accept-Encoding` header not present").unwrap();
                };

                (StatusCode::NOT_ACCEPTABLE, message).into_response()
            }
            Self::ContentError(_) => {
                error!("content error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    StatusCode::INTERNAL_SERVER_ERROR
                        .canonical_reason()
                        .unwrap_or_default(),
                )
                    .into_response()
            }
            Self::NotFound(message) => (
                StatusCode::NOT_FOUND,
                [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
                message,
            )
                .into_response(),
        }
    }
}

pub(super) trait OptionExt<T> {
    fn ok_or_not_found<F: FnOnce() -> S, S: Into<String>>(self, f: F) -> ServerResult<T>;
}

impl<T> OptionExt<T> for Option<T> {
    fn ok_or_not_found<F: FnOnce() -> S, S: Into<String>>(self, f: F) -> ServerResult<T> {
        match self {
            Some(value) => Ok(value),
            None => Err(ServerError::NotFound(f().into() + " not found")),
        }
    }
}
