use rustc_hash::FxHashMap as HashMap;

use bitcoin::{ScriptBuf, Transaction};
use ordinals::Rune;
use titan_types::{
    InscriptionId, RuneId, SerializedOutPoint, SerializedTxid, SpenderReference, TxOut,
};

use crate::{
    index::StoreError,
    models::{
        BlockId, Inscription, RuneEntry, TransactionStateChange, TransactionStateChangeInput,
    },
};

pub trait TransactionStore {
    fn get_tx_outs(
        &mut self,
        outpoints: &[SerializedOutPoint],
    ) -> Result<HashMap<SerializedOutPoint, TxOut>, StoreError>;
    fn set_tx_out(&mut self, outpoint: SerializedOutPoint, tx_out: TxOut, script_pubkey: ScriptBuf);
    fn set_spent_tx_out(
        &mut self,
        outpoint: &TransactionStateChangeInput,
        spent: SpenderReference,
    ) -> Result<(), StoreError>;
    fn get_transaction(&self, txid: &SerializedTxid) -> Result<Transaction, StoreError>;
    fn set_transaction(&mut self, txid: SerializedTxid, transaction: Transaction);
    fn get_transaction_confirming_block(
        &self,
        txid: &SerializedTxid,
    ) -> Result<BlockId, StoreError>;
    fn set_transaction_confirming_block(&mut self, txid: SerializedTxid, block_id: BlockId);
    fn get_rune(&mut self, rune_id: &RuneId) -> Result<RuneEntry, StoreError>;
    fn does_rune_exist(&mut self, rune: &Rune) -> Result<(), StoreError>;
    fn get_runes_count(&self) -> u64;
    fn set_rune_id(&mut self, rune: Rune, id: RuneId);
    fn set_rune(&mut self, id: RuneId, entry: RuneEntry);
    fn set_rune_id_number(&mut self, number: u64, id: RuneId);
    fn increment_runes_count(&mut self);
    fn set_inscription(&mut self, id: InscriptionId, inscription: Inscription);
    fn set_tx_state_changes(
        &mut self,
        txid: SerializedTxid,
        tx_state_changes: TransactionStateChange,
    );
    fn add_rune_transaction(&mut self, rune_id: RuneId, txid: SerializedTxid);
}
