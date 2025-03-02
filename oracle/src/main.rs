mod mempool;
mod mempool_smart_contract;

use arch_sdk::arch_program::pubkey::Pubkey;
use clap::Parser;
use hex;
use mempool::MempoolService;
use mempool::MempoolServiceConfig;
use mempool_smart_contract::utils::SmartContractConfig;
use std::time::Duration;
use tokio::{
    select,
    signal::unix::{signal, SignalKind},
    time::sleep,
};
use tracing::{info, Level};

/// Bitcoin mempool oracle service
///
/// This service maintains a real-time view of the Bitcoin mempool
/// using both HTTP and TCP connections to a titan-indexer instance.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// HTTP endpoint for the titan-indexer API
    #[arg(short, long, default_value = "http://localhost:8000")]
    http_endpoint: String,

    /// TCP endpoint for the titan-indexer subscription service
    #[arg(short, long, default_value = "localhost:8001")]
    tcp_endpoint: String,

    /// Arch API endpoint for the mempool oracle smart contract
    #[arg(short, long, default_value = "http://localhost:8899")]
    arch_api_endpoint: String,

    /// Refresh interval in seconds for the periodic mempool refresh
    #[arg(short, long, default_value_t = 300)]
    refresh_interval: u64,

    /// Enable verbose logging (including detailed mempool operations)
    #[arg(short, long)]
    verbose: bool,

    /// Batch size for processing large mempool updates
    #[arg(long, default_value_t = 100)]
    batch_size: usize,

    /// Set the log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Status update interval in seconds (how often to log mempool size)
    #[arg(long, default_value_t = 30)]
    status_interval: u64,

    /// Enable smart contract updates (requires program_id and account_id)
    #[arg(long)]
    enable_smart_contract: bool,

    /// Program ID for the mempool oracle smart contract (required if enable_smart_contract is set)
    #[arg(long)]
    program_id: Option<String>,

    /// Account ID that stores the mempool data (required if enable_smart_contract is set)
    #[arg(long)]
    account_id: Option<String>,

    /// Maximum batch size for smart contract updates
    #[arg(long, default_value_t = 50)]
    sc_batch_size: usize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command-line arguments
    let args = Args::parse();

    // Set up logging
    let log_level = match args.log_level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    if args.verbose {
        tracing_subscriber::fmt().with_max_level(log_level).init();
    } else {
        tracing_subscriber::fmt().with_max_level(log_level).init();
    }

    info!("Starting oracle with configuration:");
    info!("  HTTP endpoint: {}", args.http_endpoint);
    info!("  TCP endpoint: {}", args.tcp_endpoint);
    info!("  Refresh interval: {} seconds", args.refresh_interval);
    info!("  Batch size: {}", args.batch_size);
    info!("  Verbose logging: {}", args.verbose);

    // Set up smart contract config if enabled
    let smart_contract_config = if args.enable_smart_contract {
        // Check required parameters
        if args.program_id.is_none() || args.account_id.is_none() {
            return Err("When enable_smart_contract is set, both program_id and account_id must be provided".into());
        }

        let program_id_str = args.program_id.as_ref().unwrap();
        let account_id_str = args.account_id.as_ref().unwrap();

        // Parse pubkeys from hex strings
        let program_id = match hex::decode(program_id_str) {
            Ok(bytes) => {
                if bytes.len() != 32 {
                    return Err(format!("Invalid program_id length: {}", program_id_str).into());
                }
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Pubkey::from_slice(&arr)
            }
            Err(_) => return Err(format!("Invalid program_id hex: {}", program_id_str).into()),
        };

        let account_id = match hex::decode(account_id_str) {
            Ok(bytes) => {
                if bytes.len() != 32 {
                    return Err(format!("Invalid account_id length: {}", account_id_str).into());
                }
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Pubkey::from_slice(&arr)
            }
            Err(_) => return Err(format!("Invalid account_id hex: {}", account_id_str).into()),
        };

        info!("Smart contract updates enabled:");
        info!("  Program ID: {}", program_id_str);
        info!("  Account ID: {}", account_id_str);
        info!("  Smart contract batch size: {}", args.sc_batch_size);

        Some(SmartContractConfig {
            program_id,
            mempool_account: account_id,
            max_batch_size: args.sc_batch_size,
            verbose_logging: args.verbose,
            arch_api_endpoint: args.arch_api_endpoint,
        })
    } else {
        None
    };

    // Create mempool service configuration
    let mempool_config = MempoolServiceConfig {
        batch_size: args.batch_size,
        verbose_logging: args.verbose,
        smart_contract_config,
    };

    // Create and start the mempool service
    let mempool_service =
        MempoolService::with_config(&args.http_endpoint, &args.tcp_endpoint, mempool_config)
            .await?;

    // Start periodic refresh
    mempool_service.start_periodic_refresh(Duration::from_secs(args.refresh_interval));

    info!("Mempool service started");

    // Setup signal handlers for graceful shutdown
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    // Main loop with signal handling
    loop {
        select! {
            _ = sleep(Duration::from_secs(args.status_interval)) => {
                info!("Current mempool size: {} transactions", mempool_service.mempool_size());
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, shutting down gracefully...");
                mempool_service.shutdown();
                break;
            }
            _ = sigint.recv() => {
                info!("Received SIGINT (Ctrl+C), shutting down gracefully...");
                mempool_service.shutdown();
                break;
            }
        }
    }

    info!("Oracle service shutdown complete");
    Ok(())
}
