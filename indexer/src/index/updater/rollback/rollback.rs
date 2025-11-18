use {
    super::rollback_cache::RollbackCache,
    crate::{
        index::{store::Store, Settings, StoreError},
        models::{TransactionStateChange, TransactionStateChangeInput},
    },
    bitcoin::ScriptBuf,
    rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet},
    std::sync::Arc,
    thiserror::Error,
    titan_types_core::RuneId,
    titan_types_core::{InscriptionId, SerializedOutPoint, SerializedTxid, SpentStatus},
    tracing::{info, warn},
};

#[derive(Debug, Error)]
pub enum RollbackError {
    #[error("store error {0}")]
    Store(#[from] StoreError),
    #[error("overflow in {0}")]
    Overflow(String),
}

type Result<T> = std::result::Result<T, RollbackError>;

pub struct RollbackSettings {
    pub index_addresses: bool,
}

impl From<Settings> for RollbackSettings {
    fn from(settings: Settings) -> Self {
        Self {
            index_addresses: settings.index_addresses,
        }
    }
}

pub struct Rollback<'a> {
    store: &'a Arc<dyn Store + Send + Sync>,
    settings: RollbackSettings,
    cache: RollbackCache<'a>,
}

impl<'a> Rollback<'a> {
    pub fn new(
        store: &'a Arc<dyn Store + Send + Sync>,
        settings: RollbackSettings,
        mempool: bool,
    ) -> Result<Self> {
        let cache = RollbackCache::new(store, mempool)?;
        Ok(Self {
            store,
            settings,
            cache,
        })
    }

    pub fn revert_transactions(&mut self, txids: &Vec<SerializedTxid>) -> Result<()> {
        let txs_state_changes = self
            .store
            .get_txs_state_changes(txids, self.cache.mempool)?;

        self.precache_transactions(&txs_state_changes)?;

        for txid in txids.iter().rev() {
            info!("Reverting transaction {}", txid);
            let transaction = txs_state_changes.get(txid);
            // .ok_or(RollbackError::Store(StoreError::NotFound(format!(
            //     "transaction not found: {}",
            //     txid
            // ))))?;

            if let Some(transaction) = transaction {
                self.revert_transaction(txid, transaction)?;
            } else {
                warn!("Transaction to rollback not found: {}", txid);
                if self.cache.mempool {
                    self.cache.add_tx_to_delete(txid.clone());
                }
            }
        }

        if self.settings.index_addresses {
            self.revert_transactions_script_pubkeys_modifications(&txs_state_changes)?;
        }

        self.cache.flush()?;

        Ok(())
    }

    fn precache_transactions(
        &mut self,
        txs_state_changes: &HashMap<SerializedTxid, TransactionStateChange>,
    ) -> Result<()> {
        let mut tx_outs = Vec::new();
        let mut runes = HashSet::default();

        for (_, tx) in txs_state_changes.iter() {
            tx_outs.extend(tx.inputs.iter().map(|input| input.previous_outpoint));

            if let Some(mint) = &tx.minted {
                runes.insert(mint.id);
            }

            for (rune_id, _) in tx.burned.iter() {
                runes.insert(*rune_id);
            }

            if let Some((id, _)) = tx.etched {
                runes.insert(id);
            }
        }

        self.cache.precache_tx_outs(&tx_outs)?;
        self.cache.precache_runes(&runes.into_iter().collect())?;

        Ok(())
    }

    fn revert_transaction(
        &mut self,
        txid: &SerializedTxid,
        transaction: &TransactionStateChange,
    ) -> Result<()> {
        // Make spendable the inputs again.
        for tx_in in transaction.inputs.iter() {
            self.update_spendable_input(&tx_in, SpentStatus::Unspent)?;
        }

        // Remove tx_outs
        for (vout, _tx_out) in transaction.outputs.iter().enumerate() {
            let outpoint = SerializedOutPoint::from_txid_vout(txid, vout as u32);
            self.cache.add_outpoint_to_delete(outpoint);
        }

        // Remove etched rune if any.
        // if a reorg has happened and a rune was etched but it doesn't exist now,
        // we need to remove all mints and transfer edicts from other transactions
        // that might have happened.
        if !self.cache.mempool {
            if let Some((id, rune)) = transaction.etched {
                let rune_entry = self.cache.get_rune(&id);
                if let Some(rune_entry) = rune_entry {
                    self.cache.add_rune_to_delete(id);
                    self.cache.add_rune_id_to_delete(rune);
                    self.cache.add_inscription_to_delete(InscriptionId {
                        txid: txid.clone(),
                        index: 0,
                    });

                    // Decrease rune count
                    self.cache.decrement_runes_count();
                    self.cache.add_rune_number_to_delete(rune_entry.number);

                    // self.store.delete_rune_id_number(rune_entry.number)?;
                    // self.update_rune_numbers_after_revert(rune_entry.number, self.runes)?;

                    // Delete all rune transactions
                    self.cache.add_delete_all_rune_transactions(id);
                }
            }
        }

        // Remove mints if any.
        if let Some(mint) = transaction.minted.as_ref() {
            self.decrement_mint(&mint.id)?;
        }

        // Remove burned if any.
        for (rune_id, amount) in transaction.burned.iter() {
            self.update_burn_balance(rune_id, -(amount.n() as i128))?;
        }

        // Finally remove the transaction.
        self.cache.add_tx_to_delete(txid.clone());

        Ok(())
    }

