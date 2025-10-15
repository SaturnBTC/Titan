use crate::{
    alkanes::indexer::AlkanesIndexer,
    index::Index,
    server::error::{ServerError, ServerResult},
};
use alkanes_support::proto as alkanes_proto;
use axum::{
    extract::{Extension, Path},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use prost::Message;
use protorune_support::proto as protorune_proto;
use bitcoin::Address;
use std::sync::Arc;
use titan_types::SerializedOutPoint;

pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/alkanes/health", get(health_check))
        .route("/alkanes/getbytecode/:block/:tx", get(get_bytecode))
        .route("/alkanes/byaddress/:address", get(by_address))
        .route("/alkanes/byoutpoint/:outpoint", get(by_outpoint))
        .route("/alkanes/trace/:outpoint", get(trace_outpoint))
        .route("/alkanes/getinventory/:block/:tx", get(get_inventory))
        .route(
            "/alkanes/getstorageat/:block/:tx/:key",
            get(get_storage_at),
        )
        .route("/alkanes/simulate", post(simulate))
}

async fn health_check() -> impl IntoResponse {
    (axum::http::StatusCode::OK, Json("ok"))
}

#[axum::debug_handler]
async fn get_bytecode(
    Extension(alkanes_indexer): Extension<Arc<AlkanesIndexer>>,
    Extension(index): Extension<Arc<Index>>,
    Path((block, tx)): Path<(u64, u32)>,
) -> ServerResult {
    let height = index.get_block_count().unwrap_or(0);
    let request = alkanes_proto::alkanes::BytecodeRequest {
        id: Some(alkanes_proto::alkanes::AlkaneId {
            block: Some(alkanes_proto::alkanes::Uint128 {
                lo: block as u64,
                hi: 0,
            }),
            tx: Some(alkanes_proto::alkanes::Uint128 {
                lo: tx as u64,
                hi: 0,
            }),
        }),
    };
    let mut payload = Vec::new();
    request.encode(&mut payload).unwrap();
    let result = alkanes_indexer
        .view("getbytecode".to_string(), &payload, height as u32)
        .await
        .map_err(|e| ServerError::BadRequest(e.to_string()))?;
    Ok(Json(format!("0x{}", hex::encode(result))).into_response())
}

#[axum::debug_handler]
async fn by_address(
    Extension(alkanes_indexer): Extension<Arc<AlkanesIndexer>>,
    Extension(index): Extension<Arc<Index>>,
    Path(address): Path<Address<bitcoin::address::NetworkUnchecked>>,
) -> ServerResult {
    let network = index.network();
    let address = address.require_network(network).map_err(|e| ServerError::BadRequest(e.to_string()))?;
    let height = index.get_block_count().unwrap_or(0);
    let request = protorune_proto::protorune::ProtorunesWalletRequest {
        wallet: address.script_pubkey().as_bytes().to_vec(),
        protocol_tag: Some(protorune_proto::protorune::Uint128 {
            lo: 1,
            hi: 0,
        }),
    };
    let mut payload = Vec::new();
    request.encode(&mut payload).unwrap();
    let result = alkanes_indexer
        .view("protorunesbyaddress".to_string(), &payload, height as u32)
        .await
        .map_err(|e| ServerError::BadRequest(e.to_string()))?;
    let response = protorune_proto::protorune::WalletResponse::decode(result.as_slice())
        .map_err(|e| ServerError::BadRequest(e.to_string()))?;
    Ok(Json(response).into_response())
}

#[axum::debug_handler]
async fn by_outpoint(
    Extension(alkanes_indexer): Extension<Arc<AlkanesIndexer>>,
    Extension(index): Extension<Arc<Index>>,
    Path(outpoint): Path<SerializedOutPoint>,
) -> ServerResult {
    let height = index.get_block_count().unwrap_or(0);
    let request = protorune_proto::protorune::OutpointWithProtocol {
        txid: outpoint.txid().to_vec(),
        vout: outpoint.vout(),
        protocol: Some(protorune_proto::protorune::Uint128 { lo: 1, hi: 0 }),
    };
    let mut payload = Vec::new();
    request.encode(&mut payload).unwrap();
    let result = alkanes_indexer
        .view("protorunesbyoutpoint".to_string(), &payload, height as u32)
        .await
        .map_err(|e| ServerError::BadRequest(e.to_string()))?;
    let response = protorune_proto::protorune::OutpointResponse::decode(result.as_slice())
        .map_err(|e| ServerError::BadRequest(e.to_string()))?;
    Ok(Json(response).into_response())
}

#[axum::debug_handler]
async fn trace_outpoint(
    Extension(alkanes_indexer): Extension<Arc<AlkanesIndexer>>,
    Extension(index): Extension<Arc<Index>>,
    Path(outpoint): Path<SerializedOutPoint>,
) -> ServerResult {
    let height = index.get_block_count().unwrap_or(0);
    let request = protorune_proto::protorune::Outpoint {
        txid: outpoint.txid().to_vec(),
        vout: outpoint.vout(),
    };
    let mut payload = Vec::new();
    request.encode(&mut payload).unwrap();
    let result = alkanes_indexer
        .view("trace".to_string(), &payload, height as u32)
        .await
        .map_err(|e| ServerError::BadRequest(e.to_string()))?;
    let response = alkanes_proto::alkanes::AlkanesTrace::decode(result.as_slice())
        .map_err(|e| ServerError::BadRequest(e.to_string()))?;
    Ok(Json(response).into_response())
}

