use crate::SerializedTxid;

#[cfg(feature = "bitcoincore-rpc")]
use bitcoincore_rpc::json::GetMempoolEntryResult;

#[cfg(feature = "borsh")]
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "borsh")]
use std::io::{Read, Result, Write};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
pub struct MempoolEntryFee {
    pub base: u64,
    pub descendant: u64,
    pub ancestor: u64,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MempoolEntry {
    pub vsize: u64,
    pub weight: Option<u64>,
    pub descendant_count: u64,
    pub descendant_size: u64,
    pub ancestor_count: u64,
    pub ancestor_size: u64,
    pub fees: MempoolEntryFee,
    pub depends: Vec<SerializedTxid>,
    #[cfg_attr(feature = "serde", serde(rename = "spentby"))]
    pub spent_by: Vec<SerializedTxid>,
}

#[cfg(feature = "bitcoincore-rpc")]
impl From<&GetMempoolEntryResult> for MempoolEntry {
    fn from(value: &GetMempoolEntryResult) -> Self {
        Self {
            vsize: value.vsize,
            weight: value.weight,
            descendant_count: value.descendant_count,
            descendant_size: value.descendant_size,
            ancestor_count: value.ancestor_count,
            ancestor_size: value.ancestor_size,
            fees: MempoolEntryFee {
                base: value.fees.base.to_sat(),
                descendant: value.fees.descendant.to_sat(),
                ancestor: value.fees.ancestor.to_sat(),
            },
            depends: value.depends.iter().map(|txid| txid.into()).collect(),
            spent_by: value.spent_by.iter().map(|txid| txid.into()).collect(),
        }
    }
}

#[cfg(feature = "borsh")]
impl BorshSerialize for MempoolEntry {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<()> {
        BorshSerialize::serialize(&self.vsize, writer)?;

        if let Some(weight) = self.weight {
            BorshSerialize::serialize(&true, writer)?;
            BorshSerialize::serialize(&weight, writer)?;
        } else {
            BorshSerialize::serialize(&false, writer)?;
        }

        BorshSerialize::serialize(&self.descendant_count, writer)?;
        BorshSerialize::serialize(&self.descendant_size, writer)?;
        BorshSerialize::serialize(&self.ancestor_count, writer)?;
        BorshSerialize::serialize(&self.ancestor_size, writer)?;
        BorshSerialize::serialize(&self.fees, writer)?;

        BorshSerialize::serialize(&(self.depends.len() as u64), writer)?;
        for txid in &self.depends {
            BorshSerialize::serialize(&txid, writer)?;
        }

        BorshSerialize::serialize(&(self.spent_by.len() as u64), writer)?;
        for txid in &self.spent_by {
            BorshSerialize::serialize(&txid, writer)?;
        }
        Ok(())
    }
}

#[cfg(feature = "borsh")]
impl BorshDeserialize for MempoolEntry {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self> {
        let vsize = u64::deserialize_reader(reader)?;

        let weight_exists = bool::deserialize_reader(reader)?;
        let weight = if weight_exists {
            Some(u64::deserialize_reader(reader)?)
        } else {
            None
        };

        let descendant_count = u64::deserialize_reader(reader)?;
        let descendant_size = u64::deserialize_reader(reader)?;
        let ancestor_count = u64::deserialize_reader(reader)?;
        let ancestor_size = u64::deserialize_reader(reader)?;
        let fees = MempoolEntryFee::deserialize_reader(reader)?;

        let depends_len = u64::deserialize_reader(reader)?;
        let mut depends = Vec::new();
        for _ in 0..depends_len {
            let txid = SerializedTxid::deserialize_reader(reader)?;
            depends.push(txid);
        }

        let spent_by_len = u64::deserialize_reader(reader)?;
        let mut spent_by = Vec::new();
        for _ in 0..spent_by_len {
            let txid = SerializedTxid::deserialize_reader(reader)?;
            spent_by.push(txid);
        }

