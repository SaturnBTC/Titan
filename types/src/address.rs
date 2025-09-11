use {
    crate::{
        transaction::TransactionStatus, RuneAmount, SerializedOutPoint, SpentStatus, TxOut,
    },
    bitcoin::Txid,
    serde::{Deserialize, Serialize},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressData {
    pub value: u64,
    pub runes: Vec<RuneAmount>,
    pub outputs: Vec<AddressTxOut>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressTxOut {
    pub txid: Txid,
    pub vout: u32,
    pub value: u64,
    pub runes: Vec<RuneAmount>,
    pub risky_runes: Vec<RuneAmount>,
    pub spent: SpentStatus,
    pub status: TransactionStatus,
    pub size: u64,
    pub weight: u64,
}

impl From<(SerializedOutPoint, TxOut, TransactionStatus)> for AddressTxOut {
    fn from(
        (outpoint, tx_out, status): (SerializedOutPoint, TxOut, TransactionStatus),
    ) -> Self {
        Self {
            txid: outpoint.to_txid(),
            vout: outpoint.vout(),
            value: tx_out.value,
            runes: tx_out.runes,
            risky_runes: tx_out.risky_runes,
            spent: tx_out.spent,
            status,
            size: 0,
            weight: 0,
        }
    }
}

impl From<(
        SerializedOutPoint,
        TxOut,
        TransactionStatus,
        u64,
        u64,
    )> for AddressTxOut {
    fn from(
        (outpoint, tx_out, status, size, weight): (
            SerializedOutPoint,
            TxOut,
            TransactionStatus,
            u64,
            u64,
        ),
    ) -> Self {
        Self {
            txid: outpoint.to_txid(),
            vout: outpoint.vout(),
            value: tx_out.value,
            runes: tx_out.runes,
            risky_runes: tx_out.risky_runes,
            spent: tx_out.spent,
            status,
            size,
            weight,
        }
    }
}
