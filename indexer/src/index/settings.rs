use {
    super::*,
    crate::bitcoin_rpc::{RpcClientError, RpcClientProvider},
    bitcoincore_rpc::{Auth, Client},
    std::path::PathBuf,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub(crate) data_dir: PathBuf,
    pub(crate) zmq_endpoint: String,
    pub(crate) bitcoin_rpc_limit: u32,
    pub(crate) bitcoin_rpc_url: String,
    pub(crate) bitcoin_rpc_auth: Auth,
    pub(crate) chain: Chain,
    pub(crate) no_index_inscriptions: bool,
    pub(crate) index_bitcoin_transactions: bool,
    pub(crate) index_spent_outputs: bool,
    pub(crate) index_addresses: bool,
    pub(crate) commit_interval: u64,
    pub(crate) main_loop_interval: u64,
}   

impl RpcClientProvider for Settings {
    fn get_new_rpc_client(&self) -> Result<Client, RpcClientError> {
        Client::new(&self.bitcoin_rpc_url, self.bitcoin_rpc_auth.clone())
            .map_err(|e| RpcClientError::FailedToConnect(e.to_string()))
    }
}

impl Settings {
    pub fn max_recoverable_reorg_depth(&self) -> u64 {
        match self.chain {
            Chain::Mainnet => 10,
            Chain::Testnet => 100,
            Chain::Testnet4 => 100,
            Chain::Regtest => 100,
            Chain::Signet => 100,
        }
    }
}
