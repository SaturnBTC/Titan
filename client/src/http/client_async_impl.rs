use bitcoin::{OutPoint, Txid};
use reqwest::{header::HeaderMap, Client as AsyncReqwestClient};
use std::{collections::HashMap, str::FromStr};
use titan_types_api::*;
use titan_types_core::*;

use crate::Error;

use super::TitanApiAsync;

#[derive(Clone)]
pub struct AsyncClient {
    /// The async HTTP client from `reqwest`.
    http_client: AsyncReqwestClient,
    /// The base URL for all endpoints (e.g. http://localhost:3030).
    base_url: String,
}

impl AsyncClient {
    /// Creates a new `AsyncClient` for the given `base_url`.
    pub fn new(base_url: &str) -> Self {
        Self {
            http_client: AsyncReqwestClient::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    async fn call_text(&self, path: &str) -> Result<String, Error> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.http_client.get(&url).send().await?;
        if response.status().is_success() {
            Ok(response.text().await?)
        } else {
            Err(Error::TitanError(response.status(), response.text().await?))
        }
    }

    async fn call_bytes(&self, path: &str) -> Result<Vec<u8>, Error> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.http_client.get(&url).send().await?;
        if response.status().is_success() {
            Ok(response.bytes().await?.to_vec())
        } else {
            Err(Error::TitanError(response.status(), response.text().await?))
        }
    }

    async fn post_text(&self, path: &str, body: String) -> Result<String, Error> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.http_client.post(&url).body(body).send().await?;
        if response.status().is_success() {
            Ok(response.text().await?)
        } else {
            Err(Error::TitanError(response.status(), response.text().await?))
        }
    }

    async fn delete(&self, path: &str) -> Result<(), Error> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.http_client.delete(&url).send().await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::TitanError(response.status(), response.text().await?))
        }
    }
}

#[async_trait::async_trait]
impl TitanApiAsync for AsyncClient {
    async fn get_status(&self) -> Result<Status, Error> {
        let text = self.call_text("/status").await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_tip(&self) -> Result<BlockTip, Error> {
        let text = self.call_text("/tip").await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_block(&self, query: &query::Block) -> Result<Block, Error> {
        let text = self.call_text(&format!("/block/{}", query)).await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_block_hash_by_height(&self, height: u64) -> Result<String, Error> {
        self.call_text(&format!("/block/{}/hash", height)).await
    }

    async fn get_block_txids(&self, query: &query::Block) -> Result<Vec<String>, Error> {
        let text = self.call_text(&format!("/block/{}/txids", query)).await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_address(&self, address: &str) -> Result<AddressData, Error> {
        let text = self.call_text(&format!("/address/{}", address)).await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_transaction(&self, txid: &Txid) -> Result<Transaction, Error> {
        let text = self.call_text(&format!("/tx/{}", txid)).await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_transaction_raw(&self, txid: &Txid) -> Result<Vec<u8>, Error> {
        self.call_bytes(&format!("/tx/{}/raw", txid)).await
    }

    async fn get_transaction_hex(&self, txid: &Txid) -> Result<String, Error> {
        self.call_text(&format!("/tx/{}/hex", txid)).await
    }

    async fn get_transaction_status(&self, txid: &Txid) -> Result<TransactionStatus, Error> {
        let text = self.call_text(&format!("/tx/{}/status", txid)).await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn send_transaction(&self, tx_hex: String) -> Result<Txid, Error> {
        let text = self.post_text("/tx/broadcast", tx_hex).await?;
        Txid::from_str(&text).map_err(Error::from)
    }

    async fn get_output(&self, outpoint: &OutPoint) -> Result<TxOut, Error> {
        let text = self.call_text(&format!("/output/{}", outpoint)).await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_inscription(
        &self,
        inscription_id: &InscriptionId,
    ) -> Result<(HeaderMap, Vec<u8>), Error> {
        let url = format!("{}/inscription/{}", self.base_url, inscription_id);
        let resp = self.http_client.get(&url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::TitanError(status, body));
        }
        let headers = resp.headers().clone();
        let bytes = resp.bytes().await?.to_vec();
        Ok((headers, bytes))
    }

    async fn get_runes(
        &self,
        pagination: Option<Pagination>,
    ) -> Result<PaginationResponse<RuneResponse>, Error> {
        let mut path = "/runes".to_string();
        if let Some(p) = pagination {
            path = format!("{}?skip={}&limit={}", path, p.skip, p.limit);
        }
        let text = self.call_text(&path).await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_rune(&self, rune: &query::Rune) -> Result<RuneResponse, Error> {
        let text = self.call_text(&format!("/rune/{}", rune)).await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_rune_transactions(
        &self,
        rune: &query::Rune,
        pagination: Option<Pagination>,
    ) -> Result<PaginationResponse<Txid>, Error> {
        let mut path = format!("/rune/{}/transactions", rune);
        if let Some(p) = pagination {
            path = format!("{}?skip={}&limit={}", path, p.skip, p.limit);
        }
        let text = self.call_text(&path).await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_mempool_txids(&self) -> Result<Vec<Txid>, Error> {
        let text = self.call_text("/mempool/txids").await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_mempool_entry(&self, txid: &Txid) -> Result<MempoolEntry, Error> {
        let text = self.call_text(&format!("/mempool/entry/{}", txid)).await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_mempool_entries(
        &self,
        txids: &[Txid],
    ) -> Result<HashMap<Txid, Option<MempoolEntry>>, Error> {
        let text = self
            .post_text("/mempool/entries", serde_json::to_string(txids)?)
            .await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_all_mempool_entries(&self) -> Result<HashMap<Txid, MempoolEntry>, Error> {
        let text = self.call_text("/mempool/entries/all").await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_mempool_entries_with_ancestors(
        &self,
        txids: &[Txid],
    ) -> Result<HashMap<Txid, MempoolEntry>, Error> {
        let url = format!("{}/mempool/entries/ancestors", self.base_url);
        let response = self.http_client.post(&url).json(txids).send().await?;

        if response.status().is_success() {
            let text = response.text().await?;
            serde_json::from_str(&text).map_err(Error::from)
        } else {
            Err(Error::TitanError(response.status(), response.text().await?))
        }
    }

    async fn get_subscription(&self, id: &str) -> Result<Subscription, Error> {
        let text = self.call_text(&format!("/subscription/{}", id)).await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn list_subscriptions(&self) -> Result<Vec<Subscription>, Error> {
        let text = self.call_text("/subscriptions").await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn add_subscription(&self, subscription: &Subscription) -> Result<Subscription, Error> {
        let text = self
            .post_text("/subscription", serde_json::to_string(subscription)?)
            .await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn delete_subscription(&self, id: &str) -> Result<(), Error> {
        self.delete(&format!("/subscription/{}", id)).await
    }
}