        Ok(Self {
            vsize,
            weight,
            descendant_count,
            descendant_size,
            ancestor_count,
            ancestor_size,
            fees,
            depends,
            spent_by,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use borsh::{BorshDeserialize, BorshSerialize};

    fn create_test_mempool_entry() -> MempoolEntry {
        MempoolEntry {
            vsize: 250,
            weight: Some(1000),
            descendant_count: 5,
            descendant_size: 1500,
            ancestor_count: 3,
            ancestor_size: 800,
            fees: MempoolEntryFee {
                base: 5000,
                descendant: 7500,
                ancestor: 4200,
            },
            depends: vec![
                SerializedTxid::from([1u8; 32]),
                SerializedTxid::from([2u8; 32]),
            ],
            spent_by: vec![SerializedTxid::from([3u8; 32])],
        }
    }

    fn create_minimal_mempool_entry() -> MempoolEntry {
        MempoolEntry {
            vsize: 100,
            weight: None,
            descendant_count: 0,
            descendant_size: 0,
            ancestor_count: 0,
            ancestor_size: 0,
            fees: MempoolEntryFee {
                base: 1000,
                descendant: 1000,
                ancestor: 1000,
            },
            depends: vec![],
            spent_by: vec![],
        }
    }

    fn serialize_entry(entry: &MempoolEntry) -> std::io::Result<Vec<u8>> {
        let mut buffer = Vec::new();
        BorshSerialize::serialize(entry, &mut buffer)?;
        Ok(buffer)
    }

    fn deserialize_entry(data: &[u8]) -> std::io::Result<MempoolEntry> {
        MempoolEntry::deserialize_reader(&mut data.as_ref())
    }

    fn serialize_fee(fee: &MempoolEntryFee) -> std::io::Result<Vec<u8>> {
        let mut buffer = Vec::new();
        BorshSerialize::serialize(fee, &mut buffer)?;
        Ok(buffer)
    }

    fn deserialize_fee(data: &[u8]) -> std::io::Result<MempoolEntryFee> {
        MempoolEntryFee::deserialize_reader(&mut data.as_ref())
    }

    #[test]
    fn test_mempool_entry_serialization_deserialization_round_trip() {
        let original = create_test_mempool_entry();

        // Serialize
        let serialized = serialize_entry(&original).expect("Serialization should succeed");

        // Deserialize
        let deserialized = deserialize_entry(&serialized).expect("Deserialization should succeed");

        // Assert equality
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_mempool_entry_minimal_serialization_deserialization() {
        let original = create_minimal_mempool_entry();

        // Serialize
        let serialized = serialize_entry(&original).expect("Serialization should succeed");

        // Deserialize
        let deserialized = deserialize_entry(&serialized).expect("Deserialization should succeed");

        // Assert equality
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_mempool_entry_with_none_weight() {
        let mut entry = create_test_mempool_entry();
        entry.weight = None;

        let serialized = serialize_entry(&entry).expect("Serialization should succeed");
        let deserialized = deserialize_entry(&serialized).expect("Deserialization should succeed");

        assert_eq!(entry, deserialized);
        assert_eq!(deserialized.weight, None);
    }

    #[test]
    fn test_mempool_entry_with_some_weight() {
        let mut entry = create_test_mempool_entry();
        entry.weight = Some(42000);

        let serialized = serialize_entry(&entry).expect("Serialization should succeed");
        let deserialized = deserialize_entry(&serialized).expect("Deserialization should succeed");

        assert_eq!(entry, deserialized);
        assert_eq!(deserialized.weight, Some(42000));
    }

    #[test]
    fn test_mempool_entry_empty_vectors() {
        let mut entry = create_test_mempool_entry();
        entry.depends = vec![];
        entry.spent_by = vec![];

        let serialized = serialize_entry(&entry).expect("Serialization should succeed");
        let deserialized = deserialize_entry(&serialized).expect("Deserialization should succeed");

        assert_eq!(entry, deserialized);
        assert!(deserialized.depends.is_empty());
        assert!(deserialized.spent_by.is_empty());
    }

    #[test]
    fn test_mempool_entry_large_vectors() {
        let mut entry = create_test_mempool_entry();

        // Create larger vectors
        entry.depends = (0..10)
            .map(|i| SerializedTxid::from([i as u8; 32]))
            .collect();
        entry.spent_by = (10..15)
            .map(|i| SerializedTxid::from([i as u8; 32]))
            .collect();

        let serialized = serialize_entry(&entry).expect("Serialization should succeed");
        let deserialized = deserialize_entry(&serialized).expect("Deserialization should succeed");

        assert_eq!(entry, deserialized);
        assert_eq!(deserialized.depends.len(), 10);
        assert_eq!(deserialized.spent_by.len(), 5);
    }

    #[test]
    fn test_mempool_entry_fee_serialization() {
        let fee = MempoolEntryFee {
            base: u64::MAX,
            descendant: 0,
            ancestor: 12345,
        };

        let serialized = serialize_fee(&fee).expect("Fee serialization should succeed");
        let deserialized =
            deserialize_fee(&serialized).expect("Fee deserialization should succeed");

        assert_eq!(fee, deserialized);
    }

    #[test]
    fn test_mempool_entry_extreme_values() {
        let entry = MempoolEntry {
            vsize: u64::MAX,
            weight: Some(u64::MAX),
            descendant_count: u64::MAX,
            descendant_size: u64::MAX,
            ancestor_count: u64::MAX,
            ancestor_size: u64::MAX,
            fees: MempoolEntryFee {
                base: u64::MAX,
                descendant: u64::MAX,
                ancestor: u64::MAX,
            },
            depends: vec![SerializedTxid::all_zeros()],
            spent_by: vec![SerializedTxid::from([255u8; 32])],
        };

        let serialized = serialize_entry(&entry).expect("Serialization should succeed");
        let deserialized = deserialize_entry(&serialized).expect("Deserialization should succeed");

        assert_eq!(entry, deserialized);
    }

    #[test]
    fn test_mempool_entry_zero_values() {
        let entry = MempoolEntry {
            vsize: 0,
            weight: Some(0),
            descendant_count: 0,
            descendant_size: 0,
            ancestor_count: 0,
            ancestor_size: 0,
            fees: MempoolEntryFee {
                base: 0,
                descendant: 0,
                ancestor: 0,
            },
            depends: vec![],
            spent_by: vec![],
        };

        let serialized = serialize_entry(&entry).expect("Serialization should succeed");
        let deserialized = deserialize_entry(&serialized).expect("Deserialization should succeed");

        assert_eq!(entry, deserialized);
    }

    #[test]
    fn test_serialized_data_consistency() {
        let entry1 = create_test_mempool_entry();
        let entry2 = create_test_mempool_entry();

        let serialized1 = serialize_entry(&entry1).expect("Serialization should succeed");
        let serialized2 = serialize_entry(&entry2).expect("Serialization should succeed");

        // Same data should produce same serialized bytes
        assert_eq!(serialized1, serialized2);
    }

    #[test]
    fn test_serialization_size_efficiency() {
        let minimal = create_minimal_mempool_entry();
        let full = create_test_mempool_entry();

        let minimal_size = serialize_entry(&minimal).unwrap().len();
        let full_size = serialize_entry(&full).unwrap().len();

        // Full entry should be larger than minimal (basic sanity check)
        assert!(full_size > minimal_size);

        // Print sizes for debugging (will only show when test runs with --nocapture)
        println!("Minimal entry serialized size: {} bytes", minimal_size);
        println!("Full entry serialized size: {} bytes", full_size);
    }

    #[test]
    fn test_invalid_deserialization_handling() {
        // Test with incomplete data
        let incomplete_data = vec![1, 2, 3]; // Too short to be valid
        let result = deserialize_entry(&incomplete_data);
        assert!(
            result.is_err(),
            "Should fail to deserialize incomplete data"
        );
    }
}
