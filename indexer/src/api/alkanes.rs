use crate::server::error::ServerResult;
use axum::{
    response::IntoResponse,
    routing::get, Json, Router
};

pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new().route("/alkanes/health", get(health_check))
}

async fn health_check() -> impl IntoResponse {
    (axum::http::StatusCode::OK, Json("ok"))
}