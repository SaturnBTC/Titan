use {
    crate::{tx_in::TxIn, tx_out::SpentStatus, TxOut},
    bitcoin::{constants::WITNESS_SCALE_FACTOR, BlockHash, Txid},
};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TransactionStatus {
    pub confirmed: bool,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub block_height: Option<u64>,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub block_hash: Option<BlockHash>,
}

impl TransactionStatus {
    pub fn unconfirmed() -> Self {
        Self {
            confirmed: false,
            block_height: None,
            block_hash: None,
        }
    }

    pub fn confirmed(block_height: u64, block_hash: BlockHash) -> Self {
        Self {
            confirmed: true,
            block_height: Some(block_height),
            block_hash: Some(block_hash),
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Transaction {
    pub txid: Txid,
    pub version: i32,
    pub lock_time: u32,
    pub input: Vec<TxIn>,
    pub output: Vec<TxOut>,
    pub status: TransactionStatus,
    pub size: u64,
    pub weight: u64,
}

impl Transaction {
    pub fn vbytes(&self) -> u64 {
        (self.weight + WITNESS_SCALE_FACTOR as u64 - 1) / WITNESS_SCALE_FACTOR as u64
    }

    pub fn is_coinbase(&self) -> bool {
        if self.input.len() != 1 {
            return false;
        }

        let prev = &self.input[0].previous_output;
        let is_zero_txid = prev.txid().iter().all(|b| *b == 0);
        let is_max_vout = prev.vout() == u32::MAX;
        is_zero_txid && is_max_vout
    }

    pub fn input_value_sat(&self) -> Option<u64> {
        let mut sum: u64 = 0;

        for txin in &self.input {
            let value = txin.previous_output_data.as_ref()?.value;
            sum = sum.saturating_add(value);
        }

        Some(sum)
    }

    pub fn output_value_sat(&self) -> u64 {
        self.output
            .iter()
            .fold(0u64, |acc, o| acc.saturating_add(o.value))
    }

    pub fn fee_paid_sat(&self) -> Option<u64> {
        if self.is_coinbase() {
            return None;
        }

        let input_sum = self.input_value_sat()?;
        input_sum.checked_sub(self.output_value_sat())
    }

    pub fn fee_rate_sat_vb(&self) -> Option<f64> {
        let fee = self.fee_paid_sat()? as f64;
        let vbytes = self.vbytes() as f64;
        if vbytes == 0.0 {
            return None;
        }
        Some(fee / vbytes)
    }

    pub fn num_inputs(&self) -> usize {
        self.input.len()
    }

    pub fn num_outputs(&self) -> usize {
        self.output.len()
    }

    pub fn has_runes(&self) -> bool {
        self.output.iter().any(|o| !o.runes.is_empty())
    }

    pub fn has_risky_runes(&self) -> bool {
        self.output.iter().any(|o| !o.risky_runes.is_empty())
    }
}

impl
    From<(
        bitcoin::Transaction,
        TransactionStatus,
        Vec<Option<TxOut>>,
        Vec<Option<TxOut>>,
    )> for Transaction
{
    fn from(
        (transaction, status, prev_outputs, outputs): (
            bitcoin::Transaction,
            TransactionStatus,
            Vec<Option<TxOut>>,
            Vec<Option<TxOut>>,
        ),
    ) -> Self {
        Transaction {
            size: transaction.total_size() as u64,
            weight: transaction.weight().to_wu(),
            txid: transaction.compute_txid(),
            version: transaction.version.0,
            lock_time: transaction.lock_time.to_consensus_u32(),
            input: transaction
                .input
                .into_iter()
                .zip(prev_outputs.into_iter())
                .map(|(tx_in, prev_output)| TxIn::from((tx_in, prev_output)))
                .collect(),
            output: transaction
                .output
                .into_iter()
                .zip(outputs.into_iter())
                .map(|(tx_out, tx_out_entry)| {
                    let (runes, risky_runes, spent) = match tx_out_entry {
                        Some(tx_out_entry) => (
                            tx_out_entry.runes,
                            tx_out_entry.risky_runes,
                            tx_out_entry.spent,
                        ),
                        None => (vec![], vec![], SpentStatus::SpentUnknown),
                    };

                    let tx_out = TxOut {
                        value: tx_out.value.to_sat(),
                        script_pubkey: tx_out.script_pubkey,
                        runes,
                        risky_runes,
                        spent,
                    };

                    tx_out
                })
                .collect(),
            status,
        }
    }
}
