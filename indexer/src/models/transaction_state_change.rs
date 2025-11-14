use {
    super::Lot,
    bitcoin::ScriptBuf,
    borsh::{BorshDeserialize, BorshSerialize},
    ordinals::Rune,
    rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet},
    std::{
        fmt::Display,
        io::{Read, Result, Write},
    },
    titan_types::{RuneAmount, RuneId, SerializedOutPoint, TxOut},
};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct TransactionStateChangeInput {
    pub previous_outpoint: SerializedOutPoint,
    pub script_pubkey: Option<ScriptBuf>,
}

impl BorshSerialize for TransactionStateChangeInput {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<()> {
        self.previous_outpoint.serialize(writer)?;
        if let Some(script_pubkey) = &self.script_pubkey {
            true.serialize(writer)?;
            let script_bytes = script_pubkey.as_bytes();
            script_bytes.serialize(writer)?;
        } else {
            false.serialize(writer)?;
        }
        Ok(())
    }
}

impl BorshDeserialize for TransactionStateChangeInput {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self> {
        let outpoint = SerializedOutPoint::deserialize_reader(reader)?;
        let script_present = bool::deserialize_reader(reader)?;
        let script_pubkey = if script_present {
            let script_bytes = Vec::<u8>::deserialize_reader(reader)?;
            Some(ScriptBuf::from_bytes(script_bytes))
        } else {
            None
        };

        Ok(TransactionStateChangeInput {
            previous_outpoint: outpoint,
            script_pubkey,
        })
    }
}

#[derive(Debug, Clone)]
pub struct TransactionStateChange {
    pub inputs: Vec<TransactionStateChangeInput>,
    pub outputs: Vec<TxOut>,
    pub etched: Option<(RuneId, Rune)>,
    pub minted: Option<RuneAmount>,
    pub burned: HashMap<RuneId, Lot>,
    pub is_coinbase: bool,
}

impl BorshSerialize for TransactionStateChange {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<()> {
        // 1) inputs (Vec<OutPoint>) - manually serialize each OutPoint's components
        (self.inputs.len() as u64).serialize(writer)?;
        for outpoint in &self.inputs {
            outpoint.serialize(writer)?;
        }

        // 2) outputs (Vec<TxOutEntry>) - also already derive BorshSerialize
        self.outputs.serialize(writer)?;

        // 3) etched: Option<(RuneId, Rune)>
        //    We'll encode it as:
        //    - a boolean to indicate presence
        //    - if present, the fields of RuneId and then the u128 from Rune
        match &self.etched {
            Some((rune_id, rune)) => {
                true.serialize(writer)?; // present
                rune_id.block.serialize(writer)?;
                rune_id.tx.serialize(writer)?;
                rune.0.serialize(writer)?;
            }
            None => {
                false.serialize(writer)?; // not present
            }
        }

        // 4) minted: Option<RuneAmount>
        //    Since we have a custom Borsh impl for RuneAmount, we can just do the usual Option pattern.
        match &self.minted {
            Some(rune_amount) => {
                true.serialize(writer)?;
                rune_amount.serialize(writer)?;
            }
            None => {
                false.serialize(writer)?;
            }
        }

        // 5) burned: HashMap<RuneId, Lot>
        //    We'll serialize it as a length (u64) followed by each key/value.
        (self.burned.len() as u64).serialize(writer)?;
        for (rune_id, lot) in &self.burned {
            rune_id.block.serialize(writer)?;
            rune_id.tx.serialize(writer)?;
            lot.0.serialize(writer)?;
        }

        // 6) is_coinbase: bool
        self.is_coinbase.serialize(writer)?;

        Ok(())
    }
}

impl BorshDeserialize for TransactionStateChange {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self> {
        // 1) inputs - manually deserialize TxIn components
        let input_len = u64::deserialize_reader(reader)?;
        let mut inputs = Vec::with_capacity(input_len as usize);
        for _ in 0..input_len {
            let tx_in = TransactionStateChangeInput::deserialize_reader(reader)?;
            inputs.push(tx_in);
        }

        // 2) outputs
        let outputs = <Vec<TxOut>>::deserialize_reader(reader)?;

        // 3) etched
        let etched_present = bool::deserialize_reader(reader)?;
        let etched = if etched_present {
            let block = u64::deserialize_reader(reader)?;
            let tx = u32::deserialize_reader(reader)?;
            let rune_value = u128::deserialize_reader(reader)?;
            Some((RuneId::new(block, tx), Rune(rune_value)))
        } else {
            None
        };

        // 4) minted
        let minted_present = bool::deserialize_reader(reader)?;
        let minted = if minted_present {
            Some(RuneAmount::deserialize_reader(reader)?)
        } else {
            None
        };

        // 5) burned
        let burned_len = u64::deserialize_reader(reader)?;
        let mut burned = HashMap::default();
        for _ in 0..burned_len {
            let block = u64::deserialize_reader(reader)?;
            let tx = u32::deserialize_reader(reader)?;
            let lot_value = u128::deserialize_reader(reader)?;
            burned.insert(RuneId::new(block, tx), Lot(lot_value));
        }

        // 6) is_coinbase
        let is_coinbase = bool::deserialize_reader(reader)?;

        Ok(TransactionStateChange {
            inputs,
            outputs,
            etched,
            minted,
            burned,
            is_coinbase,
        })
    }
}

impl Display for TransactionStateChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TransactionStateChange {{ inputs: {:?}, outputs: {:?}, etched: {:?}, minted: {:?}, burned: {:?}, is_coinbase: {:?} }}",
            self.inputs, self.outputs, self.etched, self.minted, self.burned, self.is_coinbase
        )
    }
}

impl TransactionStateChange {
    pub fn has_rune_updates(&self) -> bool {
        self.outputs.iter().any(|output| output.has_runes())
            || self.etched.is_some()
            || self.minted.is_some()
            || !self.burned.is_empty()
    }

    pub fn rune_ids(&self) -> Vec<RuneId> {
        // Unique rune ids
        let mut rune_ids = HashSet::default();

        // Add burned rune ids
        self.burned.keys().for_each(|rune_id| {
            rune_ids.insert(*rune_id);
        });

        // Add minted rune id
        if let Some(minted) = self.minted.as_ref() {
            rune_ids.insert(minted.rune_id);
        }

        // Add etched rune id
        if let Some((id, _)) = self.etched.as_ref() {
            rune_ids.insert(*id);
        }

        // Add rune ids from outputs
        self.outputs.iter().for_each(|output| {
            output.runes.iter().for_each(|rune| {
                rune_ids.insert(rune.rune_id);
            });
        });

        rune_ids.into_iter().collect()
    }
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct TxRuneIndexRef {
    pub rune_id: Vec<u8>,
    pub index: u64,
}
