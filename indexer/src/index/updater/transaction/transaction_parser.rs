use {
    super::TransactionStore,
    crate::{
        bitcoin_rpc::BitcoinCoreRpcResultExt,
        index::{Chain, StoreError},
        models::{Lot, TransactionStateChange, TransactionStateChangeInput},
        util::IntoUsize,
    },
    bitcoin::{consensus::encode, OutPoint, Transaction},
    bitcoincore_rpc::{Client, RpcApi},
    ordinals::{Artifact, Edict, Height, Rune, RuneId, Runestone},
    rustc_hash::FxHashMap as HashMap,
    thiserror::Error,
    titan_types::{RuneAmount, SerializedOutPoint, SpentStatus, TxOut},
};

#[derive(Debug, Error)]
pub enum TransactionParserError {
    #[error("store error {0}")]
    Store(#[from] StoreError),
    #[error("bitcoin rpc error {0}")]
    BitcoinRpc(#[from] bitcoincore_rpc::Error),
    #[error("encode error {0}")]
    Encode(#[from] encode::Error),
}

type Result<T> = std::result::Result<T, TransactionParserError>;

pub struct TransactionParser<'client> {
    pub(super) client: &'client Client,
    pub(super) height: u64,
    pub(super) minimum_rune: Rune,
    pub(super) should_index_runes: bool,
    pub(super) mempool: bool,
}

impl<'client> TransactionParser<'client> {
    pub fn new(client: &'client Client, chain: Chain, height: u64, mempool: bool) -> Result<Self> {
        let minimum_rune = Rune::minimum_at_height(chain.into(), Height(height as u32));

        let min_rune_height = chain.first_rune_height() as u64;
        let should_index_runes = height >= min_rune_height;

        Ok(Self {
            client,
            height,
            minimum_rune,
            should_index_runes,
            mempool,
        })
    }

    pub fn parse(
        &mut self,
        store: &mut dyn TransactionStore,
        tx_index: u32,
        tx: &Transaction,
    ) -> Result<TransactionStateChange> {
        let prev_outputs = self.get_prev_outputs(store, tx)?;
        let inputs = tx
            .input
            .iter()
            .map(|txin| -> Result<TransactionStateChangeInput> {
                let previous_outpoint = txin.previous_output.into();
                let tx_out = prev_outputs.get(&previous_outpoint);

                let script_pubkey = tx_out.map(|tx_out| tx_out.script_pubkey.clone());

                Ok(TransactionStateChangeInput {
                    previous_outpoint,
                    script_pubkey,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let (allocated, risky_allocated, minted, etched, burned) = if self.should_index_runes {
            self.parse_runes(store, prev_outputs, tx_index, tx)?
        } else {
            (
                vec![HashMap::default(); tx.output.len()],
                vec![HashMap::default(); tx.output.len()],
                None,
                None,
                HashMap::default(),
            )
        };

        // update outpoint balances
        let mut tx_outs: Vec<TxOut> = Vec::with_capacity(tx.output.len());
        for (vout, balances) in allocated.into_iter().enumerate() {
            let mut balances = balances.into_iter().collect::<Vec<(RuneId, Lot)>>();

            // Sort balances by id so tests can assert balances in a fixed order
            balances.sort();

            let mut tx_out = TxOut {
                runes: vec![],
                risky_runes: vec![],
                spent: SpentStatus::Unspent,
                value: tx.output[vout].value.to_sat(),
                script_pubkey: tx.output[vout].script_pubkey.clone(),
            };

            for (id, balance) in balances {
                tx_out.runes.push(RuneAmount {
                    rune_id: id,
                    amount: balance.0,
                });
            }

            let mut risky_runes_vec = risky_allocated[vout]
                .clone()
                .into_iter()
                .collect::<Vec<(RuneId, Lot)>>();

            risky_runes_vec.sort();

            for (id, balance) in risky_runes_vec {
                tx_out.risky_runes.push(RuneAmount {
                    rune_id: id,
                    amount: balance.0,
                });
            }

            tx_outs.push(tx_out);
        }

        let transaction_state_change = TransactionStateChange {
            burned,
            inputs,
            outputs: tx_outs,
            etched,
            minted,
            is_coinbase: tx.is_coinbase(),
        };

        Ok(transaction_state_change)
    }

    fn parse_runes(
        &mut self,
        store: &mut dyn TransactionStore,
        prev_outputs: HashMap<SerializedOutPoint, TxOut>,
        tx_index: u32,
        tx: &Transaction,
    ) -> Result<(
        Vec<HashMap<RuneId, Lot>>, // allocated runes per output
        Vec<HashMap<RuneId, Lot>>, // allocated risky runes per output
        Option<RuneAmount>,        // minted rune at transaction level
        Option<(RuneId, Rune)>,    // etched rune, if any
        HashMap<RuneId, Lot>,      // burned runes
    )> {
        let artifact = Runestone::decipher(tx);
        let (mut unallocated, mut risky_unallocated) = self.unallocated(tx, prev_outputs)?;

        // Create per-output allocation maps
        let mut allocated: Vec<HashMap<RuneId, Lot>> = vec![HashMap::default(); tx.output.len()];
        let mut allocated_risky: Vec<HashMap<RuneId, Lot>> =
            vec![HashMap::default(); tx.output.len()];
        let mut minted: Option<RuneAmount> = None;
        let mut etched: Option<(RuneId, Rune)> = None;

        // If there is a mintable rune, add its amount into the unallocated maps.
        if let Some(artifact) = &artifact {
            if let Some(id) = artifact.mint() {
                if let Some(amount) = self.mint(store, id)? {
                    if self.mempool {
                        *risky_unallocated.entry(id).or_default() += amount;
                    } else {
                        *unallocated.entry(id).or_default() += amount;
                    }

                    minted = Some(RuneAmount {
                        rune_id: id,
                        amount: amount.0,
                    });
                }
            }

            etched = self.etched(store, tx_index, tx, artifact)?;

            if let Artifact::Runestone(runestone) = artifact {
                if let Some((id, ..)) = etched {
                    if self.mempool {
                        *risky_unallocated.entry(id).or_default() +=
                            runestone.etching.unwrap().premine.unwrap_or_default();
                    } else {
                        *unallocated.entry(id).or_default() +=
                            runestone.etching.unwrap().premine.unwrap_or_default();
                    }
                }

                for Edict { id, amount, output } in runestone.edicts.iter().copied() {
                    let amount = Lot(amount);
                    let output = usize::try_from(output).unwrap();
                    assert!(output <= tx.output.len());

                    let id = if id == RuneId::default() {
                        // If the edict did not specify an id, use the etched id (if available)
                        let Some((id, ..)) = etched else {
                            continue;
                        };
                        id
                    } else {
                        id
                    };

                    if let Some(balance) = unallocated.get_mut(&id) {
                        self.allocate_edicts(tx, &mut allocated, balance, id, amount, output);
                    }

                    if let Some(risky_balance) = risky_unallocated.get_mut(&id) {
                        self.allocate_edicts(
                            tx,
                            &mut allocated_risky,
                            risky_balance,
                            id,
                            amount,
                            output,
                        );
                    }
                }
            }
        }

        let mut burned: HashMap<RuneId, Lot> = HashMap::default();

        if let Some(Artifact::Cenotaph(_)) = artifact {
            for (id, balance) in unallocated {
                *burned.entry(id).or_default() += balance;
            }
        } else {
            let pointer = artifact
                .map(|artifact| match artifact {
                    Artifact::Runestone(runestone) => runestone.pointer,
                    Artifact::Cenotaph(_) => unreachable!(),
                })
                .unwrap_or_default();

            if let Some(vout) = pointer
                .map(|pointer| pointer.try_into().unwrap())
                .inspect(|&pointer| assert!(pointer < allocated.len()))
                .or_else(|| {
                    tx.output
                        .iter()
                        .enumerate()
                        .find(|(_vout, tx_out)| !tx_out.script_pubkey.is_op_return())
                        .map(|(vout, _tx_out)| vout)
                })
            {
                self.allocate_remaining_in_pointer(&mut unallocated, &mut allocated, vout);
                self.allocate_remaining_in_pointer(
                    &mut risky_unallocated,
                    &mut allocated_risky,
                    vout,
                );
            } else {
                for (id, balance) in unallocated {
                    if balance > 0 {
                        *burned.entry(id).or_default() += balance;
                    }
                }
            }
        }

        // If an output is an OP_RETURN then mark all runes there as burned.
        for (vout, balances) in allocated.iter().enumerate() {
            if tx.output[vout].script_pubkey.is_op_return() {
                for (id, balance) in balances {
                    *burned.entry(*id).or_default() += *balance;
                }
            }
        }

        Ok((allocated, allocated_risky, minted, etched, burned))
    }

    fn allocate_edicts(
        &mut self,
        tx: &Transaction,
        allocated: &mut Vec<HashMap<RuneId, Lot>>,
        balance: &mut Lot,
        id: RuneId,
        amount: Lot,
        output: usize,
    ) {
        // allocate function that also flags minted allocations
        let mut allocate = |balance: &mut Lot, amount: Lot, output: usize| {
            if amount > 0 {
                *balance -= amount;
                *allocated[output].entry(id).or_default() += amount;
            }
        };

        if output == tx.output.len() {
            // When output equals the number of outputs, distribute among eligible outputs.
            let destinations: Vec<usize> = tx
                .output
                .iter()
                .enumerate()
                .filter_map(|(output, tx_out)| {
                    (!tx_out.script_pubkey.is_op_return()).then_some(output)
                })
                .collect();

            if !destinations.is_empty() {
                if amount == 0 {
                    let amount_div = *balance / destinations.len() as u128;
                    let remainder = usize::try_from(*balance % destinations.len() as u128).unwrap();

                    for (i, dest) in destinations.iter().enumerate() {
                        let alloc_amount = if i < remainder {
                            amount_div + 1
                        } else {
                            amount_div
                        };
                        allocate(balance, alloc_amount, *dest);
                    }
                } else {
                    for dest in destinations {
                        allocate(balance, amount.min(*balance), dest);
                    }
                }
            }
        } else {
            let alloc_amount = if amount == 0 {
                *balance
            } else {
                amount.min(*balance)
            };
            allocate(balance, alloc_amount, output);
        }
    }

    fn allocate_remaining_in_pointer(
        &mut self,
        unallocated: &mut HashMap<RuneId, Lot>,
        allocated: &mut Vec<HashMap<RuneId, Lot>>,
        vout: usize,
    ) {
        for (id, balance) in unallocated {
            if *balance > 0 {
                *allocated[vout].entry(*id).or_default() += *balance;
            }
        }
    }

    fn etched(
        &mut self,
        store: &mut dyn TransactionStore,
        tx_index: u32,
        tx: &Transaction,
        artifact: &Artifact,
    ) -> Result<Option<(RuneId, Rune)>> {
        // TODO: Currently we don't add etched runes.
        // But that means that there are outputs that could have premined runes that we're not showing.
        // so this is something that we should address soon.
        if self.mempool {
            return Ok(None);
        }

        let rune: Option<Rune> = match artifact {
            Artifact::Runestone(runestone) => match runestone.etching {
                Some(etching) => etching.rune,
                None => return Ok(None),
            },
            Artifact::Cenotaph(cenotaph) => match cenotaph.etching {
                Some(rune) => Some(rune),
                None => return Ok(None),
            },
        };

        let rune = if let Some(rune) = rune {
            let exists = store.does_rune_exist(&rune);
            match exists {
                Ok(_) => return Ok(None),
                Err(e) => {
                    if !e.is_not_found() {
                        return Err(e.into());
                    }
                }
            }

            if rune < self.minimum_rune
                || rune.is_reserved()
                || !self.tx_commits_to_rune(store, tx, rune)?
            {
                return Ok(None);
            }
            rune
        } else {
            Rune::reserved(self.height.into(), tx_index)
        };

        let rune_id = RuneId {
            block: self.height.into(),
            tx: tx_index,
        };

        Ok(Some((rune_id, rune)))
    }

    fn mint(&mut self, store: &mut dyn TransactionStore, id: RuneId) -> Result<Option<Lot>> {
        let rune_entry = match store.get_rune(&id) {
            Ok(rune_entry) => rune_entry,
            Err(err) => {
                if err.is_not_found() {
                    return Ok(None);
                } else {
                    return Err(err.into());
                }
            }
        };

        let Ok(amount) = rune_entry.mintable(self.height.into()) else {
            return Ok(None);
        };

        Ok(Some(Lot(amount)))
    }

    fn tx_commits_to_rune(
        &self,
        store: &dyn TransactionStore,
        tx: &Transaction,
        rune: Rune,
    ) -> Result<bool> {
        let commitment = rune.commitment();

        for input in &tx.input {
            // extracting a tapscript does not indicate that the input being spent
            // was actually a taproot output. this is checked below, when we load the
            // output's entry from the database
            let Some(tapscript) = input.witness.tapscript() else {
                continue;
            };

            for instruction in tapscript.instructions() {
                // ignore errors, since the extracted script may not be valid
                let Ok(instruction) = instruction else {
                    break;
                };

                let Some(pushbytes) = instruction.push_bytes() else {
                    continue;
                };

                if pushbytes.as_bytes() != commitment {
                    continue;
                }

                match self.validate_commit_transaction_with_cache(store, input.previous_output) {
                    Ok(true) => return Ok(true),
                    Ok(false) => continue,
                    Err(e) => {
                        if matches!(e, TransactionParserError::Store(StoreError::NotFound(_))) {
                            return self.validate_commit_transaction(input.previous_output);
                        } else {
                            return Err(e.into());
                        }
                    }
                }
            }
        }

        Ok(false)
    }

    fn validate_commit_transaction_with_cache(
        &self,
        store: &dyn TransactionStore,
        outpoint: OutPoint,
    ) -> Result<bool> {
        let transaction = store.get_transaction(&outpoint.txid.into())?;

        let taproot = transaction.output[outpoint.vout.into_usize()]
            .script_pubkey
            .is_p2tr();

        if !taproot {
            return Ok(false);
        }

        let block_id = store.get_transaction_confirming_block(&outpoint.txid.into())?;

        let confirmations = self.height.checked_sub(block_id.height).unwrap() + 1;

        Ok(confirmations >= Runestone::COMMIT_CONFIRMATIONS.into())
    }

    fn validate_commit_transaction(&self, outpoint: OutPoint) -> Result<bool> {
        let Some(tx_info) = self
            .client
            .get_raw_transaction_info(&outpoint.txid, None)
            .into_option()?
        else {
            // Transaction not found in Bitcoin Core (likely txindex not enabled or tx pruned)
            // Return false instead of panicking - this is not a fatal error
            tracing::warn!(
                "Cannot validate commit transaction {}: transaction not found in Bitcoin Core. \
                 This may indicate txindex is not enabled or the transaction was pruned.",
                outpoint.txid
            );
            return Ok(false);
        };

        let taproot = tx_info.vout[outpoint.vout.into_usize()]
            .script_pub_key
            .script()?
            .is_p2tr();

        if !taproot {
            return Ok(false);
        }

        let commit_tx_height = self
            .client
            .get_block_header_info(&tx_info.blockhash.unwrap())
            .into_option()?
            .unwrap()
            .height;

        let confirmations = self
            .height
            .checked_sub(commit_tx_height.try_into().unwrap())
            .unwrap()
            + 1;

        Ok(confirmations >= Runestone::COMMIT_CONFIRMATIONS.into())
    }

    fn unallocated(
        &self,
        tx: &Transaction,
        outputs: HashMap<SerializedOutPoint, TxOut>,
    ) -> Result<(HashMap<RuneId, Lot>, HashMap<RuneId, Lot>)> {
        let mut unallocated = HashMap::default();
        let mut risky_unallocated = HashMap::default();

        if tx.is_coinbase() {
            return Ok((unallocated, risky_unallocated));
        }

        // 3) Accumulate unallocated:
        for (_, tx_out) in outputs {
            for rune_amount in tx_out.runes {
                *unallocated.entry(rune_amount.rune_id).or_default() += rune_amount.amount;
            }

            if self.mempool {
                for rune_amount in tx_out.risky_runes {
                    *risky_unallocated.entry(rune_amount.rune_id).or_default() +=
                        rune_amount.amount;
                }
            }
        }

        Ok((unallocated, risky_unallocated))
    }

    fn get_prev_outputs(
        &self,
        store: &mut dyn TransactionStore,
        tx: &Transaction,
    ) -> Result<HashMap<SerializedOutPoint, TxOut>> {
        // 1) Collect all outpoints:
        let mut outpoints = Vec::with_capacity(tx.input.len());
        for input in &tx.input {
            outpoints.push(SerializedOutPoint::from(input.previous_output));
        }

        let tx_out_map = store.get_tx_outs(&outpoints)?;

        Ok(tx_out_map)
    }
}
