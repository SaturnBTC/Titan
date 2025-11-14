use {
    crate::{inscription_id::InscriptionId, rune_id::RuneId, SerializedTxid},
    borsh::{BorshDeserialize, BorshSerialize},
    ordinals::SpacedRune,
    serde::{Deserialize, Serialize},
    std::io::{Read, Result, Write},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MintResponse {
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub mintable: bool,
    #[serde(with = "crate::serde_str")]
    pub cap: u128,
    #[serde(with = "crate::serde_str")]
    pub amount: u128,
    #[serde(with = "crate::serde_str")]
    pub mints: u128,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuneResponse {
    pub id: RuneId,
    pub block: u64,
    #[serde(with = "crate::serde_str")]
    pub burned: u128,
    pub divisibility: u8,
    pub etching: SerializedTxid,
    pub number: u64,
    #[serde(with = "crate::serde_str")]
    pub premine: u128,
    #[serde(with = "crate::serde_str")]
    pub supply: u128,
    #[serde(with = "crate::serde_str")]
    pub max_supply: u128,
    pub spaced_rune: SpacedRune,
    pub symbol: Option<char>,
    pub mint: Option<MintResponse>,
    #[serde(with = "crate::serde_str")]
    pub burns: u128,
    #[serde(with = "crate::serde_str")]
    pub pending_burns: u128,
    #[serde(with = "crate::serde_str")]
    pub pending_mints: u128,
    pub inscription_id: Option<InscriptionId>,
    pub timestamp: u64,
    pub turbo: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuneAmount {
    pub rune_id: RuneId,
    #[serde(with = "crate::serde_str")]
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
    use borsh::{BorshDeserialize, BorshSerialize};
    use ordinals::{Rune, SpacedRune};

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

    fn create_test_rune_amount() -> RuneAmount {
        RuneAmount {
            rune_id: RuneId::new(840000, 1),
            amount: 1000000000000000000u128,
        }
    }

    fn create_test_mint_response() -> MintResponse {
        MintResponse {
            start: Some(840000),
            end: Some(850000),
            mintable: true,
            cap: 21000000u128,
            amount: 100000000u128,
            mints: 50000u128,
        }
    }

    fn create_test_rune_response() -> RuneResponse {
        RuneResponse {
            id: RuneId::new(840000, 1),
            block: 840000,
            burned: 1000000u128,
            divisibility: 8,
            etching: SerializedTxid::from([42u8; 32]),
            number: 1,
            premine: 21000000000000000000u128,
            supply: 21000000000000000000u128,
            max_supply: 21000000000000000000u128,
            spaced_rune: SpacedRune {
                rune: Rune(42),
                spacers: 0,
            },
            symbol: Some('₿'),
            mint: Some(create_test_mint_response()),
            burns: 500000u128,
            pending_burns: 250000u128,
            pending_mints: 10000u128,
            inscription_id: None,
            timestamp: 1640995200,
            turbo: false,
        }
    }

    #[test]
    fn test_rune_amount_borsh_roundtrip() {
        let original = create_test_rune_amount();
        test_borsh_roundtrip(&original);
    }

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
        test_borsh_roundtrip(&rune_amount);
        test_serde_roundtrip(&rune_amount);
    }

    #[test]
    fn test_rune_amount_max_values() {
        let rune_amount = RuneAmount {
            rune_id: RuneId::new(u64::MAX, u32::MAX),
            amount: u128::MAX,
        };
        test_borsh_roundtrip(&rune_amount);
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

    #[test]
    fn test_mint_response_serde_roundtrip() {
        let original = create_test_mint_response();
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_mint_response_no_limits() {
        let mint = MintResponse {
            start: None,
            end: None,
            mintable: false,
            cap: 0,
            amount: 0,
            mints: 0,
        };
        test_serde_roundtrip(&mint);
    }

    #[test]
    fn test_mint_response_with_limits() {
        let mint = MintResponse {
            start: Some(840000),
            end: Some(850000),
            mintable: true,
            cap: u128::MAX,
            amount: u128::MAX,
            mints: u128::MAX,
        };
        test_serde_roundtrip(&mint);
    }

    #[test]
    fn test_rune_response_serde_roundtrip() {
        let original = create_test_rune_response();
        test_serde_roundtrip(&original);
    }

    #[test]
    fn test_rune_response_minimal() {
        let minimal = RuneResponse {
            id: RuneId::new(0, 0),
            block: 0,
            burned: 0,
            divisibility: 0,
            etching: SerializedTxid::from([0u8; 32]),
            number: 0,
            premine: 0,
            supply: 0,
            max_supply: 0,
            spaced_rune: SpacedRune {
                rune: Rune(0),
                spacers: 0,
            },
            symbol: None,
            mint: None,
            burns: 0,
            pending_burns: 0,
            pending_mints: 0,
            inscription_id: None,
            timestamp: 0,
            turbo: false,
        };
        test_serde_roundtrip(&minimal);
    }

    #[test]
    fn test_rune_response_with_inscription() {
        let mut response = create_test_rune_response();
        response.inscription_id = Some(InscriptionId {
            txid: SerializedTxid::from([99u8; 32]),
            index: 0,
        });
        test_serde_roundtrip(&response);
    }

    #[test]
    fn test_rune_response_turbo_enabled() {
        let mut response = create_test_rune_response();
        response.turbo = true;
        test_serde_roundtrip(&response);
    }

    #[test]
    fn test_rune_response_extreme_values() {
        let extreme = RuneResponse {
            id: RuneId::new(u64::MAX, u32::MAX),
            block: u64::MAX,
            burned: u128::MAX,
            divisibility: u8::MAX,
            etching: SerializedTxid::from([255u8; 32]),
            number: u64::MAX,
            premine: u128::MAX,
            supply: u128::MAX,
            max_supply: u128::MAX,
            spaced_rune: SpacedRune {
                rune: Rune(u128::MAX),
                spacers: 0x7FFFFFF,
            }, // Use a valid spacers value
            symbol: Some('🚀'),
            mint: Some(MintResponse {
                start: Some(u64::MAX),
                end: Some(u64::MAX),
                mintable: true,
                cap: u128::MAX,
                amount: u128::MAX,
                mints: u128::MAX,
            }),
            burns: u128::MAX,
            pending_burns: u128::MAX,
            pending_mints: u128::MAX,
            inscription_id: Some(InscriptionId {
                txid: SerializedTxid::from([255u8; 32]),
                index: u32::MAX,
            }),
            timestamp: u64::MAX,
            turbo: true,
        };
        test_serde_roundtrip(&extreme);
    }

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
    fn test_rune_response_optional_fields() {
        // Test various combinations of optional fields
        let test_cases = vec![
            (None, None),
            (Some('₿'), None),
            (None, Some(create_test_mint_response())),
            (Some('₿'), Some(create_test_mint_response())),
        ];

        for (symbol, mint) in test_cases {
            let mut response = create_test_rune_response();
            response.symbol = symbol;
            response.mint = mint;
            test_serde_roundtrip(&response);
        }
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
            test_borsh_roundtrip(rune_amount);
            test_serde_roundtrip(rune_amount);
        }

        // Test the collection as a whole
        let serialized = borsh::to_vec(&rune_amounts).unwrap();
        let deserialized: Vec<RuneAmount> = borsh::from_slice(&serialized).unwrap();
        assert_eq!(rune_amounts, deserialized);
    }
}
