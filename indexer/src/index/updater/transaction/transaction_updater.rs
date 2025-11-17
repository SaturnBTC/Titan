use {
    super::TransactionStore,
    crate::{
        index::{inscription::index_rune_icon, Settings, StoreError},
        models::{BlockId, RuneEntry, TransactionStateChange, TransactionStateChangeInput},
    },
    bitcoin::Transaction,
    ordinals::{Artifact, Etching, Runestone},
    thiserror::Error,
    titan_types::RuneId,
    titan_types::{Event, SerializedOutPoint, SerializedTxid, SpenderReference, TxOut},
    titan_types::{Rune, SpacedRune},
    tokio::sync::mpsc::error::SendError,
};

pub trait TransactionEventMgr {
    fn add_event(&mut self, event: Event);
}

#[derive(Debug, Error)]
pub enum TransactionUpdaterError {
    #[error("store error {0}")]
    Store(#[from] StoreError),
    #[error("event sender error {0}")]
    EventSender(#[from] SendError<Event>),
    #[error("overflow in {0}")]
    Overflow(String),
}

type Result<T> = std::result::Result<T, TransactionUpdaterError>;

#[derive(Debug)]
pub struct TransactionUpdaterSettings {
    pub(super) index_bitcoin_transactions: bool,
}

impl From<Settings> for TransactionUpdaterSettings {
    fn from(settings: Settings) -> Self {
        Self {
            index_bitcoin_transactions: settings.index_bitcoin_transactions,
        }
    }
}

pub struct TransactionUpdater {
    pub(super) settings: TransactionUpdaterSettings,
    pub(super) mempool: bool,
}

impl TransactionUpdater {
    pub fn new(settings: TransactionUpdaterSettings, mempool: bool) -> Result<Self> {
        Ok(Self { settings, mempool })
    }

    pub fn save(
        &mut self,
        store: &mut dyn TransactionStore,
        events: &mut dyn TransactionEventMgr,
        block_time: u32,
        block_id: Option<BlockId>,
        txid: SerializedTxid,
        transaction: &Transaction,
        transaction_state_change: &TransactionStateChange,
    ) -> Result<()> {
        if let Some((id, rune)) = transaction_state_change.etched {
            self.etched_rune(
                store,
                events,
                block_time,
                block_id.as_ref().map(|id| id.height),
                txid,
                transaction,
                &id,
                rune,
            )?;
        }

        // Update burned rune
        for (rune_id, amount) in transaction_state_change.burned.iter() {
            self.burn_rune(
                store,
                events,
                block_id.as_ref().map(|id| id.height),
                txid,
                rune_id,
                amount.n(),
            )?;
        }

        // Add minted rune if any.
        if let Some(minted) = transaction_state_change.minted.as_ref() {
            self.mint_rune(
                store,
                events,
                block_id.as_ref().map(|id| id.height),
                txid,
                &minted.rune_id,
                minted.amount,
            )?;
        }

        if !transaction.is_coinbase() {
            for (vin, tx_in) in transaction_state_change.inputs.iter().enumerate() {
                // Spend inputs
                self.update_spendable_input(
                    store,
                    tx_in,
                    SpenderReference {
                        txid,
                        vin: vin as u32,
                    },
                )?;
            }
        }

        // Create new outputs
        for (vout, output) in transaction_state_change.outputs.iter().enumerate() {
            // Create new outputs
            let outpoint = SerializedOutPoint::from_txid_vout(&txid, vout as u32);

            let script_pubkey = transaction.output[vout].script_pubkey.clone();

            store.set_tx_out(outpoint, output.clone(), script_pubkey);

            self.transfer_rune(
                events,
                block_id.as_ref().map(|id| id.height),
                txid,
                outpoint,
                output,
            );
        }

        // Save transaction state change
        store.set_tx_state_changes(txid, transaction_state_change.clone());

        // Save rune transactions
        // Get all modified rune ids.
        let rune_ids = transaction_state_change.rune_ids();

        for rune_id in rune_ids {
            store.add_rune_transaction(rune_id, txid);
        }

        if self.settings.index_bitcoin_transactions {
            store.set_transaction(txid, transaction.clone());
        }

        if !self.mempool {
            store.set_transaction_confirming_block(txid, block_id.unwrap());
        }

        Ok(())
    }

    fn update_spendable_input(
        &mut self,
        store: &mut dyn TransactionStore,
        outpoint: &TransactionStateChangeInput,
        spent: SpenderReference,
    ) -> Result<()> {
        let previous_outpoint = outpoint.previous_outpoint;
        match store.set_spent_tx_out(outpoint, spent) {
            Ok(()) => Ok(()),
            Err(StoreError::NotFound(_)) => {
                return Err(TransactionUpdaterError::Store(StoreError::NotFound(
                    format!("outpoint not found: {:?}", previous_outpoint),
                )));
            }
            Err(e) => {
                return Err(TransactionUpdaterError::Store(e));
            }
        }
    }

    fn burn_rune(
        &mut self,
        store: &mut dyn TransactionStore,
        events: &mut dyn TransactionEventMgr,
        height: Option<u64>,
        txid: SerializedTxid,
        rune_id: &RuneId,
        amount: u128,
    ) -> Result<()> {
        self.update_burn_balance(store, rune_id, amount as i128)?;

        events.add_event(Event::RuneBurned {
            location: height.into(),
            txid,
            amount: amount as u128,
            rune_id: *rune_id,
        });

        Ok(())
    }

    fn mint_rune(
        &mut self,
        store: &mut dyn TransactionStore,
        events: &mut dyn TransactionEventMgr,
        height: Option<u64>,
        txid: SerializedTxid,
        rune_id: &RuneId,
        amount: u128,
    ) -> Result<()> {
        self.increment_mint(store, rune_id)?;

        events.add_event(Event::RuneMinted {
            location: height.into(),
            txid,
            amount: amount as u128,
            rune_id: *rune_id,
        });

        Ok(())
    }

    fn update_burn_balance(
        &mut self,
        store: &mut dyn TransactionStore,
        rune_id: &RuneId,
        amount: i128,
    ) -> Result<()> {
        let mut rune_entry = store.get_rune(rune_id)?;

        if self.mempool {
            let result = rune_entry
                .pending_burns
                .checked_add_signed(amount)
                .ok_or(TransactionUpdaterError::Overflow("burn".to_string()))?;
            rune_entry.pending_burns = result;
        } else {
            let result = rune_entry
                .burned
                .checked_add_signed(amount)
                .ok_or(TransactionUpdaterError::Overflow("burn".to_string()))?;

            rune_entry.burned = result;
        }

        store.set_rune(*rune_id, rune_entry);

        Ok(())
    }

    fn increment_mint(&mut self, store: &mut dyn TransactionStore, rune_id: &RuneId) -> Result<()> {
        let mut rune_entry = store.get_rune(rune_id)?;

        if self.mempool {
            let result = rune_entry.pending_mints.saturating_add_signed(1);
            rune_entry.pending_mints = result;
        } else {
            let result = rune_entry.mints.saturating_add_signed(1);

            rune_entry.mints = result;
        }

        store.set_rune(*rune_id, rune_entry);

        Ok(())
    }

    fn etched_rune(
        &mut self,
        store: &mut dyn TransactionStore,
        events: &mut dyn TransactionEventMgr,
        block_time: u32,
        height: Option<u64>,
        txid: SerializedTxid,
        transaction: &Transaction,
        id: &RuneId,
        rune: Rune,
    ) -> Result<()> {
        let artifact = Runestone::decipher(transaction).unwrap();
        self.create_rune_entry(store, block_time, txid, transaction, *id, rune, &artifact)?;

        events.add_event(Event::RuneEtched {
            location: height.into(),
            txid,
            rune_id: *id,
        });

        Ok(())
    }

    fn create_rune_entry(
        &mut self,
        store: &mut dyn TransactionStore,
        block_time: u32,
        txid: SerializedTxid,
        transaction: &Transaction,
        id: RuneId,
        rune: Rune,
        artifact: &Artifact,
    ) -> Result<()> {
        let inscription = index_rune_icon(transaction, txid);

        if let Some((id, inscription)) = inscription.as_ref() {
            store.set_inscription(id.clone(), inscription.clone());
        }

        let entry = match artifact {
            Artifact::Cenotaph(_) => RuneEntry {
                block: id.block,
                burned: 0,
                divisibility: 0,
                etching: txid,
                terms: None,
                mints: 0,
                number: store.get_runes_count(),
                premine: 0,
                spaced_rune: SpacedRune { rune, spacers: 0 },
                symbol: None,
                pending_burns: 0,
                pending_mints: 0,
                inscription_id: inscription.map(|(id, _)| id),
                timestamp: block_time.into(),
                turbo: false,
            },
            Artifact::Runestone(Runestone { etching, .. }) => {
                let Etching {
                    divisibility,
                    terms,
                    premine,
                    spacers,
                    symbol,
                    turbo,
                    ..
                } = etching.unwrap();

                RuneEntry {
                    block: id.block,
                    burned: 0,
                    divisibility: divisibility.unwrap_or_default(),
                    etching: txid,
                    terms,
                    mints: 0,
                    number: store.get_runes_count(),
                    premine: premine.unwrap_or_default(),
                    spaced_rune: SpacedRune {
                        rune,
                        spacers: spacers.unwrap_or_default(),
                    },
                    symbol,
                    pending_burns: 0,
                    pending_mints: 0,
                    inscription_id: inscription.map(|(id, _)| id),
                    timestamp: block_time.into(),
                    turbo,
                }
            }
        };

        store.set_rune(id, entry);
        store.set_rune_id_number(store.get_runes_count(), id);
        store.set_rune_id(rune, id);
        store.increment_runes_count();

        Ok(())
    }

    fn transfer_rune(
        &mut self,
        events: &mut dyn TransactionEventMgr,
        height: Option<u64>,
        txid: SerializedTxid,
        outpoint: SerializedOutPoint,
        output: &TxOut,
    ) {
        for rune_amount in output.runes.iter() {
            events.add_event(Event::RuneTransferred {
                location: height.into(),
                txid,
                outpoint,
                rune_id: rune_amount.rune_id,
                amount: rune_amount.amount,
            });
        }
    }
}
