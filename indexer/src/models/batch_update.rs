use {
    super::{BlockId, Inscription, RuneEntry, TransactionStateChange},
    bitcoin::{BlockHash, ScriptBuf, Transaction},
    ordinals::RuneId,
    rustc_hash::FxHashMap as HashMap,
    std::fmt::Display,
    titan_types::{
        Block, InscriptionId, MempoolEntry, SerializedOutPoint, SerializedTxid, SpenderReference,
        TxOut,
    },
};

#[derive(Debug, Clone, Default)]
pub struct BatchUpdate {
    pub script_pubkeys: HashMap<ScriptBuf, (Vec<SerializedOutPoint>, Vec<SerializedOutPoint>)>,
    pub script_pubkeys_outpoints: HashMap<SerializedOutPoint, ScriptBuf>,
    pub spent_outpoints_in_mempool: HashMap<SerializedOutPoint, SpenderReference>,
    pub blocks: HashMap<BlockHash, Block>,
    pub block_hashes: HashMap<u64, BlockHash>,
    pub txouts: HashMap<SerializedOutPoint, TxOut>,
    pub tx_state_changes: HashMap<SerializedTxid, TransactionStateChange>,
    pub rune_transactions: HashMap<RuneId, Vec<SerializedTxid>>,
    pub runes: HashMap<RuneId, RuneEntry>,
    pub rune_ids: HashMap<u128, RuneId>,
    pub rune_numbers: HashMap<u64, RuneId>,
    pub inscriptions: HashMap<InscriptionId, Inscription>,
    pub transactions: HashMap<SerializedTxid, Transaction>,
    pub transaction_confirming_block: HashMap<SerializedTxid, BlockId>,
    pub mempool_txs: HashMap<SerializedTxid, MempoolEntry>,
    pub rune_count: u64,
    pub block_count: u64,
    pub purged_blocks_count: u64,
    pub misc: HashMap<Vec<u8>, Vec<u8>>,
}

impl BatchUpdate {
    pub fn new(rune_count: u64, block_count: u64, purged_blocks_count: u64) -> Self {
        Self {
            script_pubkeys: HashMap::default(),
            script_pubkeys_outpoints: HashMap::default(),
            spent_outpoints_in_mempool: HashMap::default(),
            blocks: HashMap::default(),
            block_hashes: HashMap::default(),
            txouts: HashMap::default(),
            tx_state_changes: HashMap::default(),
            rune_transactions: HashMap::default(),
            runes: HashMap::default(),
            rune_ids: HashMap::default(),
            rune_numbers: HashMap::default(),
            inscriptions: HashMap::default(),
            transactions: HashMap::default(),
            transaction_confirming_block: HashMap::default(),
            mempool_txs: HashMap::default(),
            rune_count,
            block_count,
            purged_blocks_count,
            misc: HashMap::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.script_pubkeys.is_empty()
            && self.script_pubkeys_outpoints.is_empty()
            && self.spent_outpoints_in_mempool.is_empty()
            && self.blocks.is_empty()
            && self.block_hashes.is_empty()
            && self.txouts.is_empty()
            && self.tx_state_changes.is_empty()
            && self.rune_transactions.is_empty()
            && self.runes.is_empty()
            && self.rune_ids.is_empty()
            && self.rune_numbers.is_empty()
            && self.inscriptions.is_empty()
            && self.mempool_txs.is_empty()
            && self.transactions.is_empty()
            && self.transaction_confirming_block.is_empty()
            && self.misc.is_empty()
    }

    /// Removes all per-batch collections while preserving the counter fields.
    /// This lets the `HashMap`s keep their capacity, significantly reducing
    /// the number of allocations that occur during repetitive flushing.
    pub fn clear(&mut self) {
        self.script_pubkeys.clear();
        self.script_pubkeys_outpoints.clear();
        self.spent_outpoints_in_mempool.clear();
        self.blocks.clear();
        self.block_hashes.clear();
        self.txouts.clear();
        self.tx_state_changes.clear();
        self.rune_transactions.clear();
        self.runes.clear();
        self.rune_ids.clear();
        self.rune_numbers.clear();
        self.inscriptions.clear();
        self.mempool_txs.clear();
        self.transactions.clear();
        self.transaction_confirming_block.clear();
        self.misc.clear();
    }
}

impl Display for BatchUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "BatchUpdate: \
             counts: [blocks: {}, runes: {}, purged_blocks: {}] \
             added: [blocks: {}, txouts: {}, tx_changes: {}, \
             addresses: {} , address_outpoints: {}, \
             spent_outpoints_in_mempool: {}, \
             mempool_txs: {}, \
             runes: txs {}/ runes {}/ ids {}, \
             inscriptions: {}, \
             transactions: {}, \
             transaction_confirming_block: {}, misc: {}]",
            self.block_count,
            self.rune_count,
            self.purged_blocks_count,
            self.blocks.len(),
            self.txouts.len(),
            self.tx_state_changes.len(),
            self.script_pubkeys.len(),
            self.script_pubkeys_outpoints.len(),
            self.spent_outpoints_in_mempool.len(),
            self.mempool_txs.len(),
            self.rune_transactions.len(),
            self.runes.len(),
            self.rune_ids.len(),
            self.inscriptions.len(),
            self.transactions.len(),
            self.transaction_confirming_block.len(),
            self.misc.len(),
        )
    }
}
