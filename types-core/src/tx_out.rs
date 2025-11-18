use {
    crate::{rune_amount::RuneAmount, SerializedTxid},
    bitcoin::ScriptBuf,
};

#[cfg(feature = "borsh")]
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "borsh")]
use std::io::{Read, Result, Write};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SpenderReference {
    pub txid: SerializedTxid,
    pub vin: u32,
}

#[cfg(feature = "borsh")]
impl BorshSerialize for SpenderReference {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<()> {
        BorshSerialize::serialize(&self.txid, writer)?;
        BorshSerialize::serialize(&self.vin, writer)?;
        Ok(())
    }
}

#[cfg(feature = "borsh")]
impl BorshDeserialize for SpenderReference {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self> {
        let txid = SerializedTxid::deserialize_reader(reader)?;
        let vin = u32::deserialize_reader(reader)?;
        Ok(Self { txid, vin })
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
pub enum SpentStatus {
    Unspent,
    Spent(SpenderReference),
    SpentUnknown,
}

// Intermediate structure for JSON serialization
#[cfg(feature = "serde")]
#[derive(Clone, Serialize, Deserialize)]
struct SpentStatusJson {
    spent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    vin: Option<SpenderReference>,
}

#[cfg(feature = "serde")]
impl Serialize for SpentStatus {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            SpentStatus::Unspent => SpentStatusJson {
                spent: false,
                vin: None,
            },
            SpentStatus::Spent(vin) => SpentStatusJson {
                spent: true,
                vin: Some(vin.clone()),
            },
            SpentStatus::SpentUnknown => SpentStatusJson {
                spent: true,
                vin: None,
            },
        }
        .serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for SpentStatus {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let json = SpentStatusJson::deserialize(deserializer)?;
        Ok(if json.spent {
            if let Some(vin) = json.vin {
                SpentStatus::Spent(vin)
            } else {
                SpentStatus::SpentUnknown
            }
        } else {
            SpentStatus::Unspent
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TxOut {
    pub runes: Vec<RuneAmount>,
    pub risky_runes: Vec<RuneAmount>,
    pub value: u64,
    pub spent: SpentStatus,
    pub script_pubkey: ScriptBuf,
}

impl TxOut {
    pub fn has_runes(&self) -> bool {
        !self.runes.is_empty()
    }
}

#[cfg(feature = "borsh")]
impl BorshSerialize for TxOut {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<()> {
        BorshSerialize::serialize(&self.value, writer)?;
        let script_bytes = self.script_pubkey.as_bytes();
        BorshSerialize::serialize(&(script_bytes.len() as u32), writer)?;
        writer.write_all(script_bytes)?;
        BorshSerialize::serialize(&self.runes, writer)?;
        BorshSerialize::serialize(&self.risky_runes, writer)?;
        BorshSerialize::serialize(&self.spent, writer)?;
        Ok(())
    }
}

#[cfg(feature = "borsh")]
impl BorshDeserialize for TxOut {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self> {
        let value = u64::deserialize_reader(reader)?;
        let script_len = u32::deserialize_reader(reader)? as usize;
        let mut script_bytes = vec![0u8; script_len];
        reader.read_exact(&mut script_bytes)?;
        let script_pubkey = ScriptBuf::from_bytes(script_bytes);
        let runes = Vec::<RuneAmount>::deserialize_reader(reader)?;
        let risky_runes = Vec::<RuneAmount>::deserialize_reader(reader)?;
        let spent = SpentStatus::deserialize_reader(reader)?;

        Ok(Self {
            value,
            script_pubkey,
            runes,
            risky_runes,
            spent,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{rune_amount::RuneAmount, rune_id::RuneId};
    use borsh::{BorshDeserialize, BorshSerialize};

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

    fn create_test_spender_reference() -> SpenderReference {
        SpenderReference {
            txid: SerializedTxid::from([42u8; 32]),
            vin: 12345,
        }
    }

    fn create_test_rune_amount() -> RuneAmount {
        RuneAmount {
            id: RuneId::new(840000, 1),
            amount: 1000000000000000000u128,
        }
    }

    fn create_test_tx_out_entry() -> TxOut {
        let rune_amount = create_test_rune_amount();
        TxOut {
            runes: vec![rune_amount.clone()],
            risky_runes: vec![rune_amount],
            value: 50000,
            spent: SpentStatus::Unspent,
            script_pubkey: ScriptBuf::new(),
        }
    }

    #[test]
    fn test_spender_reference_borsh_roundtrip() {
        let original = create_test_spender_reference();
        test_borsh_roundtrip(&original);
    }

    #[test]
    fn test_spender_reference_serde_roundtrip() {
        let original = create_test_spender_reference();
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_spender_reference_zero_vin() {
        let spender = SpenderReference {
            txid: SerializedTxid::from([0u8; 32]),
            vin: 0,
        };
        test_borsh_roundtrip(&spender);
        test_serde_roundtrip(&spender);
    }

    #[test]
    fn test_spender_reference_max_vin() {
        let spender = SpenderReference {
            txid: SerializedTxid::from([255u8; 32]),
            vin: u32::MAX,
        };
        test_borsh_roundtrip(&spender);
        test_serde_roundtrip(&spender);
    }

    #[test]
    fn test_spent_status_unspent() {
        let unspent = SpentStatus::Unspent;
        test_borsh_roundtrip(&unspent);
        test_serde_roundtrip(&unspent);
    }

    #[test]
    fn test_spent_status_spent() {
        let spent = SpentStatus::Spent(create_test_spender_reference());
        test_borsh_roundtrip(&spent);
        test_serde_roundtrip(&spent);
    }

    #[test]
    fn test_spent_status_different_variants() {
        let unspent = SpentStatus::Unspent;
        let spent = SpentStatus::Spent(create_test_spender_reference());

        assert_ne!(unspent, spent);

        // Test serialization produces different results
        let unspent_serialized = borsh::to_vec(&unspent).unwrap();
        let spent_serialized = borsh::to_vec(&spent).unwrap();
        assert_ne!(unspent_serialized, spent_serialized);
    }

    #[test]
    fn test_spent_status_serde_json_format() {
        // Test the specific JSON format for SpentStatus
        let unspent = SpentStatus::Unspent;
        let json = serde_json::to_string(&unspent).unwrap();
        assert!(json.contains("\"spent\":false"));
        assert!(!json.contains("vin"));

        let spent = SpentStatus::Spent(create_test_spender_reference());
        let json = serde_json::to_string(&spent).unwrap();
        assert!(json.contains("\"spent\":true"));
        assert!(json.contains("vin"));
    }

    #[test]
    fn test_tx_out_entry_borsh_roundtrip() {
        let original = create_test_tx_out_entry();
        test_borsh_roundtrip(&original);
    }

    #[test]
    fn test_tx_out_entry_serde_roundtrip() {
        let original = create_test_tx_out_entry();
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_tx_out_entry_empty_runes() {
        let entry = TxOut {
            runes: vec![],
            risky_runes: vec![],
            value: 1000,
            spent: SpentStatus::Unspent,
            script_pubkey: ScriptBuf::new(),
        };
        test_borsh_roundtrip(&entry);
        test_serde_roundtrip(&entry);
        assert!(!entry.has_runes());
    }

    #[test]
    fn test_tx_out_entry_with_runes() {
        let entry = create_test_tx_out_entry();
        assert!(entry.has_runes());
    }

    #[test]
    fn test_tx_out_entry_multiple_runes() {
        let rune1 = RuneAmount {
            id: RuneId::new(840000, 1),
            amount: 1000000000000000000u128,
        };
        let rune2 = RuneAmount {
            id: RuneId::new(840001, 2),
            amount: 2000000000000000000u128,
        };
        let rune3 = RuneAmount {
            id: RuneId::new(840002, 3),
            amount: 3000000000000000000u128,
        };

        let entry = TxOut {
            runes: vec![rune1.clone(), rune2.clone()],
            risky_runes: vec![rune3],
            value: 100000,
            spent: SpentStatus::Spent(create_test_spender_reference()),
            script_pubkey: ScriptBuf::new(),
        };

        test_borsh_roundtrip(&entry);
        test_serde_roundtrip(&entry);
        assert!(entry.has_runes());
    }

    #[test]
    fn test_tx_out_entry_zero_value() {
        let entry = TxOut {
            runes: vec![],
            risky_runes: vec![],
            value: 0,
            spent: SpentStatus::Unspent,
            script_pubkey: ScriptBuf::new(),
        };
        test_borsh_roundtrip(&entry);
        test_serde_roundtrip(&entry);
    }

    #[test]
    fn test_tx_out_entry_max_value() {
        let entry = TxOut {
            runes: vec![],
            risky_runes: vec![],
            value: u64::MAX,
            spent: SpentStatus::Unspent,
            script_pubkey: ScriptBuf::new(),
        };
        test_borsh_roundtrip(&entry);
        test_serde_roundtrip(&entry);
    }

    #[test]
    fn test_tx_out_entry_large_rune_collections() {
        // Test with larger collections
        let runes: Vec<RuneAmount> = (0..10)
            .map(|i| RuneAmount {
                id: RuneId::new(840000 + i, i as u32),
                amount: (i as u128 + 1) * 1000000000000000000u128,
            })
            .collect();

        let risky_runes: Vec<RuneAmount> = (10..15)
            .map(|i| RuneAmount {
                id: RuneId::new(840000 + i, i as u32),
                amount: (i as u128 + 1) * 500000000000000000u128,
            })
            .collect();

        let entry = TxOut {
            runes,
            risky_runes,
            value: 75000,
            spent: SpentStatus::Unspent,
            script_pubkey: ScriptBuf::new(),
        };

        test_borsh_roundtrip(&entry);
        test_serde_roundtrip(&entry);
        assert!(entry.has_runes());
    }

    #[test]
    fn test_tx_out_entry_consistency() {
        let original = create_test_tx_out_entry();

        // Test that multiple serializations produce the same result
        let serialized1 = borsh::to_vec(&original).unwrap();
        let serialized2 = borsh::to_vec(&original).unwrap();
        assert_eq!(serialized1, serialized2);

        // Test that deserialization produces the same value
        let deserialized1 = borsh::from_slice::<TxOut>(&serialized1).unwrap();
        let deserialized2 = borsh::from_slice::<TxOut>(&serialized2).unwrap();
        assert_eq!(deserialized1, deserialized2);
        assert_eq!(original, deserialized1);
    }

    #[test]
    fn test_edge_cases_with_extreme_rune_values() {
        let extreme_rune = RuneAmount {
            id: RuneId::new(u64::MAX, u32::MAX),
            amount: u128::MAX,
        };

        let entry = TxOut {
            runes: vec![extreme_rune.clone()],
            risky_runes: vec![extreme_rune],
            value: u64::MAX,
            spent: SpentStatus::Spent(SpenderReference {
                txid: SerializedTxid::from([255u8; 32]),
                vin: u32::MAX,
            }),
            script_pubkey: ScriptBuf::new(),
        };

        test_borsh_roundtrip(&entry);
        test_serde_roundtrip(&entry);
    }

    #[test]
    fn test_mixed_spent_statuses() {
        let entries = vec![
            TxOut {
                runes: vec![],
                risky_runes: vec![],
                value: 1000,
                spent: SpentStatus::Unspent,
                script_pubkey: ScriptBuf::new(),
            },
            TxOut {
                runes: vec![create_test_rune_amount()],
                risky_runes: vec![],
                value: 2000,
                spent: SpentStatus::Spent(create_test_spender_reference()),
                script_pubkey: ScriptBuf::new(),
            },
        ];

        for entry in entries {
            test_borsh_roundtrip(&entry);
            test_serde_roundtrip(&entry);
        }
    }
}
