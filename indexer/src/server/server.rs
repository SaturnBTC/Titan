use {
    super::{
        deserialize_from_str::DeserializeFromStr,
        error::{OptionExt, ServerError, ServerResult},
        ServerConfig,
    },
    crate::{
        alkanes::indexer::AlkanesIndexer,
        api::{self, content::AcceptEncoding},
        bitcoin_rpc::RpcClientPool,
        index::Index,
        subscription::WebhookSubscriptionManager,
    },
    axum::{
        body::Bytes,
        extract::{DefaultBodyLimit, Extension, FromRef, Json, Path, Query},
        response::IntoResponse,
        routing::{get, post},
        Router,
    },
    axum_server::Handle,
    bitcoin::{address::NetworkUnchecked, Address, BlockHash},
    http::{header, StatusCode},
    std::{io, net::ToSocketAddrs, sync::Arc},
    titan_types::{
        query, InscriptionId, Pagination, SerializedOutPoint, SerializedTxid, Subscription,
    },
    tokio::task,
    tower_http::{
        compression::CompressionLayer,
        cors::{Any, CorsLayer},
    },
    tracing::{error, info},
    uuid::Uuid,
};

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("failed to bind to address")]
    BindError(#[from] std::io::Error),
    #[error("failed to parse address")]
    SocketAddrError(#[from] std::net::AddrParseError),
    #[error("no socket address found")]
    NoSocketAddr,
}

type SpawnResult<T> = std::result::Result<T, SpawnError>;

pub struct Server;

impl Server {
    pub fn start(
        &self,
        index: Arc<Index>,
        alkanes_indexer: Arc<AlkanesIndexer>,
        webhook_subscription_manager: Arc<WebhookSubscriptionManager>,
        bitcoin_rpc_pool: RpcClientPool,
        config: Arc<ServerConfig>,
        handle: Handle,
    ) -> SpawnResult<task::JoinHandle<io::Result<()>>> {
        let router = Router::new()
            // Health check
            .route("/", get(Self::health_check))
            // Status
            .route("/status", get(Self::status))
            // Blocks
            .route("/tip", get(Self::tip))
            .route("/block/{query}", get(Self::block))
            .route("/block/{height}/hash", get(Self::block_hash_by_height))
            .route("/block/{query}/txids", get(Self::block_txids))
            // Addresses
            .route("/address/{address}", get(Self::address))
            // Transactions
            .route("/tx/broadcast", post(Self::broadcast_transaction))
            .route("/txs/statuses", post(Self::transaction_statuses))
            .route("/tx/{txid}", get(Self::transaction))
            .route("/tx/{txid}/raw", get(Self::transaction_raw))
            .route("/tx/{txid}/hex", get(Self::transaction_hex))
            .route("/tx/{txid}/status", get(Self::transaction_status))
            .route("/output/{outpoint}", get(Self::output))
            // Inscriptions
            .route("/inscription/{inscription_id}", get(Self::inscription))
            // Runes
            .route("/runes", get(Self::runes))
            .route("/rune/{rune}", get(Self::rune))
            .route("/rune/{rune}/transactions", get(Self::rune_transactions))
            // Mempool
            .route("/mempool/txids", get(Self::mempool_txids))
            // Mempool entries
            .route("/mempool/entry/{txid}", get(Self::mempool_tx))
            .route("/mempool/entries", post(Self::mempool_entries))
            .route("/mempool/entries/all", get(Self::mempool_all_entries))
            .route(
                "/mempool/entries/ancestors",
                post(Self::mempool_entries_with_ancestors),
            )
            // Subscriptions
            .route(
                "/subscription/{id}",
                get(Self::get_subscription).delete(Self::delete_subscription),
            )
            .route("/subscription", post(Self::add_subscription))
            .route("/subscriptions", get(Self::subscriptions))
            .merge(crate::api::alkanes::router())
            .layer(Extension(index))
            .layer(Extension(alkanes_indexer))
            .layer(Extension(webhook_subscription_manager))
            .layer(Extension(config.clone()))
            .layer(Extension(bitcoin_rpc_pool))
            .layer(
                CorsLayer::new()
                    .allow_methods([http::Method::GET])
                    .allow_origin(Any),
            )
            .layer(DefaultBodyLimit::disable())
            .layer(CompressionLayer::new())
            .with_state(config.clone());

        let jh = self.spawn(&config, router, handle)?;

        Ok(jh)
    }

    fn spawn(
        &self,
        config: &ServerConfig,
        router: Router,
        handle: Handle,
    ) -> SpawnResult<task::JoinHandle<io::Result<()>>> {
        let addr = config
            .http_listen
            .to_socket_addrs()?
            .next()
            .ok_or(SpawnError::NoSocketAddr)?;

        info!("Listening on http://{addr}");

        Ok(tokio::spawn(async move {
            axum_server::Server::bind(addr)
                .handle(handle)
                .serve(router.into_make_service())
                .await
        }))
    }

    async fn health_check() -> ServerResult {
        Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(serde_json::json!({"status": "ok"})),
        )
            .into_response())
    }

    async fn tip(Extension(index): Extension<Arc<Index>>) -> ServerResult {
        task::block_in_place(|| Ok(Json(api::tip(index)?).into_response()))
    }

    async fn status(Extension(index): Extension<Arc<Index>>) -> ServerResult {
        task::block_in_place(|| Ok(Json(api::status(index)?).into_response()))
    }

    async fn block(
        Extension(index): Extension<Arc<Index>>,
        Path(DeserializeFromStr(query)): Path<DeserializeFromStr<query::Block>>,
    ) -> ServerResult {
        task::block_in_place(|| Ok(Json(api::block(index, &query)?).into_response()))
    }

    async fn block_hash_by_height(
        Extension(index): Extension<Arc<Index>>,
        Path(DeserializeFromStr(height)): Path<DeserializeFromStr<u64>>,
    ) -> ServerResult {
        task::block_in_place(|| Ok(Json(api::block_hash_by_height(index, height)?).into_response()))
    }

    async fn block_txids(
        Extension(index): Extension<Arc<Index>>,
        Path(DeserializeFromStr(query)): Path<DeserializeFromStr<query::Block>>,
    ) -> ServerResult {
        task::block_in_place(|| Ok(Json(api::block_txids(index, &query)?).into_response()))
    }

    async fn broadcast_transaction(
        Extension(index): Extension<Arc<Index>>,
        Extension(bitcoin_rpc_pool): Extension<RpcClientPool>,
        hex: String,
    ) -> ServerResult {
        task::block_in_place(|| {
            let txid = api::broadcast_transaction(index, bitcoin_rpc_pool.get()?, &hex)?;

            Ok((
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/plain")],
                txid.to_string(),
            )
                .into_response())
        })
    }

    async fn transaction(
        Extension(index): Extension<Arc<Index>>,
        Extension(bitcoin_rpc_pool): Extension<RpcClientPool>,
        Path(txid): Path<SerializedTxid>,
    ) -> ServerResult {
        task::block_in_place(|| {
            let transaction = api::transaction(index, bitcoin_rpc_pool.get()?, &txid)?;
            Ok(Json(transaction).into_response())
        })
    }

    async fn transaction_raw(
        Extension(index): Extension<Arc<Index>>,
        Extension(bitcoin_rpc_pool): Extension<RpcClientPool>,
        Path(txid): Path<SerializedTxid>,
    ) -> ServerResult {
        task::block_in_place(|| {
            let raw_tx = api::bitcoin_transaction_raw(index, bitcoin_rpc_pool.get()?, &txid)?;

            Ok((
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/octet-stream")],
                Bytes::from(raw_tx),
            )
                .into_response())
        })
    }

    async fn transaction_hex(
        Extension(index): Extension<Arc<Index>>,
        Extension(bitcoin_rpc_pool): Extension<RpcClientPool>,
        Path(txid): Path<SerializedTxid>,
    ) -> ServerResult {
        task::block_in_place(|| {
            let hex_string = api::bitcoin_transaction_hex(index, bitcoin_rpc_pool.get()?, &txid)?;

            Ok((
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/plain")],
                hex_string,
            )
                .into_response())
        })
    }

    async fn transaction_status(
        Extension(index): Extension<Arc<Index>>,
        Path(txid): Path<SerializedTxid>,
    ) -> ServerResult {
        task::block_in_place(|| Ok(Json(api::transaction_status(index, &txid)?).into_response()))
    }

    async fn transaction_statuses(
        Extension(index): Extension<Arc<Index>>,
        Query(blockhash): Query<Option<BlockHash>>,
        txids: Json<Vec<SerializedTxid>>,
    ) -> ServerResult {
        task::block_in_place(|| {
            let result = api::transaction_statuses(index, &txids, &blockhash)?;

            Ok((
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                Json(result),
            )
                .into_response())
        })
    }

    async fn output(
        Extension(index): Extension<Arc<Index>>,
        Path(DeserializeFromStr(outpoint)): Path<DeserializeFromStr<SerializedOutPoint>>,
    ) -> ServerResult {
        task::block_in_place(|| Ok(Json(api::output(index, &outpoint)?).into_response()))
    }

    async fn runes(
        Extension(index): Extension<Arc<Index>>,
        Query(pagination): Query<Pagination>,
    ) -> ServerResult {
        task::block_in_place(|| Ok(Json(api::runes(index, pagination)?).into_response()))
    }

    async fn rune(
        Extension(index): Extension<Arc<Index>>,
        Path(DeserializeFromStr(rune)): Path<DeserializeFromStr<query::Rune>>,
    ) -> ServerResult {
        task::block_in_place(|| Ok(Json(api::rune(index, &rune)?).into_response()))
    }

    async fn rune_transactions(
        Extension(index): Extension<Arc<Index>>,
        Path(DeserializeFromStr(rune)): Path<DeserializeFromStr<query::Rune>>,
        Query(pagination): Query<Pagination>,
    ) -> ServerResult {
        task::block_in_place(|| {
            Ok(Json(api::last_rune_transactions(index, &rune, Some(pagination))?).into_response())
        })
    }

    async fn inscription(
        Extension(index): Extension<Arc<Index>>,
        Extension(config): Extension<Arc<ServerConfig>>,
        Path(DeserializeFromStr(inscription_id)): Path<DeserializeFromStr<InscriptionId>>,
        accept_encoding: AcceptEncoding,
    ) -> ServerResult {
        task::block_in_place(|| {
            Ok(api::inscription_content(
                index,
                &inscription_id,
                accept_encoding,
                config.csp_origin.clone(),
                config.decompress,
            )?
            .ok_or_not_found(|| format!("inscription {inscription_id} content"))?
            .into_response())
        })
    }

    async fn mempool_txids(Extension(index): Extension<Arc<Index>>) -> ServerResult {
        task::block_in_place(|| Ok(Json(api::mempool_txids(index)?).into_response()))
    }

    async fn mempool_tx(
        Extension(index): Extension<Arc<Index>>,
        Path(txid): Path<SerializedTxid>,
    ) -> ServerResult {
        task::block_in_place(|| Ok(Json(api::mempool_tx(index, &txid)?).into_response()))
    }

    async fn mempool_entries(
        Extension(index): Extension<Arc<Index>>,
        Json(txids): Json<Vec<SerializedTxid>>,
    ) -> ServerResult {
        task::block_in_place(|| Ok(Json(api::mempool_entries(index, &txids)?).into_response()))
    }

    async fn mempool_all_entries(Extension(index): Extension<Arc<Index>>) -> ServerResult {
        task::block_in_place(|| Ok(Json(api::mempool_entries_all(index)?).into_response()))
    }

    async fn mempool_entries_with_ancestors(
        Extension(index): Extension<Arc<Index>>,
        Json(txids): Json<Vec<SerializedTxid>>,
    ) -> ServerResult {
        task::block_in_place(|| {
            Ok(Json(api::mempool_entries_with_ancestors(index, &txids)?).into_response())
        })
    }

    async fn address(
        Extension(index): Extension<Arc<Index>>,
        Extension(config): Extension<Arc<ServerConfig>>,
        Path(address): Path<Address<NetworkUnchecked>>,
    ) -> ServerResult {
        if !config.index_addresses {
            return Err(ServerError::BadRequest(
                "addresses are not indexed. Enable --index-addresses to index addresses"
                    .to_string(),
            ));
        }

        let address = address
            .require_network(config.chain.network())
            .map_err(|err| ServerError::BadRequest(err.to_string()))?;

        task::block_in_place(|| Ok(Json(api::address(index, &address)?).into_response()))
    }

    async fn subscriptions(
        Extension(subscription_manager): Extension<Arc<WebhookSubscriptionManager>>,
        Extension(config): Extension<Arc<ServerConfig>>,
    ) -> ServerResult {
        if !config.enable_webhook_subscriptions {
            return Err(ServerError::BadRequest(
                "subscriptions are not enabled".to_string(),
            ));
        }

        task::block_in_place(|| Ok(Json(api::subscriptions(subscription_manager)?).into_response()))
    }

    async fn add_subscription(
        Extension(subscription_manager): Extension<Arc<WebhookSubscriptionManager>>,
        Extension(config): Extension<Arc<ServerConfig>>,
        Json(subscription): Json<Subscription>,
    ) -> ServerResult {
        if !config.enable_webhook_subscriptions {
            return Err(ServerError::BadRequest(
                "subscriptions are not enabled".to_string(),
            ));
        }

        task::block_in_place(|| {
            Ok(Json(api::add_subscription(subscription_manager, subscription)?).into_response())
        })
    }

    async fn delete_subscription(
        Extension(subscription_manager): Extension<Arc<WebhookSubscriptionManager>>,
        Extension(config): Extension<Arc<ServerConfig>>,
        Path(id): Path<Uuid>,
    ) -> ServerResult {
        if !config.enable_webhook_subscriptions {
            return Err(ServerError::BadRequest(
                "subscriptions are not enabled".to_string(),
            ));
        }

        task::block_in_place(|| {
            Ok(Json(api::delete_subscription(subscription_manager, id)?).into_response())
        })
    }

    async fn get_subscription(
        Extension(subscription_manager): Extension<Arc<WebhookSubscriptionManager>>,
        Extension(config): Extension<Arc<ServerConfig>>,
        Path(id): Path<Uuid>,
    ) -> ServerResult {
        if !config.enable_webhook_subscriptions {
            return Err(ServerError::BadRequest(
                "subscriptions are not enabled".to_string(),
            ));
        }

        task::block_in_place(|| {
            Ok(Json(api::get_subscription(subscription_manager, id)?).into_response())
        })
    }
}

impl<S> axum::extract::FromRequestParts<S> for AcceptEncoding
where
    Arc<ServerConfig>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(parts.headers.get("accept-encoding").map(|value| {
            value.to_str().unwrap_or_default().to_owned()
        })))
    }
}
