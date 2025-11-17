pub use {
    address::{AddressData, AddressTxOut},
    block::Block,
    event::{Event, EventType, Location},
    inscription_id::InscriptionId,
    mempool_entry::{MempoolEntry, MempoolEntryFee},
    outpoint::SerializedOutPoint,
    pagination::{Pagination, PaginationResponse},
    rune::{MintResponse, RuneAmount, RuneResponse},
    stats::{BlockTip, Status},
    subscription::{Subscription, TcpSubscriptionRequest},
    transaction::{Transaction, TransactionStatus},
    tx_in::TxIn,
    tx_out::{SpenderReference, SpentStatus, TxOut},
    txid::SerializedTxid,
};

mod address;
mod block;
mod event;
mod inscription_id;
mod mempool_entry;
mod outpoint;
mod pagination;
pub mod query;
mod rune;
mod rune_id;
mod rune_type;
mod serde_str;
mod spaced_rune;
mod stats;
mod subscription;
mod transaction;
mod tx_in;
mod tx_out;
mod txid;

// Re-export from ordinals crate
pub use ordinals::{Artifact, Cenotaph, Edict, Etching, Runestone};

pub use crate::rune_id::RuneId;
pub use crate::rune_type::Rune;
pub use crate::spaced_rune::SpacedRune;

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{hashes::Hash, BlockHash, ScriptBuf};
    use borsh::{BorshDeserialize, BorshSerialize};
    use serde_json;
    use std::str::FromStr;

    /// Helper function to test borsh serialization roundtrip
    fn test_borsh_roundtrip<T>(original: &T) -> T
    where
        T: BorshSerialize + BorshDeserialize + std::fmt::Debug + PartialEq,
    {
        let serialized = borsh::to_vec(original).expect("Failed to serialize");
        let deserialized = borsh::from_slice(&serialized).expect("Failed to deserialize");
        assert_eq!(original, &deserialized, "Borsh roundtrip failed");
        deserialized
    }

    /// Helper function to test serde serialization roundtrip
    fn test_serde_roundtrip<T>(original: &T) -> T
    where
        T: serde::Serialize + for<'de> serde::Deserialize<'de> + std::fmt::Debug + PartialEq,
    {
        let serialized = serde_json::to_string(original).expect("Failed to serialize");
        let deserialized = serde_json::from_str(&serialized).expect("Failed to deserialize");
        assert_eq!(original, &deserialized, "Serde roundtrip failed");
        deserialized
    }

    #[test]
    fn test_serialized_txid() {
        let txid_bytes = [1u8; 32];
        let original = SerializedTxid::from(txid_bytes);

        // Test Borsh
        test_borsh_roundtrip(&original);

        // Test Serde
        test_serde_roundtrip(&original);

        // Test string conversion
        let hex_str = original.to_string();
        let from_str = SerializedTxid::from_str(&hex_str).expect("Failed to parse from string");
        assert_eq!(original, from_str);
    }

    #[test]
    fn test_rune_amount() {
        let rune_id = RuneId::new(840000, 1);
        let original = RuneAmount {
            rune_id,
            amount: 1000000000000000000u128,
        };

        // Test Borsh
        test_borsh_roundtrip(&original);

        // Test Serde
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_spender_reference() {
        let txid = SerializedTxid::from([42u8; 32]);
        let original = SpenderReference { txid, vin: 3 };

        // Test Borsh
        test_borsh_roundtrip(&original);

        // Test Serde
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_spent_status() {
        // Test Unspent variant
        let unspent = SpentStatus::Unspent;
        test_borsh_roundtrip(&unspent);
        test_serde_roundtrip(&unspent);

        // Test Spent variant
        let txid = SerializedTxid::from([99u8; 32]);
        let spender_ref = SpenderReference { txid, vin: 2 };
        let spent = SpentStatus::Spent(spender_ref);
        test_borsh_roundtrip(&spent);
        test_serde_roundtrip(&spent);
    }

    #[test]
    fn test_tx_out_entry() {
        let rune_id = RuneId::new(840000, 1);
        let rune_amount = RuneAmount {
            rune_id,
            amount: 500000000000000000u128,
        };

        let original = TxOut {
            runes: vec![rune_amount.clone()],
            risky_runes: vec![rune_amount],
            value: 50000,
            spent: SpentStatus::Unspent,
            script_pubkey: ScriptBuf::new(),
        };

        // Test Borsh
        test_borsh_roundtrip(&original);

        // Test Serde
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_transaction_tx_out() {
        let script = ScriptBuf::from_bytes(vec![0x76, 0xa9, 0x14]); // OP_DUP OP_HASH160 PUSH(20)
        let rune_id = RuneId::new(840000, 1);
        let rune_amount = RuneAmount {
            rune_id,
            amount: 100000000000000000u128,
        };

        let original = TxOut {
            value: 100000,
            script_pubkey: script,
            runes: vec![rune_amount.clone()],
            risky_runes: vec![rune_amount],
            spent: SpentStatus::Unspent,
        };

        // Test Borsh
        test_borsh_roundtrip(&original);

        // Test Serde
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_transaction_status() {
        // Test unconfirmed
        let unconfirmed = TransactionStatus::unconfirmed();
        test_serde_roundtrip(&unconfirmed);

        // Test confirmed
        let block_hash = BlockHash::from_raw_hash(Hash::from_slice(&[1u8; 32]).unwrap());
        let confirmed = TransactionStatus::confirmed(840000, block_hash);
        test_serde_roundtrip(&confirmed);
    }

    #[test]
    fn test_serialized_outpoint() {
        let txid_bytes = [5u8; 32];
        let original = SerializedOutPoint::new(&txid_bytes, 1);

        // Test Borsh
        test_borsh_roundtrip(&original);

        // Test Serde
        test_serde_roundtrip(&original);

        // Test component extraction
        assert_eq!(original.txid(), &txid_bytes);
        assert_eq!(original.vout(), 1);
    }

    #[test]
    fn test_mempool_entry_fee() {
        let original = MempoolEntryFee {
            base: 1000,
            descendant: 2000,
            ancestor: 3000,
        };

        // Test Borsh
        test_borsh_roundtrip(&original);

        // Test Serde
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_mempool_entry() {
        let txid = SerializedTxid::from([7u8; 32]);
        let fees = MempoolEntryFee {
            base: 1000,
            descendant: 2000,
            ancestor: 3000,
        };

        let original = MempoolEntry {
            vsize: 250,
            weight: Some(1000),
            descendant_count: 1,
            descendant_size: 250,
            ancestor_count: 1,
            ancestor_size: 250,
            fees,
            depends: vec![txid.clone()],
            spent_by: vec![txid],
        };

        // Test Borsh
        test_borsh_roundtrip(&original);

        // Test Serde
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_edge_cases() {
        // Test empty vectors
        let empty_tx_out = TxOut {
            runes: vec![],
            risky_runes: vec![],
            value: 0,
            spent: SpentStatus::Unspent,
            script_pubkey: ScriptBuf::new(),
        };
        test_borsh_roundtrip(&empty_tx_out);
        test_serde_roundtrip(&empty_tx_out);

        // Test mempool entry with no weight
        let mempool_no_weight = MempoolEntry {
            vsize: 250,
            weight: None,
            descendant_count: 1,
            descendant_size: 250,
            ancestor_count: 1,
            ancestor_size: 250,
            fees: MempoolEntryFee {
                base: 1000,
                descendant: 2000,
                ancestor: 3000,
            },
            depends: vec![],
            spent_by: vec![],
        };
        test_borsh_roundtrip(&mempool_no_weight);
        test_serde_roundtrip(&mempool_no_weight);

        // Test max values
        let max_rune_amount = RuneAmount {
            rune_id: RuneId::new(u64::MAX, u32::MAX),
            amount: u128::MAX,
        };
        test_borsh_roundtrip(&max_rune_amount);
        test_serde_roundtrip(&max_rune_amount);
    }

    #[test]
    fn test_complex_nested_structures() {
        // Create a complex TxOut with multiple runes
        let rune_id1 = RuneId::new(840000, 1);
        let rune_id2 = RuneId::new(840001, 2);

        let rune_amounts = vec![
            RuneAmount {
                rune_id: rune_id1,
                amount: 1000000000000000000u128,
            },
            RuneAmount {
                rune_id: rune_id2,
                amount: 2000000000000000000u128,
            },
        ];

        let spender_ref = SpenderReference {
            txid: SerializedTxid::from([255u8; 32]),
            vin: 0,
        };

        let complex_tx_out = TxOut {
            value: 5000000,
            script_pubkey: ScriptBuf::from_bytes(vec![0x00, 0x14]), // P2WPKH
            runes: rune_amounts.clone(),
            risky_runes: rune_amounts,
            spent: SpentStatus::Spent(spender_ref),
        };

        test_borsh_roundtrip(&complex_tx_out);
        test_serde_roundtrip(&complex_tx_out);
    }

    #[test]
    fn test_serialization_consistency() {
        // Ensure that the same data serialized with Borsh and Serde produces
        // the same result when deserialized
        let rune_id = RuneId::new(840000, 1);
        let original = RuneAmount {
            rune_id,
            amount: 1000000000000000000u128,
        };

        let borsh_serialized = borsh::to_vec(&original).unwrap();
        let borsh_deserialized: RuneAmount = borsh::from_slice(&borsh_serialized).unwrap();

        let serde_serialized = serde_json::to_string(&original).unwrap();
        let serde_deserialized: RuneAmount = serde_json::from_str(&serde_serialized).unwrap();

        // Both should deserialize to the same value
        assert_eq!(borsh_deserialized, serde_deserialized);
        assert_eq!(original, borsh_deserialized);
        assert_eq!(original, serde_deserialized);
    }
}
