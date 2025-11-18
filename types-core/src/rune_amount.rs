use crate::rune_id::RuneId;

#[cfg(feature = "borsh")]
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "borsh")]
use std::io::{Read, Result, Write};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RuneAmount {
    pub rune_id: RuneId,
    #[cfg_attr(feature = "serde", serde(with = "crate::serde_str"))]
    pub amount: u128,
}

impl From<(RuneId, u128)> for RuneAmount {
    fn from((rune_id, amount): (RuneId, u128)) -> Self {
        Self { rune_id, amount }
    }
}

impl PartialOrd for RuneAmount {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        let same_id = self.rune_id == other.rune_id;
        let amt_ord = self.amount.cmp(&other.amount);
        match (same_id, amt_ord) {
            (false, _) => None,
            (true, ord) => Some(ord),
        }
    }
}

impl PartialEq<RuneId> for RuneAmount {
    fn eq(&self, other: &RuneId) -> bool {
        self.rune_id == *other
    }
}

#[cfg(feature = "borsh")]
impl BorshSerialize for RuneAmount {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<()> {
        // Write out RuneId (block, tx):
        BorshSerialize::serialize(&self.rune_id.block, writer)?;
        BorshSerialize::serialize(&self.rune_id.tx, writer)?;

        // Write out amount
        BorshSerialize::serialize(&self.amount, writer)?;

        Ok(())
    }
}

#[cfg(feature = "borsh")]
impl BorshDeserialize for RuneAmount {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self> {
        // Read back RuneId fields:
        let block = u64::deserialize_reader(reader)?;
        let tx = u32::deserialize_reader(reader)?;

        // Read back amount
        let amount = u128::deserialize_reader(reader)?;

        Ok(RuneAmount {
            rune_id: RuneId::new(block, tx),
            amount,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rune_id::RuneId;
    #[cfg(feature = "borsh")]
    use borsh::{BorshDeserialize, BorshSerialize};

    /// Helper function to test borsh serialization roundtrip
    #[cfg(feature = "borsh")]
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
    #[cfg(feature = "serde")]
    fn test_serde_roundtrip<T>(original: &T) -> T
    where
        T: serde::Serialize + for<'de> serde::Deserialize<'de> + std::fmt::Debug + PartialEq,
    {
        let serialized = serde_json::to_string(original).expect("Failed to serialize");
        let deserialized = serde_json::from_str(&serialized).expect("Failed to deserialize");
        assert_eq!(original, &deserialized, "Serde roundtrip failed");
        deserialized
    }

    fn create_test_rune_amount() -> RuneAmount {
        RuneAmount {
            rune_id: RuneId::new(840000, 1),
            amount: 1000000000000000000u128,
        }
    }

    #[cfg(feature = "borsh")]
    #[test]
    fn test_rune_amount_borsh_roundtrip() {
        let original = create_test_rune_amount();
        test_borsh_roundtrip(&original);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_rune_amount_serde_roundtrip() {
        let original = create_test_rune_amount();
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_rune_amount_zero_values() {
        let rune_amount = RuneAmount {
            rune_id: RuneId::new(0, 0),
            amount: 0,
        };
        #[cfg(feature = "borsh")]
        test_borsh_roundtrip(&rune_amount);
        #[cfg(feature = "serde")]
        test_serde_roundtrip(&rune_amount);
    }

    #[test]
    fn test_rune_amount_max_values() {
        let rune_amount = RuneAmount {
            rune_id: RuneId::new(u64::MAX, u32::MAX),
            amount: u128::MAX,
        };
        #[cfg(feature = "borsh")]
        test_borsh_roundtrip(&rune_amount);
        #[cfg(feature = "serde")]
        test_serde_roundtrip(&rune_amount);
    }

    #[test]
    fn test_rune_amount_from_tuple() {
        let rune_id = RuneId::new(840000, 1);
        let amount = 1000000000000000000u128;
        let tuple = (rune_id, amount);

        let rune_amount: RuneAmount = tuple.into();
        assert_eq!(rune_amount.rune_id, rune_id);
        assert_eq!(rune_amount.amount, amount);
    }

    #[cfg(feature = "borsh")]
    #[test]
    fn test_rune_amount_consistency() {
        let original = create_test_rune_amount();

        // Test that multiple serializations produce the same result
        let serialized1 = borsh::to_vec(&original).unwrap();
        let serialized2 = borsh::to_vec(&original).unwrap();
        assert_eq!(serialized1, serialized2);

        // Test that deserialization produces the same value
        let deserialized1 = borsh::from_slice::<RuneAmount>(&serialized1).unwrap();
        let deserialized2 = borsh::from_slice::<RuneAmount>(&serialized2).unwrap();
        assert_eq!(deserialized1, deserialized2);
        assert_eq!(original, deserialized1);
    }

    #[cfg(feature = "borsh")]
    #[test]
    fn test_rune_amounts_different_rune_ids() {
        let rune1 = RuneAmount {
            rune_id: RuneId::new(840000, 1),
            amount: 1000000000000000000u128,
        };
        let rune2 = RuneAmount {
            rune_id: RuneId::new(840001, 2),
            amount: 1000000000000000000u128,
        };

        assert_ne!(rune1, rune2);

        // Test serialization produces different results
        let serialized1 = borsh::to_vec(&rune1).unwrap();
        let serialized2 = borsh::to_vec(&rune2).unwrap();
        assert_ne!(serialized1, serialized2);
    }

    #[cfg(feature = "borsh")]
    #[test]
    fn test_rune_amounts_different_amounts() {
        let rune1 = RuneAmount {
            rune_id: RuneId::new(840000, 1),
            amount: 1000000000000000000u128,
        };
        let rune2 = RuneAmount {
            rune_id: RuneId::new(840000, 1),
            amount: 2000000000000000000u128,
        };

        assert_ne!(rune1, rune2);

        // Test serialization produces different results
        let serialized1 = borsh::to_vec(&rune1).unwrap();
        let serialized2 = borsh::to_vec(&rune2).unwrap();
        assert_ne!(serialized1, serialized2);
    }

    #[test]
    fn test_collection_of_rune_amounts() {
        let rune_amounts = vec![
            RuneAmount {
                rune_id: RuneId::new(840000, 1),
                amount: 1000000000000000000u128,
            },
            RuneAmount {
                rune_id: RuneId::new(840001, 2),
                amount: 2000000000000000000u128,
            },
            RuneAmount {
                rune_id: RuneId::new(840002, 3),
                amount: 3000000000000000000u128,
            },
        ];

        for rune_amount in &rune_amounts {
            #[cfg(feature = "borsh")]
            test_borsh_roundtrip(rune_amount);
            #[cfg(feature = "serde")]
            test_serde_roundtrip(rune_amount);
        }

        // Test the collection as a whole
        #[cfg(feature = "borsh")]
        {
            let serialized = borsh::to_vec(&rune_amounts).unwrap();
            let deserialized: Vec<RuneAmount> = borsh::from_slice(&serialized).unwrap();
            assert_eq!(rune_amounts, deserialized);
        }
    }
}