    fn update_spendable_input(
        &mut self,
        outpoint: &TransactionStateChangeInput,
        spent: SpentStatus,
    ) -> Result<()> {
        match self.cache.get_tx_out(&outpoint.previous_outpoint) {
            Ok(tx_out) => {
                let mut tx_out = tx_out;
                tx_out.spent = spent;
                self.cache.set_tx_out(outpoint.previous_outpoint, tx_out);
                Ok(())
            }
            Err(StoreError::NotFound(_)) => {
                // We don't need to do anything.
                Ok(())
            }
            Err(e) => {
                return Err(RollbackError::Store(e));
            }
        }
    }

    fn decrement_mint(&mut self, rune_id: &RuneId) -> Result<()> {
        let rune_entry = self.cache.get_rune(rune_id);

        if let Some(mut rune_entry) = rune_entry {
            if self.cache.mempool {
                let result = rune_entry.pending_mints.saturating_add_signed(-1);
                rune_entry.pending_mints = result;
            } else {
                let result = rune_entry.mints.saturating_add_signed(-1);

                rune_entry.mints = result;
            }

            self.cache.set_rune(rune_id.clone(), rune_entry);
        }

        Ok(())
    }

    fn update_burn_balance(&mut self, rune_id: &RuneId, amount: i128) -> Result<()> {
        let rune_entry = self.cache.get_rune(rune_id);

        if let Some(mut rune_entry) = rune_entry {
            if self.cache.mempool {
                let result = rune_entry
                    .pending_burns
                    .checked_add_signed(amount)
                    .ok_or(RollbackError::Overflow("burn".to_string()))?;
                rune_entry.pending_burns = result;
            } else {
                let result = rune_entry
                    .burned
                    .checked_add_signed(amount)
                    .ok_or(RollbackError::Overflow("burn".to_string()))?;

                rune_entry.burned = result;
            }

            self.cache.set_rune(rune_id.clone(), rune_entry);
        }

        Ok(())
    }

    fn revert_transactions_script_pubkeys_modifications(
        &mut self,
        tx_to_state_changes: &HashMap<SerializedTxid, TransactionStateChange>,
    ) -> Result<()> {
        // Update script pubkey entries.
        let mut script_pubkey_entries: HashMap<
            ScriptBuf,
            (Vec<SerializedOutPoint>, Vec<SerializedOutPoint>),
        > = HashMap::default();

        // Delete outpoints.
        let mut spent_outpoints = HashSet::default();
        for (txid, tx) in tx_to_state_changes.iter() {
            for (vout, output) in tx.outputs.iter().enumerate() {
                let script_pubkey = output.script_pubkey.clone();
                let entry = script_pubkey_entries.entry(script_pubkey).or_default();

                let outpoint = SerializedOutPoint::from_txid_vout(txid, vout as u32);
                // Add "spent" outpoint.
                entry.1.push(outpoint);
                spent_outpoints.insert(outpoint);
            }

            if !self.cache.mempool {
                for input in tx.inputs.iter() {
                    if let Some(script_pubkey) = input.script_pubkey.clone() {
                        let entry = script_pubkey_entries.entry(script_pubkey).or_default();

                        if !spent_outpoints.contains(&input.previous_outpoint) {
                            entry.0.push(input.previous_outpoint);
                        }
                    }
                }
            }
        }

        if self.cache.mempool {
            // Remove spent outpoints.
            let prev_outpoints = tx_to_state_changes
                .values()
                .filter(|tx| !tx.is_coinbase)
                .flat_map(|tx| tx.inputs.iter().map(|input| input.previous_outpoint))
                .collect::<Vec<_>>();

            self.cache.add_prev_outpoint_to_delete(&prev_outpoints);
        }

        self.cache.set_script_pubkey_entries(script_pubkey_entries);

        Ok(())
    }
}
