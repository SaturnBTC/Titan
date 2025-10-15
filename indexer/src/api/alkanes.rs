use crate::{
    api::{Json, JsonResult, Page, Pagination, Path, Query, Router},
    index::Index,
};
use axum::routing::get;
use std::sync::Arc;

pub fn router(state: &Extension<Arc<Index>>) -> Router {
    Router::new()
        .route("/alkanes/protorunesbyaddress", get(protorunes_by_address))
}

async fn protorunes_by_address(
    p: Path<String>,
    Extension(index): Extension<Arc<Index>>,
) -> JsonResult<String> {
    Ok(Json("hello".to_string()))
}