#[axum::debug_handler]
async fn get_inventory(
    Extension(alkanes_indexer): Extension<Arc<AlkanesIndexer>>,
    Extension(index): Extension<Arc<Index>>,
    Path((block, tx)): Path<(u64, u32)>,
) -> ServerResult {
    let height = index.get_block_count().unwrap_or(0);
    let request = alkanes_proto::alkanes::AlkaneInventoryRequest {
        id: Some(alkanes_proto::alkanes::AlkaneId {
            block: Some(alkanes_proto::alkanes::Uint128 {
                lo: block as u64,
                hi: 0,
            }),
            tx: Some(alkanes_proto::alkanes::Uint128 { lo: tx as u64, hi: 0 }),
        }),
    };
    let mut payload = Vec::new();
    request.encode(&mut payload).unwrap();
    let result = alkanes_indexer
        .view("getinventory".to_string(), &payload, height as u32)
        .await
        .map_err(|e| ServerError::BadRequest(e.to_string()))?;
    let response = alkanes_proto::alkanes::AlkaneInventoryResponse::decode(result.as_slice())
        .map_err(|e| ServerError::BadRequest(e.to_string()))?;
    Ok(Json(response).into_response())
}

#[axum::debug_handler]
async fn get_storage_at(
    Extension(alkanes_indexer): Extension<Arc<AlkanesIndexer>>,
    Extension(index): Extension<Arc<Index>>,
    Path((block, tx, key)): Path<(u64, u32, String)>,
) -> ServerResult {
    let height = index.get_block_count().unwrap_or(0);
    let request = alkanes_proto::alkanes::AlkaneStorageRequest {
        id: Some(alkanes_proto::alkanes::AlkaneId {
            block: Some(alkanes_proto::alkanes::Uint128 {
                lo: block as u64,
                hi: 0,
            }),
            tx: Some(alkanes_proto::alkanes::Uint128 { lo: tx as u64, hi: 0 }),
        }),
        path: hex::decode(key).map_err(|e| ServerError::BadRequest(e.to_string()))?,
    };
    let mut payload = Vec::new();
    request.encode(&mut payload).unwrap();
    let result = alkanes_indexer
        .view("getstorageat".to_string(), &payload, height as u32)
        .await
        .map_err(|e| ServerError::BadRequest(e.to_string()))?;
    let response = alkanes_proto::alkanes::AlkaneStorageResponse::decode(result.as_slice())
        .map_err(|e| ServerError::BadRequest(e.to_string()))?;
    Ok(Json(format!("0x{}", hex::encode(response.value))).into_response())
}

use serde::Deserialize;



#[derive(Deserialize)]

struct SimulateRequest {

    alkanes: Vec<AlkaneTransfer>,

    transaction: String,

    target: AlkaneId,

    inputs: Vec<u128>,

    height: u64,

    block: String,

    txindex: u32,

    vout: u32,

    pointer: u32,

    refund_pointer: u32,

}



#[derive(Deserialize)]

struct AlkaneTransfer {

    id: AlkaneId,

    value: u128,

}



#[derive(Deserialize)]

struct AlkaneId {

    block: u128,

    tx: u128,

}



#[axum::debug_handler]
async fn simulate(

    Extension(alkanes_indexer): Extension<Arc<AlkanesIndexer>>,

    Extension(index): Extension<Arc<Index>>,

    Json(request): Json<SimulateRequest>,

) -> ServerResult {

    let height = index.get_block_count().unwrap_or(0);

            let alkanes = request

                .alkanes

                .into_iter()

                .map(|a| alkanes_proto::alkanes::AlkaneTransfer {

                    id: Some(alkanes_proto::alkanes::AlkaneId {

                        block: Some(alkanes_proto::alkanes::Uint128 {

                            lo: a.id.block as u64,

                            hi: (a.id.block >> 64) as u64,

                        }),

                        tx: Some(alkanes_proto::alkanes::Uint128 {

                            lo: a.id.tx as u64,

                            hi: (a.id.tx >> 64) as u64,

                        }),

                    }),

                    value: Some(alkanes_proto::alkanes::Uint128 {

                        lo: a.value as u64,

                        hi: (a.value >> 64) as u64,

                    }),

                })

                .collect();

            let target = alkanes_proto::alkanes::AlkaneId {

                block: Some(alkanes_proto::alkanes::Uint128 {

                    lo: request.target.block as u64,

                    hi: (request.target.block >> 64) as u64,

                }),

                tx: Some(alkanes_proto::alkanes::Uint128 {

                    lo: request.target.tx as u64,

                    hi: (request.target.tx >> 64) as u64,

                }),

            };

                let inputs: Vec<alkanes_proto::alkanes::Uint128> = request.inputs.iter().map(|i| alkanes_proto::alkanes::Uint128 { lo: *i as u64, hi: (*i >> 64) as u64}).collect();

                let mut calldata = Vec::new();

                leb128::write::unsigned(&mut calldata, request.target.block as u64).unwrap();

                leb128::write::unsigned(&mut calldata, request.target.tx as u64).unwrap();

                for input in request.inputs.iter() {

                    leb128::write::unsigned(&mut calldata, *input as u64).unwrap();

                }

        

            let parcel = alkanes_proto::alkanes::MessageContextParcel {

                alkanes,

                transaction: hex::decode(request.transaction).unwrap(),

                height: request.height,

                txindex: request.txindex,

                calldata,

                block: hex::decode(request.block).unwrap(),

                vout: request.vout,

                pointer: request.pointer,

                refund_pointer: request.refund_pointer,

            };

        let mut payload = Vec::new();

        parcel.encode(&mut payload).unwrap();

        let result = alkanes_indexer

            .view("simulate".to_string(), &payload, height as u32)

            .await

            .map_err(|e| ServerError::BadRequest(e.to_string()))?;

        let response = alkanes_proto::alkanes::SimulateResponse::decode(result.as_slice())

            .map_err(|e| ServerError::BadRequest(e.to_string()))?;

    Ok(Json(response).into_response())

}
