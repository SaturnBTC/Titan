use {
    crate::{
        index::{Chain, Settings},
        server::ServerConfig,
    },
    bitcoincore_rpc::Auth,
    clap::{
        arg,
        builder::{
            styling::{AnsiColor, Effects},
            Styles,
        },
        Parser,
    },
    std::path::PathBuf,
    tracing::warn,
};

#[derive(Clone, Default, Debug, Parser)]
#[command(
    name = "rune-indexer",
    about = "A minimal Rune indexer",
    version,
    styles = Styles::styled()
      .error(AnsiColor::Red.on_default() | Effects::BOLD)
      .header(AnsiColor::Yellow.on_default() | Effects::BOLD)
      .invalid(AnsiColor::Red.on_default())
      .literal(AnsiColor::Blue.on_default())
      .placeholder(AnsiColor::Cyan.on_default())
      .usage(AnsiColor::Yellow.on_default() | Effects::BOLD)
      .valid(AnsiColor::Green.on_default()),
  )]
pub struct Options {
    #[arg(
        long,
        help = "Authenticate to Bitcoin Core RPC with <BITCOIN_RPC_PASSWORD>."
    )]
    pub(super) bitcoin_rpc_password: Option<String>,
    #[arg(
        long,
        help = "Connect to Bitcoin Core RPC at <BITCOIN_RPC_URL>.",
        default_value = "http://localhost:8332"
    )]
    pub(super) bitcoin_rpc_url: String,
    #[arg(
        long,
        help = "Authenticate to Bitcoin Core RPC as <BITCOIN_RPC_USERNAME>."
    )]
    pub(super) bitcoin_rpc_username: Option<String>,
    #[arg(
        long,
        help = "Max <N> requests in flight. [default: 12]",
        default_value = "12"
    )]
    pub(super) bitcoin_rpc_limit: u32,
    #[arg(
        long = "chain",
        value_enum,
        help = "Use <CHAIN>. [default: mainnet]",
        default_value = "mainnet"
    )]
    pub(super) chain: Chain,
    #[arg(long, help = "Load configuration from <CONFIG>.")]
    pub(super) config: Option<PathBuf>,
    #[arg(long, help = "Load Bitcoin Core RPC cookie file from <COOKIE_FILE>.")]
    pub(super) cookie_file: Option<PathBuf>,

    /// Store index in <DATA_DIR>. [default: ./data]
    #[arg(
        long,
        alias = "datadir",
        help = "Store index in <DATA_DIR>.",
        default_value = "./data"
    )]
    pub(super) data_dir: PathBuf,

    /// Do not index inscriptions (rune icons). [default: false]
    #[arg(
        long,
        short,
        alias = "noindex_inscriptions",
        help = "Do not index inscriptions (rune icons)."
    )]
    pub(super) no_index_inscriptions: bool,

    /// Index spent outputs. [default: false]
    #[arg(
        long,
        short,
        help = "Index spent outputs. [default: false]",
        default_value = "false"
    )]
    pub(super) index_spent_outputs: bool,

    /// Index addresses. [default: false]
    #[arg(
        long,
        short,
        help = "Index addresses. [default: false]",
        default_value = "false"
    )]
    pub(super) index_addresses: bool,

    /// Commit interval in blocks. [default: 5000]
    #[arg(
        long,
        help = "Commit interval in blocks. [default: 5000]",
        default_value = "5000"
    )]
    pub(super) commit_interval: u64,

    /// HTTP Event destination.
    #[arg(long, help = "HTTP Event destination")]
    pub(super) http_event_destination: Option<String>,

    /// ZeroMQ endpoint for raw blocks/transactions from bitcoind
    #[arg(long, default_value = "tcp://127.0.0.1:28332")]
    pub(super) zmq_endpoint: String,

    /// Listen address for the REST API server
    #[arg(long, default_value = "0.0.0.0:3030")]
    pub(super) http_listen: String,

    #[arg(
        long,
        help = "Use <CSP_ORIGIN> in Content-Security-Policy header. Set this to the public-facing URL of your rune-indexer instance."
    )]
    pub(super) csp_origin: Option<String>,

    #[arg(
        long,
        help = "Decompress encoded content. Currently only supports brotli. Be careful using this on production instances. A decompressed inscription may be arbitrarily large, making decompression a DoS vector."
    )]
    pub(super) decompress: bool,

    /// Main loop interval in milliseconds. [default: 500]
    #[arg(
        long,
        default_value = "500",
        help = "Main loop interval in milliseconds. [default: 500]"
    )]
    pub(super) main_loop_interval: u64,
}

impl Options {
    pub fn get_bitcoin_rpc_auth(&self) -> Auth {
        let bitcoin_rpc_auth = if let Some(cookie_file) = self.cookie_file.as_ref() {
            Auth::CookieFile(cookie_file.clone())
        } else if let Some(bitcoin_rpc_username) = self.bitcoin_rpc_username.as_ref() {
            Auth::UserPass(
                bitcoin_rpc_username.clone(),
                self.bitcoin_rpc_password.clone().unwrap_or_default(),
            )
        } else {
            warn!("No authentication provided for Bitcoin Core RPC");
            Auth::None
        };

        bitcoin_rpc_auth
    }
}

impl From<Options> for Settings {
    fn from(options: Options) -> Self {
        let bitcoin_rpc_auth = options.get_bitcoin_rpc_auth();

        Self {
            data_dir: options.data_dir,
            zmq_endpoint: options.zmq_endpoint,
            bitcoin_rpc_limit: options.bitcoin_rpc_limit,
            bitcoin_rpc_url: options.bitcoin_rpc_url,
            bitcoin_rpc_auth,
            chain: options.chain,
            no_index_inscriptions: options.no_index_inscriptions,
            index_spent_outputs: options.index_spent_outputs,
            index_addresses: options.index_addresses,
            commit_interval: options.commit_interval,
            http_event_destination: options.http_event_destination,
            main_loop_interval: options.main_loop_interval,
        }
    }
}

impl From<Options> for ServerConfig {
    fn from(options: Options) -> Self {
        let bitcoin_rpc_auth = options.get_bitcoin_rpc_auth();
        Self {
            chain: options.chain,
            csp_origin: options.csp_origin,
            decompress: options.decompress,

            http_listen: options.http_listen,
            bitcoin_rpc_url: options.bitcoin_rpc_url,
            bitcoin_rpc_auth,
        }
    }
}
