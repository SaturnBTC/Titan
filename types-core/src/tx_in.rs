use bitcoin::{ScriptBuf, Sequence, Witness};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{RuneAmount, SerializedOutPoint, TxOut};

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PreviousOutputData {
    pub value: u64,
    pub runes: Vec<RuneAmount>,
    pub risky_runes: Vec<RuneAmount>,
    pub script_pubkey: ScriptBuf,
}

impl From<TxOut> for PreviousOutputData {
    fn from(tx_out_entry: TxOut) -> Self {
        Self {
            value: tx_out_entry.value,
            runes: tx_out_entry.runes,
            risky_runes: tx_out_entry.risky_runes,
            script_pubkey: tx_out_entry.script_pubkey,
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TxIn {
    pub previous_output: SerializedOutPoint,
    pub script_sig: ScriptBuf,
    pub sequence: Sequence,
    pub witness: Witness,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub previous_output_data: Option<PreviousOutputData>,
}

impl From<(bitcoin::TxIn, Option<TxOut>)> for TxIn {
    fn from((tx_in, previous_output_data): (bitcoin::TxIn, Option<TxOut>)) -> Self {
        Self {
            previous_output: tx_in.previous_output.into(),
            script_sig: tx_in.script_sig,
            sequence: tx_in.sequence,
            witness: tx_in.witness,
            previous_output_data: previous_output_data.map(Into::into),
        }
    }
}
